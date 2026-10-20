use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;

#[test]
fn kubeconfig_export_is_private_explicit_and_never_prints_credentials() {
    let root = PathBuf::from(format!(
        "/tmp/vzk-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line == "stop\n" {
                break;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "vm.exec");
            assert_eq!(
                request["params"]["cmd"],
                json!(["sudo", "-n", "cat", "/etc/rancher/k3s/k3s.yaml"])
            );
            let kubeconfig = json!({"apiVersion":"v1","clusters":[{"name":"default","cluster":{"server":"https://127.0.0.1:6443","certificate-authority-data":"CA"}}],"users":[{"name":"default","user":{"client-key-data":"PRIVATE-KEY","client-certificate-data":"CERT"}}]});
            writeln!(stream,"{}",json!({"jsonrpc":"2.0","id":request["id"],"result":{"exit":0,"stdout":kubeconfig.to_string(),"stderr":""}})).unwrap();
        }
    });
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate/valid-kubernetes.yaml");
    let output_file = root.join("kubeconfig.yaml");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(["kubernetes", "kubeconfig", "-C"])
            .arg(&config)
            .args(["--cluster", "dev", "--output"])
            .arg(&output_file)
            .args(["--format", "json"])
            .env("VZCTL_STATE_DIR", &root)
            .output()
            .unwrap()
    };
    let output = run();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "kubernetes.kubeconfig");
    assert_eq!(value["exit_code"], 0);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE-KEY"));
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::metadata(&output_file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let contents = fs::read_to_string(&output_file).unwrap();
    assert!(contents.contains("PRIVATE-KEY"));
    assert!(contents.contains("https://10.90.0.10:6443"));
    assert!(!contents.contains("insecure-skip-tls-verify"));
    let repeated = run();
    assert_eq!(repeated.status.code(), Some(24));
    assert_eq!(fs::read_to_string(&output_file).unwrap(), contents);
    UnixStream::connect(&socket)
        .unwrap()
        .write_all(b"stop\n")
        .unwrap();
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn degraded_cluster_status_is_a_warning_with_runtime_exit_code() {
    let root = PathBuf::from(format!("/tmp/vzk-down-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate/valid-kubernetes.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["kubernetes", "status", "-C"])
        .arg(config)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(24));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "warn");
    assert_eq!(value["exit_code"], 24);
    assert_eq!(value["command"], "kubernetes.status");
    assert_eq!(value["data"]["clusters"][0]["state"], "degraded");
    assert_eq!(
        value["data"]["clusters"][0]["issues"][0]["code"],
        "NodeInventoryUnavailable"
    );
    assert_eq!(
        value["data"]["clusters"][0]["issues"][0]["operation"],
        "nodes.get"
    );
    assert!(value["summary"]["message"]
        .as_str()
        .unwrap()
        .contains("degraded"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn kubernetes_status_reports_resource_failure_codes_without_guest_error_content() {
    let root = PathBuf::from(format!("/tmp/vzk-status-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let peer = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line == "stop\n" {
                break;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "vm.exec");
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":request["id"],
                "result":{"exit":1,"stdout":"","stderr":"PRIVATE-PROVIDER-CREDENTIAL"}})
            )
            .unwrap();
        }
    });
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate/valid-kubernetes.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["kubernetes", "status", "-C"])
        .arg(config)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &root)
        .output()
        .unwrap();
    UnixStream::connect(&socket)
        .unwrap()
        .write_all(b"stop\n")
        .unwrap();
    peer.join().unwrap();
    assert_eq!(output.status.code(), Some(24));
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE-PROVIDER"));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = value["data"]["clusters"][0]["issues"].as_array().unwrap();
    assert!(issues
        .iter()
        .any(|issue| issue["code"] == "ResourceReadFailed"
            && issue["operation"] == "resource.get"
            && issue["kind"] == "ConfigMap"
            && issue["name"] == "example"
            && issue["namespace"] == "app"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ready_status_accepts_real_kubectl_singleton_list_inventory() {
    use base64::Engine;
    let root = PathBuf::from(format!("/tmp/vzk-list-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let peer = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line == "stop\n" {
                break;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            let input = base64::engine::general_purpose::STANDARD
                .decode(request["params"]["stdin_b64"].as_str().unwrap())
                .unwrap();
            let response = if input.is_empty() {
                json!({"apiVersion":"v1","kind":"NodeList","items":(["kube-server","kube-worker"].map(|name|
                    json!({"metadata":{"name":name},"status":{"nodeInfo":{"kubeletVersion":"v1.33.4+k3s1"},
                        "conditions":[{"type":"Ready","status":"True"}]}})))})
            } else {
                let identity: Value = serde_json::from_slice(&input).unwrap();
                // Recorded desired fixture digests, plus normal API-generated metadata.
                let (mut resource, digest) = if identity["kind"] == "Namespace" {
                    (
                        json!({"apiVersion":"v1","kind":"Namespace","metadata":{"name":"app"}}),
                        "aa0fca28e2245254a75641cfd221bdbe42e2550926799f844b5ed4332047ab30",
                    )
                } else {
                    (
                        json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"example","namespace":"app"},"data":{"hello":"world"}}),
                        "fea241e7991e715dc5e24bc5f07f845b5afb1c4500eb1a8fd55c76f9eedf1fc1",
                    )
                };
                resource["metadata"]["labels"] = json!({"vzctl.dev/managed-by":"vzctl","vzctl.dev/project":"kube-lab","vzctl.dev/cluster":"dev"});
                resource["metadata"]["annotations"] = json!({"vzctl.dev/manifest-sha256":digest});
                resource["metadata"]["uid"] = json!("uid-1");
                resource["metadata"]["resourceVersion"] = json!("25");
                json!({"apiVersion":"v1","kind":"List","items":[resource]})
            };
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":request["id"],
                "result":{"exit":0,"stdout":response.to_string(),"stderr":""}})
            )
            .unwrap();
        }
    });
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate/valid-kubernetes.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["kubernetes", "status", "-C"])
        .arg(config)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &root)
        .output()
        .unwrap();
    UnixStream::connect(&socket)
        .unwrap()
        .write_all(b"stop\n")
        .unwrap();
    peer.join().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["clusters"][0]["state"], "ready");
    assert!(value["data"]["clusters"][0]["issues"]
        .as_array()
        .unwrap()
        .is_empty());
    fs::remove_dir_all(root).unwrap();
}

