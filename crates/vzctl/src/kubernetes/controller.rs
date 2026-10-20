use super::{managed_fields, manifests, node_ip, quote, resolver, ClusterConfig};
use crate::config::Environment;
use crate::progress::ProgressReporter;
use base64::Engine;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const UNIT: &str = "vzctl-k3s.service";
const OWNER: &str = "/var/lib/vzctl/kubernetes/owner.json";
const CONFIG: &str = "/etc/rancher/k3s/config.yaml";
const TOKEN: &str = "/etc/rancher/k3s/vzctl-token";

pub(super) struct Guest<'a> {
    pub(super) environment: &'a Environment,
    pub(super) socket: &'a Path,
}

impl Guest<'_> {
    pub(super) fn exec(
        &self,
        vm: &str,
        args: &[String],
        input: &[u8],
        seconds: u64,
    ) -> Result<String, String> {
        if input.len() > 256 * 1024 {
            return Err("guest input exceeds 256 KiB".into());
        }
        let result = crate::vm::supervisor_rpc_deadline(
            self.socket,
            "vm.exec",
            json!({
                "vm_id": format!("{}/{vm}", self.environment.spec.project),
                "cmd": args, "stdin_b64": base64::engine::general_purpose::STANDARD.encode(input),
                "timeout_ms": seconds * 1000,
            }),
            seconds + 10,
        )
        .map_err(|(_, error)| format!("K3s node {vm}: {error}"))?;
        if result["truncated"].as_bool().unwrap_or(false) {
            return Err(format!("K3s node {vm}: guest response truncated"));
        }
        if result["exit"].as_i64() != Some(0) {
            return Err(format!(
                "K3s node {vm}: {}",
                result["stderr"]
                    .as_str()
                    .unwrap_or("guest command failed")
                    .trim()
            ));
        }
        Ok(result["stdout"].as_str().unwrap_or("").to_owned())
    }

    fn shell(&self, vm: &str, script: &str, input: &[u8], seconds: u64) -> Result<String, String> {
        self.exec(
            vm,
            &[
                "sudo".into(),
                "-n".into(),
                "/bin/sh".into(),
                "-eu".into(),
                "-c".into(),
                script.into(),
            ],
            input,
            seconds,
        )
    }

    pub(super) fn kubectl(
        &self,
        cluster: &ClusterConfig,
        args: &[&str],
        input: &[u8],
        seconds: u64,
    ) -> Result<String, String> {
        let endpoint = format!(
            "https://{}:6443",
            node_ip(self.environment, cluster, &cluster.server)?
        );
        let mut cmd = vec![
            "sudo".into(),
            "-n".into(),
            "/usr/local/bin/k3s".into(),
            "kubectl".into(),
            "--kubeconfig=/etc/rancher/k3s/k3s.yaml".into(),
            format!("--server={endpoint}"),
            format!("--namespace={}", cluster.default_namespace),
            format!("--request-timeout={}s", seconds.min(30)),
        ];
        cmd.extend(args.iter().map(|s| s.to_string()));
        self.exec(&cluster.server, &cmd, input, seconds)
    }
}

