#!/usr/bin/env python3
"""Generate private lab files; no RPC, network, installation or VM operations."""
import argparse
import copy
import json
import os
from pathlib import Path
import secrets
import shutil

HERE = Path(__file__).resolve().parent
PROJECT = "controller-lab"
IMAGE = "python@sha256:a1165e272e578941b84abc79e4ab38a0305cd12803a5c4247979ac7655f4d641"
INSTALL = ("python -m pip install --disable-pip-version-check --target /tmp/deps "
           "--only-binary=:all: --require-hashes -r /opt/lab/requirements.txt "
           ">/tmp/pip.log 2>&1; export PYTHONPATH=/tmp/deps; exec python /opt/lab/probe.py serve")


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")
    path.chmod(0o600)


def manifest(kind, name, **extra):
    return dict(apiVersion="v1", kind=kind, metadata=dict(name=name, namespace="app"), **extra)


def generate(root):
    root.mkdir(mode=0o700)  # Refuse reuse: never overwrite credentials or another stack.
    (root / "secrets").mkdir(mode=0o700)
    (root / "evidence").mkdir(mode=0o700)
    (root / "assets").mkdir(mode=0o755)
    admin = dict(postgres=secrets.token_hex(32), access=secrets.token_hex(10), secret=secrets.token_hex(32))
    write(root / "secrets/admin.json", admin)
    for name, value in admin.items():
        (root / "secrets" / name).write_text(value)
        (root / "secrets" / name).chmod(0o600)
    for name in ("probe.py", "provider-setup.sh", "requirements.txt"):
        shutil.copyfile(HERE / name, root / "assets" / name)
        (root / "assets" / name).chmod(0o644)
    namespace = dict(apiVersion="v1", kind="Namespace", metadata=dict(name="app"))
    write(root / "kubernetes/namespace.yaml", namespace)
    vms = {}
    for name, ip, memory, cpus in [("ops", 10, "4Gi", 4), ("server", 11, "3Gi", 2), ("worker", 12, "2Gi", 2)]:
        vms[name] = dict(from_="ubuntu", disk="30G", cpus=cpus, memory=memory,
                         networks=[dict(name="lan", ip=f"10.123.0.{ip}")])
        vms[name]["from"] = vms[name].pop("from_")
    vms["ops"]["roles"] = ["docker", "router"]
    vms["ops"]["networks"].append(dict(name="containers", ip="10.124.0.2"))
    config = dict(apiVersion="hypernetwork/v1", kind="Environment", metadata=dict(name=PROJECT), spec=dict(
        project=PROJECT, domain=PROJECT + ".vz.test",
        dns=dict(enabled=True, hostResolver=True, hostListen="127.0.0.1:15353",
                 forward=dict(enabled=True, upstream="system")),
        images={"ubuntu": {"from": "ubuntu-24.04", "role": "base", "tag": "v1"}},
        networks={"lan": dict(cidr="10.123.0.0/24", mode="shared", natEgress=True),
                  "containers": dict(cidr="10.124.0.0/24", mode="shared", backend="docker", natEgress=False)},
        routes=[dict(name="lab-lan-to-containers", from_="lan", to="containers", via="ops")],
        policies=[
            dict(name="lab-lan-to-containers", network="lan", via="ops", forward="deny-all",
                 allow=[dict(to="containers", proto="tcp", ports=[8080]),
                        dict(to="containers", proto="icmp")]),
            dict(name="lab-container-egress", network="containers", via="ops", forward="deny-all",
                 allow=[dict(to="internet", proto="tcp", ports=[53, 80, 443]),
                        dict(to="internet", proto="udp", ports=[53]),
                        dict(to="lan", proto="tcp", ports=[53, 443, 5432, 6443, 9000]),
                        dict(to="lan", proto="udp", ports=[53])])],
        vms=vms, certs=dict(enabled=True, onRotate="reinject"),
        ingress=dict(enabled=True, bind="127.0.0.1", hostAliases=False,
                     routes=[dict(host="auth.svc.controller-lab.vz.test", to="oidc:15556")]),
        oidc=dict(enabled=True, mode="oidc-simple", issuer="https://auth.svc.controller-lab.vz.test",
                  listen="127.0.0.1:15556", clients="auto", users=[dict(username="lab", email="lab@example.test")]),
        kubernetes={"clusters": {"dev": dict(version="v1.37.1+k3s1",
            sha256="a1561ca4aef8b99f5588a840d4468ac39c8c1cd92470bbcb9634823fa221741a",
            server="server", agents=["worker"], network="lan", podCIDR="10.244.0.0/16",
            serviceCIDR="10.245.0.0/16", defaultNamespace="app", manifests=["kubernetes/namespace.yaml"])}}))
    config["spec"]["routes"][0]["from"] = config["spec"]["routes"][0].pop("from_")
    write(root / "hypernetwork.config.yaml", config)
    write(root / "bootstrap.yaml", config)
    full = copy.deepcopy(config)
    spec = full["spec"]
    spec["serviceProviders"] = {
        "database": dict(kind="postgres", via="ops", endpoint="127.0.0.1:5432",
                         advertiseEndpoint="10.123.0.10:5432", adminUser="postgres", passwordFile="secrets/postgres"),
        "objects": dict(kind="s3", via="ops", endpoint="http://10.123.0.10:9000",
                        advertiseEndpoint="http://10.123.0.10:9000", implementation="minio",
                        accessKeyFile="secrets/access", secretKeyFile="secrets/secret")}
    def consumers(name):
        return [dict(vm="ops"), dict(container="ops/proof"),
                dict(kubernetes=dict(cluster="dev", namespace="app", secret=name))]
    spec["serviceClaims"] = {
        "db": dict(kind="postgres", provider="database", database="controller_lab", consumers=consumers("db")),
        "files": dict(kind="s3", provider="objects", bucket="controller-lab-files", consumers=consumers("files")),
        "login": dict(kind="oidc", clientID="controller-lab",
                      redirectURIs=["https://docker.svc.controller-lab.vz.test/callback"],
                      consumers=consumers("login")),
        "docker-web": dict(kind="ingress", host="docker.svc.controller-lab.vz.test",
                           backend=dict(vm="ops", port=18080)),
        "kube-web": dict(kind="ingress", host="kube.svc.controller-lab.vz.test",
                         backend=dict(kubernetes=dict(cluster="dev", namespace="app", service="proof", port=8080)))}
    spec["vms"]["ops"]["containers"] = {
        "proof": dict(image=IMAGE, ports=["18080:8080"], user="10001:10001", restart="unless-stopped",
                      volumes=["/opt/vzctl-lab:/opt/lab:ro"],
                      env=dict(BINDING_ROOT="/run/vzctl/bindings", LAB_CA="/opt/lab/ca.crt", PROBE_NAME="docker"),
                      command=["sh", "-ec", INSTALL])}
    mounts = [dict(name="code", mountPath="/opt/lab", readOnly=True)]
    volumes = [dict(name="code", configMap=dict(name="proof-code"))]
    for name in ("db", "files", "login"):
        mounts.append(dict(name=name, mountPath="/bindings/" + name, readOnly=True))
        volumes.append(dict(name=name, secret=dict(secretName=name)))
    deployment = dict(apiVersion="apps/v1", kind="Deployment", metadata=dict(name="proof", namespace="app"),
        spec=dict(replicas=1, selector=dict(matchLabels=dict(app="proof")),
                  template=dict(metadata=dict(labels=dict(app="proof")), spec=dict(
                      securityContext=dict(runAsUser=10001, runAsGroup=10001, runAsNonRoot=True),
                      containers=[dict(name="proof", image=IMAGE, command=["sh", "-ec", INSTALL],
                        ports=[dict(containerPort=8080)], volumeMounts=mounts,
                        env=[dict(name=k,value=v) for k,v in dict(BINDING_ROOT="/bindings", BINDING_MODE="kubernetes",
                            LAB_CA="/opt/lab/ca.crt", PROBE_NAME="kubernetes").items()],
                        resources=dict(requests=dict(cpu="100m",memory="128Mi"), limits=dict(memory="512Mi")),
                        readinessProbe=dict(tcpSocket=dict(port=8080),initialDelaySeconds=5,periodSeconds=5))],
                      volumes=volumes))))
    service = manifest("Service", "proof", spec=dict(selector=dict(app="proof"),
                       ports=[dict(port=8080, targetPort=8080)]))
    write(root / "kubernetes/workloads.yaml", dict(apiVersion="v1", kind="List", items=[deployment,service]))
    # Placeholder public CA only. Activation replaces it before the full apply.
    code = manifest("ConfigMap", "proof-code", data={"probe.py": (HERE / "probe.py").read_text(),
                    "requirements.txt": (HERE / "requirements.txt").read_text(), "ca.crt": ""})
    write(root / "kubernetes/code.yaml", code)
    spec["kubernetes"]["clusters"]["dev"]["manifests"] += ["kubernetes/code.yaml", "kubernetes/workloads.yaml"]
    write(root / "full.yaml", full)
    late = []
    for kind, name, claim in [
        ("PostgresClaim","late-db",dict(provider="database",database="controller_late",bindingSecret="late-db")),
        ("S3Claim","late-files",dict(provider="objects",bucket="controller-lab-late",bindingSecret="late-files")),
        ("OIDCClaim","late-login",dict(clientID="controller-lab-late",
          redirectURIs=["https://late.svc.controller-lab.vz.test/callback"],bindingSecret="late-login")),
        ("IngressClaim","late-web",dict(host="late.svc.controller-lab.vz.test",
          backend=dict(kubernetes=dict(service="proof",port=8080)),bindingSecret="late-web"))]:
        late.append(dict(apiVersion="services.vzctl.dev/v1alpha1",kind=kind,
                         metadata=dict(name=name,namespace="app"),spec=claim))
    write(root / "late-claims.yaml", dict(apiVersion="v1",kind="List",items=late))
    (root / ".gitignore").write_text("*\n")


def activate(root, ca):
    assert root.name == PROJECT and root.is_dir()
    data = json.loads((root / "full.yaml").read_text())
    assert data["spec"]["project"] == PROJECT
    pem = ca.read_text()
    assert "BEGIN CERTIFICATE" in pem and "PRIVATE KEY" not in pem
    (root / "assets/ca.crt").write_text(pem)
    (root / "assets/ca.crt").chmod(0o644)
    code = json.loads((root / "kubernetes/code.yaml").read_text())
    code["data"]["ca.crt"] = pem
    write(root / "kubernetes/code.yaml", code)
    # Sole desired-state switch, preserving admin credentials.
    write(root / ".hypernetwork.next", data)
    os.replace(root / ".hypernetwork.next", root / "hypernetwork.config.yaml")


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("directory", type=Path)
    p.add_argument("--activate", type=Path, metavar="PUBLIC_CA")
    args = p.parse_args()
    root = args.directory.expanduser().resolve()
    if root.name != PROJECT:
        p.error("directory basename must be controller-lab")
    if args.activate:
        activate(root, args.activate.expanduser().resolve())
    else:
        generate(root)
    print("Prepared private files in " + str(root) + "; no runtime operation performed.")
