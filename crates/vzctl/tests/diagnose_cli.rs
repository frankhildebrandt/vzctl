use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn diagnose_is_private_bounded_explicit_and_does_not_overwrite() {
    let dir = std::env::temp_dir().join(format!(
        "vz-diag-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("edge.error.log"), "SECRET-RAW-LOG-CONTENT").unwrap();
    let target = dir.join("report.json");
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["diagnose", "--output"])
        .arg(&target)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "warn");
    assert_eq!(result["exit_code"], 0);
    let contents = fs::read(&target).unwrap();
    assert!(contents.len() <= 64 * 1024);
    let bundle: Value = serde_json::from_slice(&contents).unwrap();
    assert_eq!(bundle["apiVersion"], "vzctl.dev/diagnose/v1");
    assert_eq!(bundle["collection"]["raw_logs"], false);
    assert!(!String::from_utf8_lossy(&contents).contains("SECRET-RAW-LOG-CONTENT"));
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let overwrite = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["diagnose", "--output"])
        .arg(&target)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &dir)
        .output()
        .unwrap();
    assert_eq!(overwrite.status.code(), Some(3));
    assert_eq!(fs::read(&target).unwrap(), contents);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn diagnose_can_include_explicit_stack_status_with_safe_failure_codes() {
    let dir = std::env::temp_dir().join(format!("vz-diag-stack-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("report.json");
    let config = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate/valid-kubernetes.yaml");
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["diagnose", "-C"])
        .arg(config)
        .args(["--output"])
        .arg(&target)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", &dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let bytes = fs::read(&target).unwrap();
    let bundle: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bundle["controllers"]["project"], "kube-lab");
    assert_eq!(bundle["controllers"]["supervisor"]["state"], "Unavailable");
    assert_eq!(
        bundle["kubernetes"]["clusters"][0]["issues"][0]["code"],
        "NodeInventoryUnavailable"
    );
    assert_eq!(bundle["kubernetes"]["clusters"][0]["state"], "degraded");
    assert_eq!(bundle["collection"]["stack_requested"], true);
    assert!(bytes.len() <= 65536);
    assert!(started.elapsed().as_secs() < 20);
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(dir).unwrap();
}
