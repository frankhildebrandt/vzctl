use super::*;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validate/valid-kubernetes.yaml")
}

fn fixture() -> Environment {
    crate::config::validate_path(&fixture_path()).unwrap()
}

fn temp_dir() -> PathBuf {
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "vz-k-{fixture}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn interface_selection_executes_real_shell_and_matches_exact_guest_address() {
    let root = temp_dir();
    let ip = root.join("ip");
    fs::write(
        &ip,
        "#!/bin/sh\nprintf '%s\\n' '1: lo inet 127.0.0.1/8 scope host lo' '2: eth0 inet 10.123.0.110/24 scope global eth0' '3: eth1 inet 10.123.0.11/24 scope global eth1'\n",
    )
    .unwrap();
    fs::set_permissions(&ip, fs::Permissions::from_mode(0o755)).unwrap();
    for (address, expected) in [("10.123.0.11", "eth1\n"), ("10.123.0.12", "")] {
        let output = std::process::Command::new("/bin/sh")
            .args(["-eu", "-c", &controller::interface_script(address)])
            .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    fs::remove_dir_all(root).unwrap();
}

const VERSION_CONFLICT: &str = "error: Operation cannot be fulfilled on configmaps \"example\": the object has been modified; please apply your changes to the latest version and try again";

fn apply_race_fixture(
    errors: &[&str],
    replacement: Option<&str>,
) -> (Result<(), String>, usize, Vec<String>) {
    let environment = fixture();
    let desired = manifests::owned(
        json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"example"}}),
        &environment.spec.project,
        "dev",
    )
    .unwrap();
    let mut reads = 0;
    let mut versions = Vec::new();
    let result = controller::apply_owned_object(
        &environment.spec.project,
        "dev",
        &environment.spec.kubernetes.clusters["dev"],
        &desired,
        |args, input, _| match args[0] {
            "get" => {
                reads += 1;
                if reads == 1 {
                    match replacement {
                        Some("create" | "empty-create" | "list-create") => return Ok(String::new()),
                        Some("create-empty-list") => {
                            return Ok(
                                json!({"apiVersion":"v1","kind":"List","items":[]}).to_string()
                            )
                        }
                        Some("malformed-list") => {
                            return Ok(
                                json!({"apiVersion":"v1","kind":"List","items":{}}).to_string()
                            )
                        }
                        _ => {}
                    }
                }
                let mut actual = desired.clone();
                actual["metadata"]["resourceVersion"] = json!(reads.to_string());
                actual["metadata"]["uid"] = json!("same-object");
                if reads > 1 {
                    match replacement {
                        Some("owner") => {
                            actual["metadata"]["labels"][manifests::PROJECT] = json!("other")
                        }
                        Some("uid") => actual["metadata"]["uid"] = json!("replacement-object"),
                        Some("missing") => return Ok(String::new()),
                        Some("retry-empty-list") => {
                            return Ok(
                                json!({"apiVersion":"v1","kind":"List","items":[]}).to_string()
                            )
                        }
                        Some("unchanged-version") => {
                            actual["metadata"]["resourceVersion"] = json!("1")
                        }
                        Some("missing-version") => {
                            actual["metadata"]["resourceVersion"] = Value::Null
                        }
                        Some("missing-uid") => actual["metadata"]["uid"] = Value::Null,
                        _ => {}
                    }
                }
                match replacement {
                    Some("many-list") => Ok(
                        json!({"apiVersion":"v1","kind":"List","items":[actual.clone(),actual]})
                            .to_string(),
                    ),
                    Some("wrong-kind") => {
                        actual["kind"] = json!("Secret");
                        Ok(actual.to_string())
                    }
                    Some("wrong-name") => {
                        actual["metadata"]["name"] = json!("other");
                        Ok(actual.to_string())
                    }
                    Some("wrong-namespace") => {
                        actual["metadata"]["namespace"] = json!("other");
                        Ok(actual.to_string())
                    }
                    Some("uidless-item") => {
                        actual["metadata"]["uid"] = Value::Null;
                        Ok(actual.to_string())
                    }
                    Some("concrete") => Ok(actual.to_string()),
                    _ => Ok(json!({"apiVersion":"v1","kind":"List","items":[actual]}).to_string()),
                }
            }
            "create" => {
                assert!(matches!(
                    replacement,
                    Some("create" | "create-empty-list" | "empty-create" | "list-create")
                ));
                assert!(args.contains(&"--field-manager=vzctl-kubernetes"));
                let mut created = desired.clone();
                created["metadata"]["resourceVersion"] = json!("1");
                created["metadata"]["uid"] = json!("same-object");
                match replacement {
                    Some("empty-create") => Ok(String::new()),
                    Some("list-create") => {
                        Ok(json!({"apiVersion":"v1","kind":"List","items":[created]}).to_string())
                    }
                    _ => Ok(created.to_string()),
                }
            }
            "apply" => {
                assert!(args.contains(&"--server-side"));
                assert!(!args.iter().any(|arg| arg.contains("force")));
                let object: Value = serde_json::from_slice(input).unwrap();
                assert_eq!(object["metadata"]["uid"], "same-object");
                versions.push(
                    object["metadata"]["resourceVersion"]
                        .as_str()
                        .unwrap()
                        .into(),
                );
                match errors.get(versions.len() - 1) {
                    Some(error) => Err((*error).into()),
                    None => Ok(String::new()),
                }
            }
            command => panic!("unexpected Kubernetes operation: {command}"),
        },
    );
    (result, reads, versions)
}