pub(crate) fn ensure(
    environment: &Environment,
    config_path: &Path,
    socket: &Path,
    state: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), String> {
    let guest = Guest {
        environment,
        socket,
    };
    let config_file = crate::config::config_path(config_path);
    let base = config_file.parent().unwrap_or_else(|| Path::new("."));
    // Complete every preflight before installing anything on the first node.
    for (name, cluster) in &environment.spec.kubernetes.clusters {
        resolver::upstream(environment, cluster)?;
        for vm in std::iter::once(&cluster.server).chain(cluster.agents.iter()) {
            guest.shell(
                vm,
                &resolver::file_guard(resolver::PATH, &environment.spec.project, name),
                &[],
                15,
            )?;
            guest.shell(vm, "test \"$(cat /proc/1/comm)\" = systemd || { echo 'K3s requires systemd PID 1' >&2; exit 1; }; test \"$(uname -m)\" = aarch64 || { echo 'K3s requires ARM64' >&2; exit 1; }; command -v curl >/dev/null; command -v sha256sum >/dev/null; command -v ip >/dev/null; command -v systemctl >/dev/null", &[], 15)?;
            check_node_owner(&guest, name, cluster, vm)?;
        }
        for path in &cluster.manifests {
            for value in manifests::read(&base.join(path))? {
                manifests::owned(value, &environment.spec.project, name)?;
            }
        }
    }
    for (name, cluster) in &environment.spec.kubernetes.clusters {
        let directory = cluster_directory(state, &environment.spec.project, name);
        let token = ensure_token(&directory)?;
        progress.log(json!({"event":"kubernetes.cluster.reconciling","cluster":name,"version":cluster.version}).to_string());
        install_node(&guest, name, cluster, &cluster.server, &token)?;
        // Explicit API endpoint keeps guest-only binding compatible with local kubectl.
        wait_ready(&guest, cluster, &cluster.server)?;
        let secure_token = guest.shell(
            &cluster.server,
            "cat /var/lib/rancher/k3s/server/token",
            &[],
            15,
        )?;
        if !secure_token.trim().starts_with("K10") {
            return Err(format!(
                "K3s cluster {name}: server did not produce a CA-bound join token"
            ));
        }
        write_private(
            &directory.join("agent-token"),
            secure_token.trim().as_bytes(),
        )?;
        for vm in &cluster.agents {
            install_node(&guest, name, cluster, vm, secure_token.trim())?;
            wait_ready(&guest, cluster, vm)?;
        }
        resolver::rollout(
            &environment.spec.project,
            name,
            resolver::upstream(environment, cluster)?.as_deref(),
            |args, input, seconds| guest.kubectl(cluster, args, input, seconds),
        )?;
        for definition in crate::controllers::kubernetes_claims::definitions() {
            let definition = manifests::owned(definition, &environment.spec.project, name)?;
            apply_object(&guest, name, cluster, &definition)?;
        }
        let namespace = guest.kubectl(
            cluster,
            &[
                "get",
                "namespace",
                &cluster.default_namespace,
                "--ignore-not-found",
                "-o",
                "name",
            ],
            &[],
            30,
        )?;
        if namespace.trim().is_empty() {
            let namespace = manifests::owned(
                json!({
                    "apiVersion":"v1", "kind":"Namespace", "metadata":{"name":cluster.default_namespace}
                }),
                &environment.spec.project,
                name,
            )?;
            apply_object(&guest, name, cluster, &namespace)?;
        }
        let mut applied = 0;
        for path in &cluster.manifests {
            for value in manifests::read(&base.join(path))? {
                let value = manifests::owned(value, &environment.spec.project, name)?;
                apply_object(&guest, name, cluster, &value)?;
                applied += 1;
            }
        }
        write_private(
            &directory.join("last-apply.json"),
            &serde_json::to_vec(&json!({
                "version":cluster.version,"resources":applied,"status":"ready"
            }))
            .map_err(|e| e.to_string())?,
        )?;
        progress.log(
            json!({"event":"kubernetes.cluster.ready","cluster":name,"resources":applied})
                .to_string(),
        );
    }
    Ok(())
}

fn apply_object(
    guest: &Guest<'_>,
    name: &str,
    cluster: &ClusterConfig,
    value: &Value,
) -> Result<(), String> {
    apply_owned_object(
        &guest.environment.spec.project,
        name,
        cluster,
        value,
        |args, input, seconds| guest.kubectl(cluster, args, input, seconds),
    )
}

