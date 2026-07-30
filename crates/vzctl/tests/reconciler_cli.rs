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
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from("/private/tmp").join(format!("vzrc-{}-{nonce}", std::process::id(),));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn plan_uses_cli_v1_envelope_and_is_read_only() {
    let state = state_directory();
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["method"], "stack.inspect");
        writeln!(
            stream,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "result": {"resources": [], "journal": null, "lease": null},
                "id": 1,
            })
        )
        .unwrap();
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
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["apiVersion"], "vzctl.dev/v1");
    assert_eq!(envelope["command"], "plan");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["summary"]["actions"], 8);
    assert_eq!(envelope["actions"][0]["action"], "create");
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
