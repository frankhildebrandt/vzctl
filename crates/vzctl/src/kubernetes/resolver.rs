//! Kubelet resolver selection and the one K3s-owned DNS workload migration.
use super::{node_ip, quote, ClusterConfig};
use crate::config::{Environment, NetworkBackend};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) const PATH: &str = "/etc/rancher/k3s/vzctl-resolv.conf";
const ANNOTATION: &str = "vzctl.dev/k3s-resolver";

pub(super) fn upstream(
    environment: &Environment,
    cluster: &ClusterConfig,
) -> Result<Option<String>, String> {
    if !environment.spec.dns.enabled {
        return Ok(None);
    }
    let network = environment
        .spec
        .networks
        .get(&cluster.network)
        .ok_or("K3s resolver network missing")?;
    let cidr = network
        .cidr
        .parse::<ipnet::Ipv4Net>()
        .map_err(|_| "K3s resolver CIDR invalid")?;
    let dns = cidr.network();
    if network.backend != NetworkBackend::Vmnet
        || dns.is_loopback()
        || dns.is_link_local()
        || dns.is_multicast()
        || dns.octets()[0] == 0
        || dns.octets()[0] >= 240
    {
        return Err("K3s resolver requires a unicast vmnet network".into());
    }
    for vm in std::iter::once(&cluster.server).chain(cluster.agents.iter()) {
        let ip = node_ip(environment, cluster, vm)?
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| "K3s resolver node address invalid")?;
        if !cidr.contains(&ip) || ip == cidr.network() || ip == cidr.broadcast() {
            return Err("K3s resolver node address outside selected network".into());
        }
    }
    Ok(Some(cidr.network().to_string()))
}

pub(super) fn header(project: &str, cluster: &str) -> String {
    format!(
        "# vzctl K3s resolver v1 {}",
        digest(&format!("{project}/{cluster}"))
    )
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn contents(project: &str, cluster: &str, upstream: &str) -> String {
    format!("{}\nnameserver {upstream}\n", header(project, cluster))
}

// Check again immediately before each write/removal, including on disable. A
// foreign file is never silently adopted; /etc/resolv.conf is never touched.
pub(super) fn file_guard(path: &str, project: &str, cluster: &str) -> String {
    format!("if test -L {p}; then echo 'K3s resolver ownership conflict' >&2; exit 1; fi; if test -e {p}; then test -f {p} && test \"$(head -n 1 {p})\" = {h} || {{ echo 'K3s resolver ownership conflict' >&2; exit 1; }}; fi; ", p=quote(path), h=quote(&header(project, cluster)))
}

pub(super) fn marker(project: &str, cluster: &str, upstream: Option<&str>) -> String {
    format!(
        "v1:{}:{}",
        digest(&format!("{project}/{cluster}")),
        digest(upstream.unwrap_or("disabled"))
    )
}

fn identity(addon: &Value, deployment: &Value) -> Result<(), String> {
    let metadata = &deployment["metadata"];
    let annotations = &metadata["annotations"];
    let spec = &deployment["spec"]["template"]["spec"];
    if addon["apiVersion"] != "k3s.cattle.io/v1"
        || addon["kind"] != "Addon"
        || addon["metadata"]["name"] != "coredns"
        || addon["metadata"]["namespace"] != "kube-system"
        || addon["spec"]["source"] != "/var/lib/rancher/k3s/server/manifests/coredns.yaml"
        || deployment["apiVersion"] != "apps/v1"
        || deployment["kind"] != "Deployment"
        || metadata["name"] != "coredns"
        || metadata["namespace"] != "kube-system"
        || annotations["objectset.rio.cattle.io/owner-gvk"] != "k3s.cattle.io/v1, Kind=Addon"
        || annotations["objectset.rio.cattle.io/owner-name"] != "coredns"
        || annotations["objectset.rio.cattle.io/owner-namespace"] != "kube-system"
        || metadata["labels"]["k8s-app"] != "kube-dns"
        || metadata["labels"]["kubernetes.io/name"] != "CoreDNS"
        || spec["dnsPolicy"] != "Default"
        || spec["serviceAccountName"] != "coredns"
        || spec["containers"]
            .as_array()
            .is_none_or(|v| v.len() != 1 || v[0]["name"] != "coredns")
        || metadata
            .get("ownerReferences")
            .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()))
    {
        return Err("K3s CoreDNS Addon ownership conflict".into());
    }
    for object in [addon, deployment] {
        for key in ["uid", "resourceVersion"] {
            if object["metadata"][key].as_str().is_none_or(str::is_empty) {
                return Err("K3s CoreDNS identity missing".into());
            }
        }
    }
    Ok(())
}
fn parse(raw: String) -> Result<Value, String> {
    serde_json::from_str(&raw).map_err(|_| "K3s CoreDNS inventory invalid".into())
}