pub(super) fn apply_owned_object(
    project: &str,
    name: &str,
    cluster: &ClusterConfig,
    value: &Value,
    mut kubectl: impl FnMut(&[&str], &[u8], u64) -> Result<String, String>,
) -> Result<(), String> {
    let payload = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let namespace = value["metadata"]["namespace"]
        .as_str()
        .unwrap_or(&cluster.default_namespace);
    let existing = kubectl(
        &[
            "get",
            "--ignore-not-found",
            "--show-managed-fields=true",
            "-f",
            "-",
            "-o",
            "json",
            "--namespace",
            namespace,
        ],
        &payload,
        30,
    )?;
    let mut existing = match read_inventory(&existing, value, namespace)? {
        Some(existing) => existing,
        None => {
            // A create conflict must fail, never adopt an object created after our GET.
            let created = kubectl(
                &[
                    "create",
                    "--field-manager=vzctl-kubernetes",
                    "--show-managed-fields=true",
                    "-f",
                    "-",
                    "-o",
                    "json",
                    "--namespace",
                    namespace,
                ],
                &payload,
                30,
            )?;
            let created: Value = serde_json::from_str(&created)
                .map_err(|e| format!("invalid created resource: {e}"))?;
            validate_inventory_identity(&created, value, namespace)?;
            created
        }
    };
    manifests::check_existing(&existing, project, name)?;
    let uid = existing["metadata"]["uid"]
        .as_str()
        .filter(|uid| !uid.is_empty())
        .ok_or("Kubernetes inventory omitted UID; refusing an unguarded update")?
        .to_owned();
    for attempt in 0..3 {
        // Revalidate the owner and immutable object identity on every fresh read.
        manifests::check_existing(&existing, project, name)?;
        if existing["metadata"]["uid"].as_str() != Some(uid.as_str()) {
            return Err("ownership conflict: Kubernetes resource was replaced during apply".into());
        }
        existing = migrate_create_ownership(
            project,
            name,
            value,
            existing,
            namespace,
            &uid,
            &mut kubectl,
        )?;
        let version = existing["metadata"]["resourceVersion"]
            .as_str()
            .filter(|version| !version.is_empty())
            .ok_or("Kubernetes inventory omitted resourceVersion; refusing an unguarded update")?
            .to_owned();
        let mut desired = value.clone();
        desired["metadata"]["uid"] = json!(uid);
        desired["metadata"]["resourceVersion"] = json!(version);
        let update = serde_json::to_vec(&desired).map_err(|e| e.to_string())?;
        match kubectl(
            &[
                "apply",
                "--server-side",
                "--field-manager=vzctl-kubernetes",
                "-f",
                "-",
                "--namespace",
                namespace,
            ],
            &update,
            30,
        ) {
            Ok(_) => break,
            Err(error) if resource_version_conflict(&error) && attempt < 2 => {
                let latest = kubectl(
                    &[
                        "get",
                        "--ignore-not-found",
                        "--show-managed-fields=true",
                        "-f",
                        "-",
                        "-o",
                        "json",
                        "--namespace",
                        namespace,
                    ],
                    &payload,
                    30,
                )?;
                existing = read_inventory(&latest, value, namespace)?
                    .ok_or("ownership conflict: Kubernetes resource disappeared during apply")?;
                manifests::check_existing(&existing, project, name)?;
                if existing["metadata"]["uid"].as_str() != Some(uid.as_str()) {
                    return Err(
                        "ownership conflict: Kubernetes resource was replaced during apply".into(),
                    );
                }
                if existing["metadata"]["resourceVersion"].as_str() == Some(version.as_str()) {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    if value["kind"] == "CustomResourceDefinition" {
        let resource = format!(
            "customresourcedefinition/{}",
            value["metadata"]["name"]
                .as_str()
                .ok_or("CRD requires name")?
        );
        kubectl(
            &[
                "wait",
                "--for=condition=Established",
                &resource,
                "--timeout=60s",
            ],
            &[],
            70,
        )?;
    }
    Ok(())
}

fn migrate_create_ownership(
    project: &str,
    cluster: &str,
    desired: &Value,
    mut existing: Value,
    namespace: &str,
    uid: &str,
    kubectl: &mut impl FnMut(&[&str], &[u8], u64) -> Result<String, String>,
) -> Result<Value, String> {
    let Some(mut migration) = managed_fields::migration(&existing, desired)? else {
        return Ok(existing);
    };
    let payload = serde_json::to_vec(desired).map_err(|e| e.to_string())?;
    let read = |kubectl: &mut dyn FnMut(&[&str], &[u8], u64) -> Result<String, String>| -> Result<Value, String> {
        let raw = kubectl(
            &["get", "--ignore-not-found", "--show-managed-fields=true", "-f", "-", "-o", "json", "--namespace", namespace],
            &payload, 30,
        )?;
        let actual = read_inventory(&raw, desired, namespace)?
            .ok_or("ownership conflict: resource disappeared during field migration")?;
        manifests::check_existing(&actual, project, cluster)?;
        if actual["metadata"]["uid"].as_str() != Some(uid)
            || actual["metadata"]["resourceVersion"].as_str().is_none_or(str::is_empty)
        {
            return Err("ownership conflict: resource identity changed during field migration".into());
        }
        Ok(actual)
    };
    if let Some(bridge) = &migration.bridge {
        // Identical current values, limited to our own recorded Update+Apply
        // fields. This also handles a Create interrupted before its first SSA.
        // Foreign field conflicts remain errors; no force or reset is allowed.
        let bytes = serde_json::to_vec(bridge).map_err(|e| e.to_string())?;
        kubectl(
            &[
                "apply",
                "--server-side",
                "--field-manager=vzctl-kubernetes",
                "-f",
                "-",
                "--namespace",
                namespace,
            ],
            &bytes,
            30,
        )?;
        let actual = read(kubectl)?;
        if !managed_fields::same_values(&existing, &actual) {
            return Err("resource values changed during field ownership bridge".into());
        }
        existing = actual;
        migration = managed_fields::migration(&existing, desired)?
            .ok_or("field ownership bridge unexpectedly removed Create ownership")?;
        if migration.bridge.is_some() {
            return Err("field ownership bridge did not establish complete Apply ownership".into());
        }
    }
    if migration.retained.as_array().is_none_or(Vec::is_empty) {
        return Err("field ownership migration refuses an empty managedFields reset".into());
    }
    let patch = serde_json::to_string(&migration.patch).map_err(|e| e.to_string())?;
    // UID, resourceVersion and the complete field inventory are JSON Patch tests.
    // Any race fails this attempt. Remove only exact own Update entries.
    kubectl(
        &[
            "patch",
            "--type=json",
            "--field-manager=vzctl-kubernetes-migration",
            "-f",
            "-",
            "--patch",
            &patch,
            "--namespace",
            namespace,
        ],
        &payload,
        30,
    )?;
    let actual = read(kubectl)?;
    if actual["metadata"]["managedFields"] != migration.retained
        || !managed_fields::same_values(&existing, &actual)
    {
        return Err(
            "field ownership migration did not preserve resource and foreign managers".into(),
        );
    }
    Ok(actual)
}

// `kubectl get -f - -o json` can wrap a single object in a generic List.
// Only GET may represent absence; CREATE must return a concrete matching object.
pub(super) fn read_inventory(
    raw: &str,
    desired: &Value,
    namespace: &str,
) -> Result<Option<Value>, String> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let mut actual: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid resource inventory: {e}"))?;
    if actual["kind"] == "List" {
        if actual["apiVersion"] != "v1" {
            return Err("invalid Kubernetes inventory List version".into());
        }
        let items = actual["items"]
            .as_array_mut()
            .ok_or("invalid Kubernetes inventory List items")?;
        match items.len() {
            0 => return Ok(None),
            1 => actual = items.pop().unwrap(),
            _ => return Err("ambiguous Kubernetes inventory: expected one resource".into()),
        }
    }
    validate_inventory_identity(&actual, desired, namespace)?;
    Ok(Some(actual))
}

fn validate_inventory_identity(
    actual: &Value,
    desired: &Value,
    namespace: &str,
) -> Result<(), String> {
    if actual["apiVersion"] != desired["apiVersion"]
        || actual["kind"] != desired["kind"]
        || actual["metadata"]["name"] != desired["metadata"]["name"]
    {
        return Err(
            "Kubernetes inventory resource identity does not match the requested object".into(),
        );
    }
    match actual["metadata"].get("namespace") {
        Some(value) if value.as_str() != Some(namespace) => {
            return Err("Kubernetes inventory namespace does not match the requested object".into())
        }
        None if desired["metadata"].get("namespace").is_some() => {
            return Err("Kubernetes inventory omitted the requested namespace".into())
        }
        _ => {}
    }
    Ok(())
}

// kubectl exposes API Status errors on stderr. Only the canonical resourceVersion
// conflict is retryable; field-manager conflicts, timeouts and authorization
// errors must never be interpreted as permission to overwrite an object.
fn resource_version_conflict(error: &str) -> bool {
    error.contains("Operation cannot be fulfilled on ")
        && error.contains("the object has been modified; please apply your changes to the latest version and try again")
}

fn wait_ready(guest: &Guest<'_>, cluster: &ClusterConfig, vm: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        let result = guest
            .kubectl(
                cluster,
                &[
                    "wait",
                    "--for=condition=Ready",
                    &format!("node/{vm}"),
                    "--timeout=10s",
                ],
                &[],
                15,
            )
            .and_then(|_| guest.kubectl(cluster, &["get", "node", vm, "-o", "json"], &[], 15))
            .and_then(|text| {
                let node: Value =
                    serde_json::from_str(&text).map_err(|_| "invalid node readiness inventory")?;
                if node_ready(&node, &cluster.version) {
                    Ok(())
                } else {
                    Err(format!(
                        "node is not Ready at pinned version {}",
                        cluster.version
                    ))
                }
            });
        match result {
            Ok(()) => return Ok(()),
            Err(error) if std::time::Instant::now() >= deadline => {
                return Err(format!(
                    "K3s node {vm}: readiness deadline exceeded: {error}"
                ))
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(2)),
        }
    }
}

pub(super) fn node_ready(node: &Value, version: &str) -> bool {
    node["status"]["nodeInfo"]["kubeletVersion"] == version
        && node["status"]["conditions"]
            .as_array()
            .is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|condition| condition["type"] == "Ready" && condition["status"] == "True")
            })
}

