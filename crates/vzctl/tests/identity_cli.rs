use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct State(PathBuf);
impl State {
    fn new() -> Self {
        Self::at(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
    }

    fn at(timestamp: u128) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vz-identity-{}-{}-{}",
            std::process::id(),
            timestamp,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // A clock tick is not a unique identity across parallel tests. Also
        // refuse an existing directory instead of silently sharing its lifetime.
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn command(&self, args: &[&str], name: &str, exit: i32) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(args)
            .args(["--format", "json"])
            .env("VZCTL_STATE_DIR", &self.0)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        let value: Value =
            serde_json::from_slice(&output.stdout).expect("one JSON document on stdout");
        assert_eq!(value["apiVersion"], "vzctl.dev/v1");
        assert_eq!(value["command"], name);
        assert_eq!(value["exit_code"], exit);
        assert_eq!(value["status"], if exit == 0 { "ok" } else { "fail" });
        assert!(value["summary"].is_object());
        assert_eq!(value["ok"], exit == 0); // Additive compatibility with the earlier UI contract.
        if exit == 0 {
            assert!(value["data"].is_object());
            assert!(output.stderr.is_empty());
        } else {
            assert!(!output.stderr.is_empty());
        }
        value
    }
}
impl Drop for State {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn simultaneous_fixture_timestamps_never_share_or_delete_another_state() {
    let first = State::at(42);
    let second = State::at(42);
    let marker = second.0.join("owned-by-second");
    fs::write(&marker, "preserved").unwrap();
    let distinct = first.0 != second.0;
    drop(first);
    assert!(distinct, "parallel tests shared a state directory");
    assert_eq!(fs::read_to_string(marker).unwrap(), "preserved");
}

#[test]
fn local_ca_success_and_error_obey_cli_envelope_without_host_trust_changes() {
    let state = State::new();
    state.command(&["certs", "fingerprint"], "certs.fingerprint", 3);
    state.command(&["certs", "ca", "init"], "certs.ca.init", 0);
    state.command(&["certs", "fingerprint"], "certs.fingerprint", 0);
    state.command(&["certs", "mint", "api.svc.demo.vz.test"], "certs.mint", 0);
    state.command(&["certs", "ca", "init", "--unknown"], "certs.ca.init", 2);
}

#[test]
fn oidc_status_clients_and_invalid_configuration_obey_cli_envelope() {
    let state = State::new();
    let status = state.command(&["oidc", "status"], "oidc.status", 0);
    assert_eq!(status["data"]["running"], false);
    state.command(&["oidc", "clients", "--project", "demo"], "oidc.clients", 0);
    state.command(&["oidc", "clients"], "oidc.clients", 2);
    state.command(&["oidc", "status", "--unknown"], "oidc.status", 2);
    fs::create_dir_all(state.0.join("config")).unwrap();
    fs::write(state.0.join("config/oidc-uplink.yaml"), "[invalid yaml").unwrap();
    state.command(&["oidc", "status"], "oidc.status", 10);
}

#[test]
fn docker_json_errors_work_before_a_context_or_engine_exists() {
    let state = State::new();
    for (args, command, code) in [
        (vec!["docker", "ps", "--unknown"], "docker", 2),
        (vec!["docker", "ps"], "docker.ps", 3),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(args)
            .args(["--format", "json"])
            .env("VZCTL_STATE_DIR", &state.0)
            .current_dir(&state.0)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["command"], command);
        assert_eq!(envelope["status"], "fail");
        assert_eq!(envelope["exit_code"], code);
        assert!(!output.stderr.is_empty());
    }
}
