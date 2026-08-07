use serde_json::Value;
use std::process::Command;

#[test]
fn services_usage_without_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["services"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage: vzctl services"));
}

#[test]
fn services_unknown_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["services", "purge"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown services subcommand"));
}

#[test]
fn services_unknown_target() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["services", "start", "bogus", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown service"));
}

#[test]
fn services_status_json_envelope() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["services", "status", "--format", "json"])
        .output()
        .unwrap();
    if !cfg!(target_os = "macos") {
        assert_eq!(output.status.code(), Some(12));
        return;
    }
    assert!(
        output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["apiVersion"], "vzctl.dev/v1");
    assert_eq!(envelope["command"], "services.status");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["exit_code"], 0);
    assert!(envelope["services"].is_array());
    let services = envelope["services"].as_array().unwrap();
    assert_eq!(services.len(), 3);
    assert_eq!(services[0]["id"], "net");
    assert_eq!(services[1]["id"], "edge");
    assert_eq!(services[2]["id"], "supervisor");
}

#[test]
fn services_help_lists_lifecycle_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["services", "help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("services start"));
    assert!(stderr.contains("services stop"));
    assert!(stderr.contains("services restart"));
}

#[test]
fn top_level_help_lists_services() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .arg("help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("services status"));
    assert!(stdout.contains("services start|stop|restart"));
}