#[test]
fn resource_version_conflict_rereads_and_retries_with_latest_owned_version() {
    for replacement in [
        None,
        Some("create"),
        Some("create-empty-list"),
        Some("concrete"),
    ] {
        let (result, reads, versions) = apply_race_fixture(&[VERSION_CONFLICT], replacement);
        result.unwrap();
        assert_eq!(reads, 2);
        assert_eq!(versions, ["1", "2"]);
    }
}

#[test]
fn resource_version_retry_is_bounded_and_never_retries_other_api_errors() {
    let (result, reads, versions) = apply_race_fixture(&[VERSION_CONFLICT; 3], None);
    assert!(result.unwrap_err().contains("modified"));
    assert_eq!(reads, 3);
    assert_eq!(versions, ["1", "2", "3"]);
    for error in [
        "Forbidden",
        "timeout",
        "Apply failed with 1 conflict: .data.key",
        "AlreadyExists",
    ] {
        let (result, reads, versions) = apply_race_fixture(&[error], None);
        assert_eq!(result.unwrap_err(), error);
        assert_eq!(reads, 1);
        assert_eq!(versions, ["1"]);
    }
}

#[test]
fn resource_version_retry_refuses_changed_ownership_replacement_and_disappearance() {
    for replacement in [
        "owner",
        "uid",
        "missing",
        "retry-empty-list",
        "unchanged-version",
        "missing-version",
        "missing-uid",
    ] {
        let (result, reads, versions) = apply_race_fixture(&[VERSION_CONFLICT], Some(replacement));
        assert!(result.is_err(), "{replacement}");
        assert_eq!(reads, 2, "{replacement}");
        assert_eq!(versions, ["1"], "{replacement}");
    }
}

#[test]
fn resource_inventory_refuses_ambiguous_malformed_uidless_or_wrong_identity() {
    for replacement in [
        "many-list",
        "malformed-list",
        "wrong-kind",
        "wrong-name",
        "wrong-namespace",
        "uidless-item",
        "empty-create",
        "list-create",
    ] {
        let (result, reads, versions) = apply_race_fixture(&[], Some(replacement));
        assert!(result.is_err(), "{replacement}");
        assert_eq!(reads, 1, "{replacement}");
        assert!(versions.is_empty(), "{replacement}");
    }
}

#[test]
fn cluster_config_rejects_unpinned_release_collision_and_duplicate_node() {
    let source = fs::read_to_string(fixture_path()).unwrap();
    for changed in [
        source.replace("v1.33.4+k3s1", "latest"),
        source.replace("podCIDR: 10.42.0.0/16", "podCIDR: 10.90.0.0/16"),
        source.replace("podCIDR: 10.42.0.0/16", "podCIDR: 10.42.0.0/24"),
        source.replace("agents: [kube-worker]", "agents: [kube-server]"),
        source.replace("network: lan", "network: missing"),
        source.replace("serviceCIDR: 10.43.0.0/16", "serviceCIDR: 10.42.0.0/16"),
        source.replace(
            "manifests: [kubernetes/workloads.yaml]",
            "manifests: [https://example.com/workload.yaml]",
        ),
    ] {
        assert!(
            crate::config::validate_source(&changed).is_err(),
            "{changed}"
        );
    }
    assert!(crate::config::validate_path(&fixture_path()).is_ok());
}

