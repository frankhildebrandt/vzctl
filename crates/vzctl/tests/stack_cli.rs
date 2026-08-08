use serde_json::Value;
use std::fs;
use std::process::Command;

fn vzctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vzctl"))
}

#[test]
fn stack_init_writes_valid_config() {
    let root = std::env::temp_dir().join(format!("vzctl-stack-init-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let stack_dir = root.join("lab");
    let output = vzctl()
        .args(["stack", "init", "--name", "lab", "--format", "json"])
        .arg(&stack_dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["command"], "stack.init");
    assert_eq!(envelope["status"], "ok");

    let config_path = stack_dir.join("hypernetwork.config.yaml");
    assert!(config_path.is_file());

    let validate = vzctl()
        .args(["validate", "-C"])
        .arg(&config_path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(validate.status.success(), "{validate:?}");
}

#[test]
fn stack_init_rejects_existing_config_without_force() {
    let root =
        std::env::temp_dir().join(format!("vzctl-stack-init-existing-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let first = vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(first.status.success(), "{first:?}");

    let second = vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(3));
}

#[test]
fn stack_vm_add_assigns_next_free_ip() {
    let root = std::env::temp_dir().join(format!("vzctl-stack-vm-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .status()
        .unwrap();

    let add = vzctl()
        .args([
            "stack",
            "vm",
            "add",
            "web",
            "-C",
            root.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(add.status.success(), "{add:?}");
    let envelope: Value = serde_json::from_slice(&add.stdout).unwrap();
    assert_eq!(envelope["vm"], "web");
    assert_eq!(envelope["ip"], "10.80.0.10");

    let collision = vzctl()
        .args([
            "stack",
            "vm",
            "add",
            "db",
            "-C",
            root.to_str().unwrap(),
            "--ip",
            "10.80.0.10",
        ])
        .output()
        .unwrap();
    assert_eq!(collision.status.code(), Some(3));
}

#[test]
fn stack_vm_add_rejects_memory_below_supported_minimum() {
    let root = std::env::temp_dir().join(format!("vzctl-stack-vm-memory-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .status()
        .unwrap();

    let config_path = root.join("hypernetwork.config.yaml");
    let before = fs::read_to_string(&config_path).unwrap();
    let add = vzctl()
        .args([
            "stack",
            "vm",
            "add",
            "web",
            "-C",
            root.to_str().unwrap(),
            "--memory",
            "6",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert_eq!(add.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&add.stderr).unwrap();
    assert_eq!(envelope["command"], "stack.vm.add");
    assert!(envelope["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("at least 256 MiB"));
    assert!(envelope["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("6Gi"));
    assert_eq!(fs::read_to_string(config_path).unwrap(), before);
}

#[test]
fn stack_vm_add_accepts_explicit_gib_memory() {
    let root =
        std::env::temp_dir().join(format!("vzctl-stack-vm-memory-gib-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .status()
        .unwrap();

    let add = vzctl()
        .args([
            "stack",
            "vm",
            "add",
            "web",
            "-C",
            root.to_str().unwrap(),
            "--memory",
            "6Gi",
        ])
        .output()
        .unwrap();

    assert!(add.status.success(), "{add:?}");
    let config = fs::read_to_string(root.join("hypernetwork.config.yaml")).unwrap();
    assert!(config.contains("memory: 6Gi"));
}

#[test]
fn stack_net_remove_fails_when_vm_attached() {
    let root = std::env::temp_dir().join(format!("vzctl-stack-net-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .status()
        .unwrap();
    vzctl()
        .args(["stack", "vm", "add", "web", "-C", root.to_str().unwrap()])
        .status()
        .unwrap();

    let remove = vzctl()
        .args([
            "stack",
            "net",
            "remove",
            "lan",
            "-C",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(remove.status.code(), Some(3));
}

#[test]
fn stack_volume_and_mount_round_trip() {
    let root = std::env::temp_dir().join(format!("vzctl-stack-mount-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    vzctl()
        .args(["stack", "init", "--name", "lab"])
        .arg(&root)
        .status()
        .unwrap();
    vzctl()
        .args(["stack", "vm", "add", "web", "-C", root.to_str().unwrap()])
        .status()
        .unwrap();
    fs::create_dir_all(root.join("share")).unwrap();
    vzctl()
        .args([
            "stack",
            "volume",
            "add",
            "app",
            "./share",
            "-C",
            root.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    vzctl()
        .args([
            "stack",
            "mount",
            "add",
            "web",
            "--source",
            "app",
            "--target",
            "/srv/app",
            "-C",
            root.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    let config_path = root.join("hypernetwork.config.yaml");
    let before = fs::read_to_string(&config_path).unwrap();
    assert!(before.contains("source: app"));
    assert!(before.contains("target: /srv/app"));

    vzctl()
        .args([
            "stack",
            "mount",
            "remove",
            "web",
            "--target",
            "/srv/app",
            "-C",
            root.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    let after = fs::read_to_string(&config_path).unwrap();
    assert!(!after.contains("target: /srv/app"));
}