fn owner(environment: &Environment, name: &str, cluster: &ClusterConfig, vm: &str) -> Value {
    json!({
        "managed_by":"vzctl","project":environment.spec.project,"cluster":name,
        "server":cluster.server,"node":vm,"network":cluster.network,
        "pod_cidr":cluster.pod_cidr,"service_cidr":cluster.service_cidr
    })
}

fn check_node_owner(
    guest: &Guest<'_>,
    name: &str,
    cluster: &ClusterConfig,
    vm: &str,
) -> Result<(), String> {
    let actual = guest.shell(vm, &format!(
        "if test -f {OWNER}; then cat {OWNER}; elif test -e {CONFIG} || test -e /usr/local/bin/k3s || test -e /etc/systemd/system/{UNIT} || test -e /etc/systemd/system/k3s.service || test -e /etc/systemd/system/k3s-agent.service || test -e /var/lib/rancher/k3s; then echo 'K3s ownership conflict: existing unmanaged installation' >&2; exit 1; fi"
    ), &[], 15)?;
    if !actual.trim().is_empty() {
        let existing: Value =
            serde_json::from_str(&actual).map_err(|_| "invalid K3s ownership marker")?;
        if existing != owner(guest.environment, name, cluster, vm) {
            return Err(format!("K3s node {vm}: ownership or immutable cluster topology changed; explicit migration required"));
        }
    }
    Ok(())
}