#[test]
fn missing_invalid_and_oversized_manifests_fail_validation() {
    let root = temp_dir();
    let source = fs::read_to_string(fixture_path())
        .unwrap()
        .replace("kubernetes/workloads.yaml", "app.yaml");
    fs::write(root.join("hypernetwork.config.yaml"), &source).unwrap();
    assert!(crate::config::validate_path(&root).is_err());
    fs::write(root.join("app.yaml"), "kind: Deployment\n").unwrap();
    assert!(crate::config::validate_path(&root).is_err());
    fs::write(root.join("app.yaml"), vec![b'x'; 1_048_577]).unwrap();
    assert!(manifests::read(&root.join("app.yaml"))
        .unwrap_err()
        .contains("1 MiB"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn configs_keep_api_guest_only_tokens_in_files_and_disable_host_port_controllers() {
    let environment = fixture();
    let cluster = &environment.spec.kubernetes.clusters["dev"];
    let server = controller::node_config(&environment, cluster, &cluster.server, "eth0").unwrap();
    assert_eq!(server["bind-address"], "10.90.0.10");
    assert_eq!(server["tls-san"], json!(["10.90.0.10"]));
    assert_eq!(server["disable"], json!(["traefik", "servicelb"]));
    assert_eq!(server["cluster-dns"], "10.43.0.10");
    assert_eq!(server["write-kubeconfig-mode"], "0600");
    assert!(server.get("token").is_none());
    let agent = controller::node_config(&environment, cluster, "kube-worker", "eth0").unwrap();
    assert_eq!(agent["server"], "https://10.90.0.10:6443");
    assert!(agent.get("disable").is_none());
    let script = controller::install_script(cluster);
    assert!(script.contains("releases/download/v1.33.4+k3s1/k3s-arm64"));
    assert!(script.contains("sha256sum -c"));
    assert!(!script.contains("| sh\n"));
    assert!(script.find("sha256sum -c").unwrap() < script.find("mv \"$tmp\"").unwrap());
}

#[test]
fn node_resolver_is_pinned_only_for_stack_dns_and_the_explicit_cluster_network() {
    let mut environment = fixture();
    let cluster = environment.spec.kubernetes.clusters["dev"].clone();
    for node in [&cluster.server, "kube-worker"] {
        let config = controller::node_config(&environment, &cluster, node, "eth0").unwrap();
        assert_eq!(config["resolv-conf"], "/etc/rancher/k3s/vzctl-resolv.conf");
    }
    environment.spec.dns.enabled = false;
    assert!(
        controller::node_config(&environment, &cluster, &cluster.server, "eth0")
            .unwrap()
            .get("resolv-conf")
            .is_none()
    );
    environment.spec.dns.enabled = true;
    let mut wrong = cluster.clone();
    wrong.network = "foreign".into();
    assert!(controller::node_config(&environment, &wrong, &cluster.server, "eth0").is_err());
    // Duplicate attachment names cannot make the chosen cluster interface arbitrary.
    let attachment = environment.spec.vms[&cluster.server].networks[0].clone();
    environment
        .spec
        .vms
        .get_mut(&cluster.server)
        .unwrap()
        .networks
        .push(attachment);
    assert!(controller::node_config(&environment, &cluster, &cluster.server, "eth0").is_err());
}

#[test]
fn manifests_refuse_foreign_ownership_and_preserve_custom_fields() {
    let object = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"x"},"data":{"hello":"world"}});
    assert!(manifests::check_existing(&object, "p", "c").is_err());
    let managed = manifests::owned(object, "p", "c").unwrap();
    assert_eq!(managed["data"]["hello"], "world");
    assert!(manifests::check_existing(&managed, "p", "c").is_ok());
    assert!(manifests::check_existing(&managed, "other", "c").is_err());
    assert!(manifests::owned(managed, "other", "c").is_err());
}

