use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vzctl-cert-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(args)
            .env("VZCTL_STATE_DIR", self.0.join("active"))
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn explicit_upgrade_cli_stages_only_and_refuses_output_reuse_or_force() {
    let fixture = Fixture::new();
    assert!(fixture
        .command(&["certs", "ca", "init", "--format", "json"])
        .status
        .success());
    assert!(fixture
        .command(&["certs", "mint", "auth.svc.cli.vz.test", "--format", "json"])
        .status
        .success());
    let root = fixture.0.join("active/ca/root");
    let key = fs::read(root.join("ca.key")).unwrap();
    let cert = fs::read(root.join("ca.crt")).unwrap();
    let output = fixture.0.join("candidate");
    let args = [
        "certs",
        "ca",
        "upgrade",
        "--output",
        output.to_str().unwrap(),
        "--format",
        "json",
    ];
    let result = fixture.command(&args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let body: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(body["command"], "certs.ca.upgrade");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["data"]["staged"], true);
    assert_eq!(body["data"]["promoted"], false);
    assert_eq!(body["data"]["certificates"], 2);
    assert_eq!(fs::read(root.join("ca.key")).unwrap(), key);
    assert_eq!(fs::read(root.join("ca.crt")).unwrap(), cert);
    assert!(!String::from_utf8_lossy(&result.stdout).contains("PRIVATE KEY"));
    let manifest = fs::read(output.join("manifest.json")).unwrap();
    assert_eq!(
        fs::metadata(output.join("manifest.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let retry = fixture.command(&args);
    assert_eq!(retry.status.code(), Some(10));
    assert_eq!(fs::read(output.join("manifest.json")).unwrap(), manifest);
    assert_eq!(
        fixture
            .command(&["certs", "ca", "upgrade", "--force", "--format", "json"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn upgrade_cli_requires_explicit_output_and_existing_root() {
    let fixture = Fixture::new();
    let help = fixture.command(&["help", "certs"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("ca upgrade --output"));
    let missing_output = fixture.command(&["certs", "ca", "upgrade", "--format", "json"]);
    assert_eq!(missing_output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&missing_output.stdout).unwrap();
    assert_eq!(body["command"], "certs.ca.upgrade");
    assert_eq!(body["exit_code"], 2);
    let output = fixture.0.join("candidate");
    let missing_root = fixture.command(&[
        "certs",
        "ca",
        "upgrade",
        "--output",
        output.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(missing_root.status.code(), Some(10));
    assert!(!output.exists());
}