fn install_node(
    guest: &Guest<'_>,
    name: &str,
    cluster: &ClusterConfig,
    vm: &str,
    token: &str,
) -> Result<(), String> {
    let upstream = resolver::upstream(guest.environment, cluster)?;
    let guard = resolver::file_guard(resolver::PATH, &guest.environment.spec.project, name);
    guest.shell(vm, &guard, &[], 15)?;
    let ip = node_ip(guest.environment, cluster, vm)?;
    let interface = guest.shell(vm, &interface_script(ip), &[], 15)?;
    let interface = interface.trim();
    if interface.is_empty()
        || !interface
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(format!(
            "K3s node {vm}: configured IP {ip} has no usable guest interface"
        ));
    }
    let marker = serde_json::to_vec(&owner(guest.environment, name, cluster, vm))
        .map_err(|e| e.to_string())?;
    guest_file(guest, vm, OWNER, &marker)?;
    let mut changed = guest.shell(vm, &install_script(cluster), &[], 360)?.trim() == "changed";
    changed |= guest_file(guest, vm, TOKEN, token.as_bytes())?;
    if let Some(upstream) = upstream.as_deref() {
        changed |= guarded_guest_file(
            guest,
            vm,
            resolver::PATH,
            &guard,
            resolver::contents(&guest.environment.spec.project, name, upstream).as_bytes(),
        )?;
    }
    let config = node_config(guest.environment, cluster, vm, interface)?;
    changed |= guest_file(
        guest,
        vm,
        CONFIG,
        serde_yaml::to_string(&config)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;
    changed |= guest_file(
        guest,
        vm,
        &format!("/etc/systemd/system/{UNIT}"),
        unit(vm == cluster.server).as_bytes(),
    )?;
    guest.shell(vm, if changed {
        "systemctl daemon-reload; systemctl enable vzctl-k3s.service >/dev/null; systemctl restart vzctl-k3s.service"
    } else {
        "systemctl enable --now vzctl-k3s.service >/dev/null"
    }, &[], 200)?;
    if upstream.is_none() {
        guest.shell(
            vm,
            &format!("{guard}rm -f -- {}", quote(resolver::PATH)),
            &[],
            15,
        )?;
    }
    Ok(())
}

