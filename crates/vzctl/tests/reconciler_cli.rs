use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validate/valid-full.yaml")
}

fn state_directory() -> PathBuf {
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from("/private/tmp")
        .join(format!("vzrc-{}-{fixture}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn container_plan(actual_vm: Value, with_binding: bool) -> Value {
    workload_plan(
        actual_vm,
        json!({"proof":{"image":"python:3.12-slim"}}),
        with_binding,
    )
}

fn workload_plan(actual_vm: Value, desired_containers: Value, with_binding: bool) -> Value {
    let state = state_directory();
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/validate/valid-docker-containers.yaml"),
    )
    .unwrap();
    let mut config: Value = serde_yaml::from_str(&source).unwrap();
    config["spec"]["vms"]["docker"]["composeFiles"] = json!([]);
    config["spec"]["vms"]["docker"]["containers"] = desired_containers;
    if with_binding {
        fs::write(state.join("admin.txt"), "synthetic-offline-test-password").unwrap();
        config["spec"]["serviceProviders"] = json!({"database":{
            "kind":"postgres","via":"docker","endpoint":"127.0.0.1:5432",
            "advertiseEndpoint":"10.90.0.10:5432",
            "adminUser":"postgres","passwordFile":"admin.txt"
        }});
        config["spec"]["serviceClaims"] = json!({"db":{
            "kind":"postgres","provider":"database","database":"proof",
            "consumers":[{"container":"docker/proof"}]
        }});
    }
    fs::write(
        state.join("hypernetwork.config.yaml"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let listener = UnixListener::bind(state.join("vz.sock")).unwrap();
    let server = thread::spawn(move || {
        for method in ["stack.inspect", "net.list"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], method);
            let result = if method == "stack.inspect" {
                json!({"resources":[{
                    "kind":"vm","name":"docker-containers/docker","state":"running",
                    "labels":{"project":"docker-containers","stack_id":"docker-containers:docker-containers",
                        "spec":serde_json::to_string(&actual_vm).unwrap()}
                }],"journal":null,"lease":null})
            } else {
                json!({"networks":[],"attachments":[]})
            };
            writeln!(stream, "{}", json!({"id":1,"result":result})).unwrap();
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["plan", "-C"])
        .arg(&state)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    server.join().unwrap();
    fs::remove_dir_all(state).unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn bootstrapped_docker_vm() -> Value {
    json!({
        "from":"ubuntu-base","clone":"linked","disk":"40G","cpus":null,"memory":null,
        "networks":[{"name":"lan","ip":"10.90.0.10"},{"name":"containers","ip":"10.95.0.2"}],
        "cloudInit":null,"dependsOn":[],"roles":["docker","router"],"requires":[],
        "ports":[],"mounts":[],"composeFiles":[],"containers":{}
    })
}

#[test]
fn first_container_declaration_updates_workload_without_breaking_vm() {
    let envelope = container_plan(bootstrapped_docker_vm(), false);
    let action = envelope["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["kind"] == "vm")
        .unwrap();
    assert_eq!(action["name"], "docker-containers/docker");
    assert_eq!(action["action"], "update");
    assert_eq!(action["breaking"], false, "{action}");
}

#[test]
fn first_binding_for_a_new_container_is_creation_not_breaking_recreation() {
    let envelope = container_plan(bootstrapped_docker_vm(), true);
    let action = envelope["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["kind"] == "container_binding")
        .unwrap();
    assert_eq!(action["name"], "docker/proof");
    assert_eq!(action["action"], "create", "{action}");
    assert_eq!(action["breaking"], false, "{action}");
    assert!(envelope["actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|action| action["breaking"] == false));
}

#[test]
fn existing_container_image_env_and_volume_changes_remain_breaking() {
    for previous in [
        json!({"image":"python:3.11-slim"}),
        json!({"image":"python:3.12-slim","env":{"MODE":"previous"}}),
        json!({"image":"python:3.12-slim","volumes":["/srv/previous:/data"]}),
    ] {
        let mut actual = bootstrapped_docker_vm();
        actual["containers"] = json!({"proof":previous});
        let envelope = container_plan(actual, false);
        let action = envelope["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["kind"] == "vm")
            .unwrap();
        assert_eq!(action["breaking"], true, "{action}");
    }
}

#[test]
fn adding_a_container_preserves_unchanged_existing_containers_without_vm_recreation() {
    let existing = json!({"redis":{"image":"redis:7-alpine","volumes":["/srv/redis:/data"]}});
    let mut actual = bootstrapped_docker_vm();
    actual["containers"] = existing.clone();
    let mut desired = existing;
    desired["proof"] = json!({"image":"python:3.12-slim"});
    let envelope = workload_plan(actual, desired, true);
    assert!(envelope["actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|action| action["breaking"] == false));
}

#[test]
fn first_binding_for_existing_or_unknown_consumer_still_requires_recreation() {
    let mut declared = bootstrapped_docker_vm();
    declared["containers"] = json!({"proof":{"image":"python:3.12-slim"}});
    let mut compose = bootstrapped_docker_vm();
    compose["composeFiles"] = json!(["previous-compose.yaml"]);
    for actual in [declared, compose, json!({"legacy":"unknown"})] {
        let envelope = container_plan(actual, true);
        let action = envelope["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["kind"] == "container_binding")
            .unwrap();
        assert_eq!(action["action"], "recreate", "{action}");
        assert_eq!(action["breaking"], true, "{action}");
    }
}

#[test]
fn new_container_never_masks_hardware_network_disk_or_boot_changes() {
    for (field, value) in [
        ("disk", json!("80G")),
        ("cpus", json!(4)),
        ("memory", json!("8Gi")),
        ("from", json!("previous-base")),
        ("roles", json!(["docker"])),
        ("cloudInit", json!("previous-cloud-init.yaml")),
        ("composeFiles", json!(["previous-compose.yaml"])),
        ("networks", json!([{"name":"lan","ip":"10.90.0.11"}])),
    ] {
        let mut actual = bootstrapped_docker_vm();
        actual[field] = value;
        let envelope = container_plan(actual, false);
        let action = envelope["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["kind"] == "vm")
            .unwrap();
        assert_eq!(action["breaking"], true, "{field}: {action}");
    }
}

#[test]
fn new_container_does_not_hide_removal_of_an_existing_container() {
    let mut actual = bootstrapped_docker_vm();
    actual["containers"] = json!({"previous":{"image":"redis:7-alpine"}});
    let envelope = container_plan(actual, false);
    let action = envelope["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["kind"] == "vm")
        .unwrap();
    assert_eq!(action["breaking"], true, "{action}");
}

#[test]
fn plan_uses_cli_v1_envelope_and_is_read_only() {
    let state = state_directory();
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        for method in ["stack.inspect", "net.list"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["method"], method);
            let result = if method == "stack.inspect" {
                json!({"resources": [], "journal": null, "lease": null})
            } else {
                json!({"networks": [], "attachments": []})
            };
            writeln!(
                stream,
                "{}",
                json!({
                    "jsonrpc": "2.0",
                    "result": result,
                    "id": 1,
                })
            )
            .unwrap();
        }
    });

    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["plan", "-C"])
        .arg(fixture())
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["apiVersion"], "vzctl.dev/v1");
    assert_eq!(envelope["command"], "plan");
    assert_eq!(envelope["status"], "ok");
    let actions = envelope["actions"].as_array().unwrap();
    assert_eq!(envelope["summary"]["actions"], actions.len());
    for kind in [
        "vm",
        "network",
        "dns",
        "image",
        "route",
        "policy",
        "resilience",
        "observability",
    ] {
        assert!(
            actions.iter().any(|action| action["kind"] == kind),
            "missing {kind}"
        );
    }
    assert_eq!(envelope["actions"][0]["action"], "create");
}

#[test]
fn plan_rejects_foreign_network_overlap_without_mutating_rpc() {
    let state = state_directory();
    let listener = UnixListener::bind(state.join("vz.sock")).unwrap();
    let server = thread::spawn(move || {
        for method in ["stack.inspect", "net.list"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], method);
            let result = if method == "stack.inspect" {
                json!({"resources": [], "journal": null, "lease": null})
            } else {
                json!({"networks":[{"name":"foreign","project":"other","cidr":"10.80.0.0/24"}]})
            };
            writeln!(stream, "{}", json!({"id":1,"result":result})).unwrap();
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["plan", "-C"])
        .arg(fixture())
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    server.join().unwrap();
    fs::remove_dir_all(state).unwrap();
    assert_eq!(output.status.code(), Some(3));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "fail");
    assert!(result["summary"]["message"]
        .as_str()
        .unwrap()
        .contains("overlaps"));
}

#[test]
fn apply_rejects_resume_and_abort_together_with_exit_three() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["apply", "--resume", "--abort", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["command"], "apply");
    assert_eq!(envelope["status"], "fail");
    assert_eq!(envelope["exit_code"], 3);
}