#[test]
fn tokens_are_private_stable_and_never_implicitly_rotated() {
    let root = temp_dir();
    let token = controller::ensure_token(&root).unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(controller::ensure_token(&root).unwrap(), token);
    assert_eq!(
        fs::metadata(root.join("server-token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::write(root.join("server-token"), "broken").unwrap();
    assert!(controller::ensure_token(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolver_selects_cluster_network_not_first_attachment_or_ingress() {
    let mut environment = fixture();
    let cluster = environment.spec.kubernetes.clusters["dev"].clone();
    let mut other = environment.spec.networks[&cluster.network].clone();
    other.cidr = "10.91.0.0/24".into();
    environment.spec.networks.insert("other".into(), other);
    for vm in std::iter::once(&cluster.server).chain(cluster.agents.iter()) {
        let node = environment.spec.vms.get_mut(vm).unwrap();
        let mut extra = node.networks[0].clone();
        extra.name = "other".into();
        extra.ip = "10.91.0.10".into();
        node.networks.insert(0, extra);
    }
    assert_eq!(
        super::resolver::upstream(&environment, &cluster)
            .unwrap()
            .as_deref(),
        Some("10.90.0.0")
    );
    for invalid in ["10.91.0.10", "127.0.0.1", "10.90.0.0"] {
        let mut bad = environment.clone();
        bad.spec.vms.get_mut(&cluster.server).unwrap().networks[1].ip = invalid.into();
        assert!(super::resolver::upstream(&bad, &cluster).is_err());
    }
    for (cidr, ip, accepted) in [
        ("100.64.0.0/24", "100.64.0.10", true),
        ("127.0.0.0/24", "127.0.0.10", false),
        ("169.254.0.0/24", "169.254.0.10", false),
        ("224.0.0.0/24", "224.0.0.10", false),
    ] {
        let mut changed = environment.clone();
        changed
            .spec
            .networks
            .get_mut(&cluster.network)
            .unwrap()
            .cidr = cidr.into();
        for vm in std::iter::once(&cluster.server).chain(cluster.agents.iter()) {
            changed.spec.vms.get_mut(vm).unwrap().networks[1].ip = ip.into();
        }
        assert_eq!(
            super::resolver::upstream(&changed, &cluster).is_ok(),
            accepted,
            "{cidr}"
        );
    }
    environment.spec.dns.enabled = false;
    assert_eq!(
        super::resolver::upstream(&environment, &cluster).unwrap(),
        None
    );
}

#[test]
fn kubeconfig_export_changes_only_endpoint_and_keeps_tls_verification() {
    let mut value = json!({
        "clusters":[{"name":"default","cluster":{"server":"https://127.0.0.1:6443","certificate-authority-data":"CA"}}],
        "users":[{"name":"default","user":{"client-key-data":"PRIVATE","client-certificate-data":"CERT"}}]
    });
    let user = value["users"].clone();
    cli::rewrite_kubeconfig(&mut value, "10.90.0.10").unwrap();
    assert_eq!(
        value["clusters"][0]["cluster"]["server"],
        "https://10.90.0.10:6443"
    );
    assert_eq!(value["users"], user);
    value["clusters"][0]["cluster"]["insecure-skip-tls-verify"] = json!(true);
    assert!(cli::rewrite_kubeconfig(&mut value, "10.90.0.10").is_err());
}

#[test]
fn repeated_ensure_preserves_node_configuration_and_refuses_foreign_resources() {
    use base64::Engine;
    use serde_json::Value;
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Fake {
        files: BTreeMap<(String, String), Vec<u8>>,
        binaries: BTreeSet<String>,
        objects: BTreeMap<String, Value>,
        restarts: usize,
        applies: usize,
        dns_patches: usize,
        dns_deployment: Option<Value>,
        requests: Vec<Value>,
    }
    let root = temp_dir();
    let socket = PathBuf::from(format!(
        "/tmp/vzk-{}.sock",
        root.file_name().unwrap().to_string_lossy()
    ));
    let listener = UnixListener::bind(&socket).unwrap();
    let fake = Arc::new(Mutex::new(Fake::default()));
    let server_fake = fake.clone();
    let server = std::thread::spawn(move || {
        for connection in listener.incoming() {
            let mut stream = connection.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line == "stop\n" {
                break;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            let params = &request["params"];
            let vm = params["vm_id"].as_str().unwrap().to_owned();
            let argv: Vec<&str> = params["cmd"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect();
            let input = base64::engine::general_purpose::STANDARD
                .decode(params["stdin_b64"].as_str().unwrap())
                .unwrap();
            let mut state = server_fake.lock().unwrap();
            let mut output = String::new();
            if argv.contains(&"kubectl") {
                let command = argv[8];
                if argv.contains(&"deployment")
                    || argv.contains(&"deployment/coredns")
                    || argv.contains(&"addons.k3s.cattle.io")
                {
                    let (addon, deployment) = super::resolver::tests::objects();
                    match command {
                        "get" if argv.contains(&"addons.k3s.cattle.io") => {
                            output = addon.to_string()
                        }
                        "get" => {
                            output = state
                                .dns_deployment
                                .clone()
                                .unwrap_or(deployment)
                                .to_string()
                        }
                        "patch" => {
                            assert!(
                                state.restarts >= 2 && state.restarts % 2 == 0,
                                "CoreDNS migration only after all nodes installed"
                            );
                            let patch: Value = serde_json::from_slice(&input).unwrap();
                            let mut current = state.dns_deployment.clone().unwrap_or(deployment);
                            current["spec"]["template"]["metadata"]["annotations"] =
                                patch[2]["value"].clone();
                            output = current.to_string();
                            state.dns_deployment = Some(current);
                            state.dns_patches += 1;
                        }
                        "rollout" | "wait" => {}
                        _ => panic!("unexpected DNS command {argv:?}"),
                    }
                } else if command == "get" && argv.contains(&"node") {
                    output = json!({"status":{"nodeInfo":{"kubeletVersion":"v1.33.4+k3s1"},"conditions":[{"type":"Ready","status":"True"}]}}).to_string();
                } else if command == "get" && argv.contains(&"namespace") {
                    if state.objects.contains_key("Namespace/app") {
                        output = "namespace/app".into();
                    }
                } else if command != "wait" {
                    let mut object: Value = serde_json::from_slice(&input).unwrap();
                    let key = format!(
                        "{}/{}",
                        object["kind"].as_str().unwrap(),
                        object["metadata"]["name"].as_str().unwrap()
                    );
                    if command == "get" {
                        output = state
                            .objects
                            .get(&key)
                            .map(ToString::to_string)
                            .unwrap_or_default();
                    } else {
                        assert!(command == "create" || command == "apply", "{argv:?}");
                        assert!(!argv.contains(&"--force-conflicts"));
                        if command == "apply" {
                            assert_eq!(object["metadata"]["resourceVersion"], "1");
                            state.applies += 1;
                        } else {
                            assert!(!state.objects.contains_key(&key));
                        }
                        object["metadata"]["resourceVersion"] = json!("1");
                        object["metadata"]["uid"] = json!(key);
                        output = object.to_string();
                        state.objects.insert(key, object);
                    }
                }
            } else {
                let script = argv.last().unwrap();
                // Execute the real shell parser before simulating privileged side effects.
                assert_eq!(&argv[..5], &["sudo", "-n", "/bin/sh", "-eu", "-c"]);
                let syntax = std::process::Command::new("/bin/sh")
                    .args(["-n", "-c", script])
                    .output()
                    .unwrap();
                assert!(
                    syntax.status.success(),
                    "{}",
                    String::from_utf8_lossy(&syntax.stderr)
                );
                if script.starts_with("umask 077;") {
                    let path = script
                        .rsplit("chmod 0600 '")
                        .next()
                        .unwrap()
                        .trim_end_matches('\'')
                        .to_string();
                    let key = (vm.clone(), path);
                    if state.files.get(&key) != Some(&input) {
                        output = "changed".into();
                        state.files.insert(key, input);
                    }
                } else if script.starts_with("if test -f /var/lib/vzctl/kubernetes/owner.json") {
                    output = state
                        .files
                        .get(&(vm.clone(), "/var/lib/vzctl/kubernetes/owner.json".into()))
                        .map(|v| String::from_utf8(v.clone()).unwrap())
                        .unwrap_or_default();
                } else if script.starts_with("if test -L ") {
                    // Dedicated guard has its own real-shell ownership regression.
                    if script.contains("rm -f --") {
                        state
                            .files
                            .remove(&(vm.clone(), super::resolver::PATH.into()));
                    }
                } else if script.starts_with("ip -o") {
                    output = "eth0\n".into();
                } else if script.starts_with("wanted=") {
                    if state.binaries.insert(vm.clone()) {
                        output = "changed\n".into();
                    }
                } else if script.contains("systemctl restart") {
                    state.restarts += 1;
                } else if *script == "cat /var/lib/rancher/k3s/server/token" {
                    output = "K10secure-server-join-token".into();
                } else {
                    assert!(
                        script.starts_with("test \"$(cat /proc/1/comm)")
                            || script.starts_with("systemctl enable --now"),
                        "unexpected command {script}"
                    );
                }
            }
            state.requests.push(request.clone());
            writeln!(stream,"{}",json!({"jsonrpc":"2.0","id":request["id"],"result":{"exit":0,"stdout":output,"stderr":"","truncated":false}})).unwrap();
        }
    });
    let environment = fixture();
    let mut progress =
        crate::progress::ProgressReporter::new(crate::progress::ProgressMode::Off, None, &[]);
    controller::ensure(&environment, &fixture_path(), &socket, &root, &mut progress).unwrap();
    let first_files = fake.lock().unwrap().files.clone();
    controller::ensure(&environment, &fixture_path(), &socket, &root, &mut progress).unwrap();
    {
        let state = fake.lock().unwrap();
        assert_eq!(
            state.restarts, 2,
            "one initial restart per node, none on unchanged ensure"
        );
        assert_eq!(state.files, first_files);
        assert_eq!(state.dns_patches, 1);
        for vm in ["kube-lab/kube-server", "kube-lab/kube-worker"] {
            let resolver = state
                .files
                .get(&(vm.into(), super::resolver::PATH.into()))
                .unwrap();
            assert!(String::from_utf8_lossy(resolver).contains("nameserver 10.90.0.0"));
        }
        assert_eq!(state.applies, 13);
        assert_eq!(
            state
                .requests
                .iter()
                .filter(|request| request["params"]["cmd"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|arg| arg == "--for=condition=Established"))
                .count(),
            8
        );
        let token = fs::read_to_string(
            controller::cluster_directory(&root, &environment.spec.project, "dev")
                .join("server-token"),
        )
        .unwrap();
        for request in &state.requests {
            let argv = request["params"]["cmd"].to_string();
            assert!(!argv.contains(&token));
            assert!(!argv.contains("K10secure-server-join-token"));
        }
    }
    let mut disabled = environment.clone();
    disabled.spec.dns.enabled = false;
    controller::ensure(&disabled, &fixture_path(), &socket, &root, &mut progress).unwrap();
    controller::ensure(&disabled, &fixture_path(), &socket, &root, &mut progress).unwrap();
    {
        let state = fake.lock().unwrap();
        assert_eq!(state.restarts, 4);
        assert_eq!(state.dns_patches, 2);
        assert!(!state
            .files
            .keys()
            .any(|(_, path)| path == super::resolver::PATH));
        for ((_, path), bytes) in &state.files {
            if path == "/etc/rancher/k3s/config.yaml" {
                assert!(!String::from_utf8_lossy(bytes).contains("resolv-conf"));
            }
        }
    }
    controller::ensure(&environment, &fixture_path(), &socket, &root, &mut progress).unwrap();
    controller::ensure(&environment, &fixture_path(), &socket, &root, &mut progress).unwrap();
    {
        let mut state = fake.lock().unwrap();
        assert_eq!(state.restarts, 6);
        assert_eq!(state.dns_patches, 3);
        assert_eq!(state.files, first_files);
        state.objects.get_mut("ConfigMap/example").unwrap()["metadata"]["labels"]
            [manifests::PROJECT] = json!("foreign-project");
    }
    let error = controller::ensure(&environment, &fixture_path(), &socket, &root, &mut progress)
        .unwrap_err();
    assert!(error.contains("ownership conflict"), "{error}");
    let mut stop = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stop.write_all(b"stop\n").unwrap();
    server.join().unwrap();
    fs::remove_file(socket).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn readiness_requires_the_pinned_kubelet_version() {
    let node = json!({"status":{"nodeInfo":{"kubeletVersion":"v1.33.4+k3s1"},"conditions":[{"type":"Ready","status":"True"}]}});
    assert!(controller::node_ready(&node, "v1.33.4+k3s1"));
    assert!(!controller::node_ready(&node, "v1.33.5+k3s1"));
    assert!(!controller::node_ready(&json!({}), "v1.33.4+k3s1"));
}

// Stateful API fixture: Update and Apply entries with the same manager still
// represent distinct field owners. Equal SSA shares ownership; changing a value
// conflicts until the precise old Create/Update entry is retired.
struct ManagedFieldsApi {
    current: Option<Value>,
    events: Vec<String>,
    patch_race: Option<&'static str>,
    patches: usize,
    fields_visible: bool,
}

fn managed_entry(manager: &str, operation: &str) -> Value {
    json!({"manager":manager,"operation":operation,"apiVersion":"v1","fieldsType":"FieldsV1",
        "fieldsV1":{"f:data":{"f:ca.crt":{}},"f:metadata":{"f:annotations":{"f:vzctl.dev/manifest-sha256":{}}}}})
}

fn managed_config(value: &str) -> Value {
    manifests::owned(json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"proof-code","namespace":"app"},"data":{"ca.crt":value}}), "edge-dmz", "dev").unwrap()
}

impl ManagedFieldsApi {
    fn existing() -> Self {
        let mut value = managed_config("old-public-ca");
        value["metadata"]["uid"] = json!("original-uid");
        value["metadata"]["resourceVersion"] = json!("10");
        value["metadata"]["managedFields"] = json!([
            managed_entry("vzctl-kubernetes", "Apply"),
            managed_entry("vzctl-kubernetes", "Update"),
            {"manager":"status-controller","operation":"Update","apiVersion":"v1","subresource":"status","fieldsType":"FieldsV1","fieldsV1":{"f:status":{}}}
        ]);
        Self {
            current: Some(value),
            events: vec![],
            patch_race: None,
            patches: 0,
            fields_visible: true,
        }
    }

    fn call(&mut self, args: &[&str], input: &[u8], _seconds: u64) -> Result<String, String> {
        self.events.push(args[0].into());
        assert!(!args.iter().any(|s| s.contains("force")));
        match args[0] {
            "get" => {
                self.fields_visible &= args.contains(&"--show-managed-fields=true");
                let mut value = self.current.clone();
                if self.patches > 0 {
                    if let Some(v) = &mut value {
                        match self.patch_race {
                            Some("uid") => v["metadata"]["uid"] = json!("recreated-uid"),
                            Some("owner") => {
                                v["metadata"]["labels"][manifests::PROJECT] = json!("foreign")
                            }
                            Some("reintroduced") => v["metadata"]["managedFields"]
                                .as_array_mut()
                                .unwrap()
                                .push(managed_entry("vzctl-kubernetes", "Update")),
                            Some("missing") => value = None,
                            _ => {}
                        }
                    }
                }
                Ok(json!({"apiVersion":"v1","kind":"List","items":value.into_iter().collect::<Vec<_>> ()}).to_string())
            }
            "create" => {
                assert!(self.current.is_none());
                let mut value: Value = serde_json::from_slice(input).unwrap();
                value["metadata"]["uid"] = json!("original-uid");
                value["metadata"]["resourceVersion"] = json!("10");
                value["metadata"]["managedFields"] =
                    json!([managed_entry("vzctl-kubernetes", "Update")]);
                self.current = Some(value.clone());
                Ok(value.to_string())
            }
            "patch" => {
                self.patches += 1;
                if self.patch_race == Some("version") {
                    return Err("JSON patch test failed: resourceVersion".into());
                }
                let patch: Value = serde_json::from_str(
                    args[args.iter().position(|s| *s == "--patch").unwrap() + 1],
                )
                .unwrap();
                assert_eq!(
                    patch[0],
                    json!({"op":"test","path":"/metadata/uid","value":"original-uid"})
                );
                let current = self.current.as_mut().unwrap();
                assert_eq!(
                    patch[1],
                    json!({"op":"test","path":"/metadata/resourceVersion","value":current["metadata"]["resourceVersion"]})
                );
                assert_eq!(
                    patch[2],
                    json!({"op":"test","path":"/metadata/managedFields","value":current["metadata"]["managedFields"]})
                );
                for op in patch.as_array().unwrap().iter().skip(3) {
                    assert_eq!(op["op"], "remove");
                    let index: usize = op["path"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("/metadata/managedFields/")
                        .unwrap()
                        .parse()
                        .unwrap();
                    let removed = current["metadata"]["managedFields"]
                        .as_array_mut()
                        .unwrap()
                        .remove(index);
                    assert_eq!(removed, managed_entry("vzctl-kubernetes", "Update"));
                }
                current["metadata"]["resourceVersion"] = json!("11");
                Ok(current.to_string())
            }
            "apply" => {
                let desired: Value = serde_json::from_slice(input).unwrap();
                let current = self.current.as_mut().unwrap();
                assert_eq!(desired["metadata"]["uid"], current["metadata"]["uid"]);
                assert_eq!(
                    desired["metadata"]["resourceVersion"],
                    current["metadata"]["resourceVersion"]
                );
                let managers = current["metadata"]["managedFields"].as_array_mut().unwrap();
                let has_conflict = managers.iter().any(|f| {
                    f["subresource"].is_null()
                        && (f["manager"] != "vzctl-kubernetes" || f["operation"] != "Apply")
                });
                if has_conflict && desired["data"] != current["data"] {
                    return Err("Apply failed with 2 conflicts: .data.ca.crt and .metadata.annotations.vzctl.dev/manifest-sha256".into());
                }
                let managers = current["metadata"]["managedFields"].as_array_mut().unwrap();
                if !managers
                    .iter()
                    .any(|f| f["manager"] == "vzctl-kubernetes" && f["operation"] == "Apply")
                {
                    managers.push(managed_entry("vzctl-kubernetes", "Apply"));
                }
                current["data"] = desired["data"].clone();
                current["metadata"]["annotations"] = desired["metadata"]["annotations"].clone();
                Ok(current.to_string())
            }
            other => panic!("unexpected {other}"),
        }
    }

    fn apply(&mut self, desired: &Value) -> Result<(), String> {
        let config = fixture();
        controller::apply_owned_object(
            "edge-dmz",
            "dev",
            &config.spec.kubernetes.clusters["dev"],
            desired,
            |a, i, t| self.call(a, i, t),
        )
    }
}

#[test]
fn managed_fields_retires_only_own_create_update_before_changed_ssa() {
    let mut api = ManagedFieldsApi::existing();
    let foreign = api.current.as_ref().unwrap()["metadata"]["managedFields"][2].clone();
    api.apply(&managed_config("new-public-ca")).unwrap();
    assert!(api.fields_visible);
    assert_eq!(api.patches, 1);
    assert_eq!(api.events, ["get", "patch", "get", "apply"]);
    assert_eq!(
        api.current.as_ref().unwrap()["metadata"]["managedFields"],
        json!([managed_entry("vzctl-kubernetes", "Apply"), foreign])
    );
    api.apply(&managed_config("next-public-ca")).unwrap();
    assert_eq!(api.patches, 1, "cleanup must be idempotent");
}

#[test]
fn managed_fields_new_create_establishes_apply_then_removes_update() {
    let mut api = ManagedFieldsApi::existing();
    api.current = None;
    api.apply(&managed_config("first-public-ca")).unwrap();
    assert_eq!(
        api.events,
        ["get", "create", "apply", "get", "patch", "get", "apply"]
    );
    api.apply(&managed_config("second-public-ca")).unwrap();
    assert_eq!(api.patches, 1);
}

#[test]
fn managed_fields_preserves_foreign_conflicts_and_bridges_only_update() {
    let mut api = ManagedFieldsApi::existing();
    let foreign = managed_entry("another-deployer", "Apply");
    api.current.as_mut().unwrap()["metadata"]["managedFields"]
        .as_array_mut()
        .unwrap()
        .push(foreign.clone());
    assert!(api
        .apply(&managed_config("changed"))
        .unwrap_err()
        .contains("conflicts"));
    assert!(api.current.as_ref().unwrap()["metadata"]["managedFields"]
        .as_array()
        .unwrap()
        .contains(&foreign));
    assert_eq!(api.events.iter().filter(|e| *e == "apply").count(), 1);
    let mut api = ManagedFieldsApi::existing();
    api.current.as_mut().unwrap()["metadata"]["managedFields"] =
        json!([managed_entry("vzctl-kubernetes", "Update")]);
    api.apply(&managed_config("changed")).unwrap();
    assert_eq!(api.events, ["get", "apply", "get", "patch", "get", "apply"]);
    assert_eq!(api.current.as_ref().unwrap()["data"]["ca.crt"], "changed");
}

#[test]
fn managed_fields_migration_refuses_uid_owner_version_or_inventory_races() {
    for race in ["uid", "owner", "version", "missing", "reintroduced"] {
        let mut api = ManagedFieldsApi::existing();
        api.patch_race = Some(race);
        assert!(api.apply(&managed_config("changed")).is_err(), "{race}");
        assert!(!api.events.iter().any(|event| event == "apply"), "{race}");
        assert_eq!(api.patches, 1, "{race}");
    }
}