pub(super) fn interface_script(ip: &str) -> String {
    format!(
        "ip -o -4 address show | awk -v wanted={} {}",
        quote(ip),
        quote(r#"{ split($4,a,"/"); if(a[1]==wanted){print $2;exit} }"#)
    )
}

pub(super) fn node_config(
    environment: &Environment,
    cluster: &ClusterConfig,
    vm: &str,
    interface: &str,
) -> Result<Value, String> {
    let ip = node_ip(environment, cluster, vm)?;
    let server_ip = node_ip(environment, cluster, &cluster.server)?;
    let mut config = json!({
        "token-file":TOKEN,"node-name":vm,"node-ip":ip,"flannel-iface":interface,
        "node-label":[format!("vzctl.dev/project={}",environment.spec.project)]
    });
    if resolver::upstream(environment, cluster)?.is_some() {
        config["resolv-conf"] = json!(resolver::PATH);
    }
    if vm == cluster.server {
        config["bind-address"] = json!(server_ip);
        config["advertise-address"] = json!(server_ip);
        config["tls-san"] = json!([server_ip]);
        config["cluster-cidr"] = json!(cluster.pod_cidr);
        config["service-cidr"] = json!(cluster.service_cidr);
        let services = cluster
            .service_cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|e| e.to_string())?;
        config["cluster-dns"] =
            json!(std::net::Ipv4Addr::from(u32::from(services.network()) + 10).to_string());
        config["disable"] = json!(["traefik", "servicelb"]);
        config["write-kubeconfig-mode"] = json!("0600");
    } else {
        config["server"] = json!(format!("https://{server_ip}:6443"));
    }
    Ok(config)
}