/// Observe the public status contract with an API-defaulted Deployment and Service.
fn status_with_defaulted_workload(change: fn(&mut Value)) -> std::process::Output {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    // SystemTime can have identical ticks in parallel tests. The process-local
    // sequence keeps independently owned UDS fixtures distinct even then.
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = PathBuf::from(format!(
        "/tmp/vzk-defaults-{}-{fixture}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let desired = json!([
        {"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"proof","namespace":"app"},
         "spec":{"replicas":1,"selector":{"matchLabels":{"app":"proof"}},"template":{
           "metadata":{"labels":{"app":"proof"}},"spec":{
             "containers":[{"name":"proof","image":"example.invalid/proof:v1","args":["serve","8080"],
               "ports":[{"containerPort":8080}],
               "readinessProbe":{"httpGet":{"path":"/","port":8080}}},
               {"name":"sidecar","image":"example.invalid/sidecar:v1"}],
             "volumes":[{"name":"config","configMap":{"name":"proof"}}]}}}},
        {"apiVersion":"v1","kind":"Service","metadata":{"name":"proof","namespace":"app"},
         "spec":{"selector":{"app":"proof"},"ports":[{"port":8080,"targetPort":8080}]}}
    ]);
    fs::write(
        root.join("workloads.yaml"),
        json!({"apiVersion":"v1","kind":"List","items":desired}).to_string(),
    )
    .unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate/valid-kubernetes.yaml");
    let mut config: Value = serde_yaml::from_slice(&fs::read(fixture).unwrap()).unwrap();
    config["spec"]["kubernetes"]["clusters"]["dev"]["manifests"] = json!(["workloads.yaml"]);
    fs::write(root.join("hypernetwork.config.yaml"), config.to_string()).unwrap();
    let mut observed = desired;
    for resource in observed.as_array_mut().unwrap() {
        // Ownership/digest reflect the declared document before API defaulting.
        resource["metadata"]["labels"] = json!({"vzctl.dev/managed-by":"vzctl",
            "vzctl.dev/project":"kube-lab","vzctl.dev/cluster":"dev"});
        resource["metadata"]["annotations"] = json!({});
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(resource).unwrap())
        );
        resource["metadata"]["annotations"]["vzctl.dev/manifest-sha256"] = json!(digest);
        resource["metadata"]["uid"] = json!("owned-uid");
        resource["metadata"]["resourceVersion"] = json!("42");
    }
    let pod = &mut observed[0]["spec"]["template"]["spec"];
    for container in pod["containers"].as_array_mut().unwrap() {
        container["imagePullPolicy"] = json!("IfNotPresent");
        container["terminationMessagePath"] = json!("/dev/termination-log");
        container["terminationMessagePolicy"] = json!("File");
    }
    pod["containers"][0]["ports"][0]["protocol"] = json!("TCP");
    pod["containers"][0]["readinessProbe"]["failureThreshold"] = json!(3);
    pod["volumes"][0]["configMap"]["defaultMode"] = json!(420);
    observed[1]["spec"]["ports"][0]["protocol"] = json!("TCP");
    change(&mut observed);
    let socket = root.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let peer = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line == "stop\n" {
                break;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "vm.exec");
            let input = base64::engine::general_purpose::STANDARD
                .decode(request["params"]["stdin_b64"].as_str().unwrap())
                .unwrap();
            let response = if input.is_empty() {
                json!({"kind":"NodeList","items":(["kube-server","kube-worker"].map(|name|
                    json!({"metadata":{"name":name},"status":{"nodeInfo":{"kubeletVersion":"v1.33.4+k3s1"},
                        "conditions":[{"type":"Ready","status":"True"}]}})))})
            } else {
                let identity: Value = serde_json::from_slice(&input).unwrap();
                observed
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|value| value["kind"] == identity["kind"])
                    .unwrap()
                    .clone()
            };
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":request["id"],
                "result":{"exit":0,"stdout":response.to_string(),"stderr":""}})
            )
            .unwrap();
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["kubernetes", "status", "-C"])
        .arg(&root)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &root)
        .output()
        .unwrap();
    UnixStream::connect(&socket)
        .unwrap()
        .write_all(b"stop\n")
        .unwrap();
    peer.join().unwrap();
    fs::remove_dir_all(root).unwrap();
    output
}