/// Only the K3s Addon's Deployment is rolled. UID/RV tests fence the mutation;
/// fresh Addon reads fence its indirect objectset identity before and after it.
/// No retry of a mutation after a conflict or an uncertain response.
pub(super) fn rollout(
    project: &str,
    cluster: &str,
    upstream: Option<&str>,
    mut kubectl: impl FnMut(&[&str], &[u8], u64) -> Result<String, String>,
) -> Result<(), String> {
    let get_deployment = [
        "get",
        "deployment",
        "coredns",
        "-n",
        "kube-system",
        "-o",
        "json",
        "--ignore-not-found",
    ];
    let mut raw = kubectl(&get_deployment, &[], 30)?;
    if raw.trim().is_empty() {
        if upstream.is_none() {
            return Ok(());
        }
        kubectl(
            &[
                "wait",
                "--for=create",
                "deployment/coredns",
                "-n",
                "kube-system",
                "--timeout=60s",
            ],
            &[],
            70,
        )?;
        raw = kubectl(&get_deployment, &[], 30)?;
    }
    let deployment = parse(raw)?;
    let old = deployment["spec"]["template"]["metadata"]["annotations"].get(ANNOTATION);
    if upstream.is_none() && old.is_none() {
        return Ok(());
    }
    let wanted = marker(project, cluster, upstream);
    if let Some(old) = old {
        let old = old.as_str().ok_or("K3s CoreDNS resolver marker invalid")?;
        let prefix = format!("v1:{}:", digest(&format!("{project}/{cluster}")));
        if !old.starts_with(&prefix)
            || old.len() != prefix.len() + 64
            || !old[prefix.len()..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("K3s CoreDNS resolver marker ownership conflict".into());
        }
    }
    let get_addon = [
        "get",
        "addons.k3s.cattle.io",
        "coredns",
        "-n",
        "kube-system",
        "-o",
        "json",
    ];
    let addon = parse(kubectl(&get_addon, &[], 30)?)?;
    identity(&addon, &deployment)?;
    if old.and_then(Value::as_str) != Some(wanted.as_str()) {
        let fresh_addon = parse(kubectl(&get_addon, &[], 30)?)?;
        if fresh_addon["metadata"]["uid"] != addon["metadata"]["uid"]
            || fresh_addon["metadata"]["resourceVersion"] != addon["metadata"]["resourceVersion"]
        {
            return Err("K3s CoreDNS Addon changed before rollout".into());
        }
        identity(&fresh_addon, &deployment)?;
        let mut annotations = deployment["spec"]["template"]["metadata"]["annotations"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        annotations.insert(ANNOTATION.into(), json!(wanted));
        let patch = json!([
            {"op":"test","path":"/metadata/uid","value":deployment["metadata"]["uid"]},
            {"op":"test","path":"/metadata/resourceVersion","value":deployment["metadata"]["resourceVersion"]},
            {"op":"add","path":"/spec/template/metadata/annotations","value":annotations}
        ]);
        // Patch file on stdin avoids shell interpolation and argv length limits.
        kubectl(
            &[
                "patch",
                "deployment",
                "coredns",
                "-n",
                "kube-system",
                "--type=json",
                "--patch-file=/dev/stdin",
            ],
            &serde_json::to_vec(&patch).map_err(|e| e.to_string())?,
            30,
        )?;
    }
    kubectl(
        &[
            "rollout",
            "status",
            "deployment/coredns",
            "-n",
            "kube-system",
            "--timeout=120s",
        ],
        &[],
        130,
    )?;
    let after = parse(kubectl(&get_deployment, &[], 30)?)?;
    let after_addon = parse(kubectl(&get_addon, &[], 30)?)?;
    identity(&after_addon, &after)?;
    if after_addon["metadata"]["uid"] != addon["metadata"]["uid"]
        || after["metadata"]["uid"] != deployment["metadata"]["uid"]
        || after["spec"]["template"]["metadata"]["annotations"][ANNOTATION] != wanted
    {
        return Err("K3s CoreDNS identity changed during rollout".into());
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in crate::kubernetes) fn objects() -> (Value, Value) {
        (
            json!({"apiVersion":"k3s.cattle.io/v1","kind":"Addon","metadata":{"name":"coredns","namespace":"kube-system","uid":"addon-1","resourceVersion":"1"},"spec":{"source":"/var/lib/rancher/k3s/server/manifests/coredns.yaml"}}),
            json!({"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"coredns","namespace":"kube-system","uid":"deployment-1","resourceVersion":"1","labels":{"k8s-app":"kube-dns","kubernetes.io/name":"CoreDNS"},"annotations":{"objectset.rio.cattle.io/owner-gvk":"k3s.cattle.io/v1, Kind=Addon","objectset.rio.cattle.io/owner-name":"coredns","objectset.rio.cattle.io/owner-namespace":"kube-system"}},"spec":{"template":{"metadata":{"annotations":{"other":"preserved"}},"spec":{"dnsPolicy":"Default","serviceAccountName":"coredns","containers":[{"name":"coredns"}]}}}}),
        )
    }
    struct Api {
        addon: Value,
        deployment: Value,
        patches: usize,
        addon_reads: usize,
        race: &'static str,
    }
    impl Api {
        fn new() -> Self {
            let (addon, deployment) = objects();
            Self {
                addon,
                deployment,
                patches: 0,
                addon_reads: 0,
                race: "",
            }
        }
        fn run(&mut self, upstream: Option<&str>) -> Result<(), String> {
            rollout("project", "dev", upstream, |args, input, _| {
                assert!(!args.iter().any(|s| s.contains("force")));
                match (args[0], args[1]) {
                    ("get", "deployment") => Ok(self.deployment.to_string()),
                    ("get", "addons.k3s.cattle.io") => {
                        self.addon_reads += 1;
                        if self.addon_reads == 2 && self.race == "addon" {
                            self.addon["metadata"]["uid"] = json!("replacement");
                        }
                        Ok(self.addon.to_string())
                    }
                    ("patch", "deployment") => {
                        let patch: Value = serde_json::from_slice(input).unwrap();
                        if self.race == "uid" {
                            self.deployment["metadata"]["uid"] = json!("replacement");
                        }
                        if self.race == "rv" {
                            self.deployment["metadata"]["resourceVersion"] = json!("2");
                        }
                        for test in patch.as_array().unwrap().iter().take(2) {
                            assert_eq!(test["op"], "test");
                            if self.deployment.pointer(test["path"].as_str().unwrap())
                                != Some(&test["value"])
                            {
                                return Err("JSON patch precondition failed".into());
                            }
                        }
                        assert_eq!(patch.as_array().unwrap().len(), 3);
                        assert_eq!(patch[2]["path"], "/spec/template/metadata/annotations");
                        self.deployment["spec"]["template"]["metadata"]["annotations"] =
                            patch[2]["value"].clone();
                        assert_eq!(
                            self.deployment["spec"]["template"]["metadata"]["annotations"]["other"],
                            "preserved"
                        );
                        self.patches += 1;
                        Ok(self.deployment.to_string())
                    }
                    ("rollout", "status") => {
                        if self.race == "after" {
                            self.addon["metadata"]["uid"] = json!("replacement");
                        }
                        Ok(String::new())
                    }
                    _ => panic!("unexpected Kubernetes operation {args:?}"),
                }
            })
        }
    }
    #[test]
    fn initial_addon_install_waits_only_for_absent_deployment() {
        for enabled in [false, true] {
            let mut api = Api::new();
            let mut reads = 0;
            let mut waits = 0;
            let result = rollout(
                "project",
                "dev",
                enabled.then_some("10.90.0.0"),
                |args, _, _| match args[0] {
                    "get" if args[1] == "deployment" => {
                        reads += 1;
                        if reads == 1 {
                            Ok(String::new())
                        } else {
                            Err("synthetic stop after creation wait".into())
                        }
                    }
                    "wait" => {
                        assert_eq!(args[1], "--for=create");
                        waits += 1;
                        Ok(String::new())
                    }
                    _ => {
                        api.patches += 1;
                        panic!("unexpected operation")
                    }
                },
            );
            assert_eq!(waits, usize::from(enabled));
            assert_eq!(result.is_ok(), !enabled);
        }
        let mut calls = 0;
        assert!(rollout("project", "dev", Some("10.90.0.0"), |_, _, _| {
            calls += 1;
            Err("Forbidden".into())
        })
        .is_err());
        assert_eq!(calls, 1);
    }
    #[test]
    fn rollout_is_idempotent_and_dns_toggle_restores_default_without_touching_applications() {
        let mut api = Api::new();
        api.run(None).unwrap();
        assert_eq!(api.patches, 0);
        assert_eq!(api.addon_reads, 0);
        api.run(Some("10.90.0.0")).unwrap();
        assert_eq!(api.patches, 1);
        api.run(Some("10.90.0.0")).unwrap();
        assert_eq!(api.patches, 1);
        api.run(None).unwrap();
        assert_eq!(api.patches, 2);
        api.run(None).unwrap();
        assert_eq!(api.patches, 2);
        api.run(Some("10.90.0.0")).unwrap();
        assert_eq!(api.patches, 3);
        assert_eq!(api.deployment["metadata"]["uid"], "deployment-1");
    }
    #[test]
    fn foreign_addon_deployment_and_resolver_marker_are_never_patched() {
        for case in 0..7 {
            let mut api = Api::new();
            match case {
                0 => api.addon["spec"]["source"] = json!("/foreign.yaml"),
                1 => {
                    api.deployment["metadata"]["annotations"]
                        ["objectset.rio.cattle.io/owner-name"] = json!("foreign")
                }
                2 => api.deployment["metadata"]["ownerReferences"] = json!([{"uid":"foreign"}]),
                3 => {
                    api.deployment["spec"]["template"]["spec"]["dnsPolicy"] = json!("ClusterFirst")
                }
                4 => {
                    api.deployment["spec"]["template"]["metadata"]["annotations"][ANNOTATION] =
                        json!(marker("foreign", "dev", Some("10.90.0.0")))
                }
                5 => api.deployment["metadata"]["uid"] = Value::Null,
                _ => api.deployment["spec"]["template"]["spec"]["containers"] = json!([]),
            }
            assert!(api.run(Some("10.90.0.0")).is_err(), "case {case}");
            assert_eq!(api.patches, 0);
        }
    }
    #[test]
    fn addon_uid_and_deployment_uid_rv_races_fail_without_mutation_retry() {
        for race in ["addon", "uid", "rv", "after"] {
            let mut api = Api::new();
            api.race = race;
            assert!(api.run(Some("10.90.0.0")).is_err(), "{race}");
            assert_eq!(api.patches, usize::from(race == "after"));
        }
    }
    #[test]
    fn resolver_file_guard_rejects_foreign_files_and_symlinks() {
        let dir = std::env::temp_dir().join(format!("vzk-resolver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("resolv.conf");
        let script = file_guard(path.to_str().unwrap(), "project", "dev");
        let run = || {
            std::process::Command::new("/bin/sh")
                .args(["-eu", "-c", &script])
                .output()
                .unwrap()
                .status
                .success()
        };
        assert!(run());
        std::fs::write(&path, "nameserver 1.1.1.1\n").unwrap();
        assert!(!run());
        let own = contents("project", "dev", "10.90.0.0");
        std::fs::write(&path, &own).unwrap();
        assert!(run());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("target", &path).unwrap();
        assert!(!run());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
