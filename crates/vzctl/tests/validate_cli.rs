use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate")
        .join(name)
}

#[test]
fn validate_cli_accepts_positive_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["validate", "-C"])
        .arg(fixture("valid-full.yaml"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["apiVersion"], "vzctl.dev/v1");
    assert_eq!(envelope["command"], "validate");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["exit_code"], 0);
}

#[test]
fn validate_cli_reports_json_paths_for_negative_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["validate", "-C"])
        .arg(fixture("invalid-references.yaml"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["status"], "fail");
    assert_eq!(envelope["exit_code"], 3);
    let errors = envelope["errors"].as_array().unwrap();
    assert!(errors
        .iter()
        .any(|error| error["path"] == "$.spec.routes[0].via"));
    assert!(errors
        .iter()
        .any(|error| error["path"] == "$.spec.vms.web.networks[0].ip"));
}

#[test]
fn validate_cli_reports_schema_paths_for_structural_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["validate", "-C"])
        .arg(fixture("invalid-schema.yaml"))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let errors = envelope["errors"].as_array().unwrap();
    assert!(errors
        .iter()
        .any(|error| error["path"] == "$.metadata.name"));
    assert!(errors.iter().any(|error| error["path"] == "$.spec.routes"));
}

#[test]
fn validate_cli_exports_json_schema() {
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["validate", "--schema"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let schema: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        schema["$id"],
        "https://vzctl.dev/schemas/hypernetwork-v1.schema.json"
    );
    assert!(schema["definitions"]["VmConfig"].is_object());
}
