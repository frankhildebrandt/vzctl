use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn controller_status_reports_pending_from_private_config_without_a_daemon() {
    let root = std::env::temp_dir().join(format!(
        "vzctl-controller-status-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config: Value =
        serde_yaml::from_str(include_str!("fixtures/validate/valid-kubernetes.yaml")).unwrap();
    config["spec"].as_object_mut().unwrap().remove("kubernetes");
    config["spec"]["serviceProviders"] = json!({
        "database":{"kind":"postgres","via":"kube-server","endpoint":"127.0.0.1:5432",
            "adminUser":"postgres","passwordFile":"admin"}
    });
    config["spec"]["serviceClaims"] = json!({"app-db":{
        "kind":"postgres","provider":"database","database":"app"
    }});
    fs::write(
        root.join("hypernetwork.config.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();
    fs::write(root.join("admin"), "PRIVATE-ADMIN-DO-NOT-PRINT").unwrap();
    let state = root.join("state");
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["controller", "status", "-C"])
        .arg(&root)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(24),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "controller.status");
    assert_eq!(result["status"], "warn");
    assert_eq!(result["data"]["claims"][0]["name"], "app-db");
    assert_eq!(result["data"]["claims"][0]["state"], "Pending");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE-ADMIN-DO-NOT-PRINT"));
    assert!(
        !state.exists(),
        "read-only status must not create runtime state or connect to a daemon"
    );
    fs::remove_dir_all(root).unwrap();
}