#[test]
fn kubernetes_status_accepts_api_defaults_inside_declared_arrays() {
    let output = status_with_defaulted_workload(|_| {});
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["clusters"][0]["state"], "ready");
    assert_eq!(value["data"]["clusters"][0]["issues"], json!([]));
}

#[test]
fn kubernetes_status_still_reports_array_membership_order_and_declared_field_drift() {
    let changes: [(&str, fn(&mut Value)); 8] = [
        ("extra container", |v| {
            v[0]["spec"]["template"]["spec"]["containers"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"unrequested","image":"other:v1"}));
        }),
        ("missing container", |v| {
            v[0]["spec"]["template"]["spec"]["containers"]
                .as_array_mut()
                .unwrap()
                .pop();
        }),
        ("container order", |v| {
            v[0]["spec"]["template"]["spec"]["containers"]
                .as_array_mut()
                .unwrap()
                .reverse();
        }),
        ("declared image", |v| {
            v[0]["spec"]["template"]["spec"]["containers"][0]["image"] = json!("other:v2");
        }),
        ("missing declared field", |v| {
            v[0]["spec"]["template"]["spec"]["containers"][0]
                .as_object_mut()
                .unwrap()
                .remove("image");
        }),
        ("scalar array order", |v| {
            v[0]["spec"]["template"]["spec"]["containers"][0]["args"] = json!(["8080", "serve"]);
        }),
        ("array type changed", |v| {
            v[0]["spec"]["template"]["spec"]["containers"][0]["args"] = json!(null);
        }),
        ("nested service port", |v| {
            v[1]["spec"]["ports"][0]["targetPort"] = json!(9090);
        }),
    ];
    for (name, change) in changes {
        let output = status_with_defaulted_workload(change);
        assert_eq!(output.status.code(), Some(24), "accepted {name}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["status"], "warn", "{name}");
        let issues = value["data"]["clusters"][0]["issues"].as_array().unwrap();
        assert!(
            issues.iter().any(|issue| issue["code"] == "ResourceDrift"),
            "{name}: {value}"
        );
    }
}