pub(super) fn install_script(cluster: &ClusterConfig) -> String {
    let digest = quote(&cluster.sha256.to_ascii_lowercase());
    let url = quote(&format!(
        "https://github.com/k3s-io/k3s/releases/download/{}/k3s-arm64",
        cluster.version
    ));
    format!(
        r#"wanted={digest}
if test -x /usr/local/bin/k3s && test "$(sha256sum /usr/local/bin/k3s | cut -d' ' -f1)" = "$wanted"; then exit 0; fi
mkdir -p /usr/local/bin
tmp=$(mktemp /usr/local/bin/.vzctl-k3s.XXXXXX)
trap 'rm -f "$tmp"' EXIT
curl --fail --location --proto '=https' --proto-redir '=https' --retry 2 --retry-max-time 300 --connect-timeout 15 --max-time 240 --silent --show-error {url} -o "$tmp"
echo "$wanted  $tmp" | sha256sum -c - >/dev/null
chmod 0755 "$tmp"
mv "$tmp" /usr/local/bin/k3s
echo changed
"#
    )
}

fn unit(server: bool) -> String {
    format!("[Unit]\nDescription=vzctl managed K3s\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=notify\nUser=root\nKillMode=process\nDelegate=yes\nLimitNOFILE=1048576\nLimitNPROC=infinity\nLimitCORE=infinity\nTasksMax=infinity\nTimeoutStartSec=180\nRestart=always\nRestartSec=5\nExecStartPre=-/sbin/modprobe br_netfilter\nExecStartPre=-/sbin/modprobe overlay\nExecStart=/usr/local/bin/k3s {} --config {CONFIG}\n\n[Install]\nWantedBy=multi-user.target\n", if server {"server"} else {"agent"})
}

fn guest_file(guest: &Guest<'_>, vm: &str, path: &str, value: &[u8]) -> Result<bool, String> {
    guarded_guest_file(guest, vm, path, "", value)
}

fn guarded_guest_file(
    guest: &Guest<'_>,
    vm: &str,
    path: &str,
    guard: &str,
    value: &[u8],
) -> Result<bool, String> {
    let directory = Path::new(path)
        .parent()
        .ok_or("invalid guest path")?
        .to_string_lossy();
    let script = format!(
        "umask 077; {guard}mkdir -p {}; tmp=$(mktemp {}); trap 'rm -f \"$tmp\"' EXIT; cat > \"$tmp\"; chmod 0600 \"$tmp\"; if ! cmp -s \"$tmp\" {}; then mv \"$tmp\" {}; echo changed; fi; chmod 0600 {}",
        quote(&directory),quote(&format!("{path}.vzctl.XXXXXX")),quote(path),quote(path),quote(path)
    );
    Ok(guest.shell(vm, &script, value, 30)?.trim() == "changed")
}

pub(super) fn cluster_directory(state: &Path, project: &str, cluster: &str) -> PathBuf {
    state
        .join("projects")
        .join(project)
        .join("kubernetes")
        .join(cluster)
}

pub(super) fn ensure_token(directory: &Path) -> Result<String, String> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let path = directory.join("server-token");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(mut file) => {
            let mut bytes = [0u8; 32];
            fs::File::open("/dev/urandom")
                .and_then(|mut random| random.read_exact(&mut bytes))
                .map_err(|e| e.to_string())?;
            let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
            file.write_all(token.as_bytes())
                .map_err(|e| e.to_string())?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
                .map_err(|e| e.to_string())?;
            let mut token = String::new();
            (&mut file)
                .take(65)
                .read_to_string(&mut token)
                .map_err(|e| e.to_string())?;
            if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(
                    "invalid saved K3s token; refusing to rotate cluster credentials implicitly"
                        .into(),
                );
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
            Ok(token)
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}
