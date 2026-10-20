use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn example_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/edge-dmz")
}

fn state_directory() -> PathBuf {
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from("/private/tmp").join(format!(
        "vzctl-edge-dmz-{fixture}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn reference_environment_validates_and_plan_diff_are_read_only() {
    let example = example_directory();
    for path in [
        "hypernetwork.config.yaml",
        "cloud-init/router.yaml",
        "cloud-init/web.yaml",
        "cloud-init/docker.yaml",
        "README.md",
    ] {
        assert!(example.join(path).is_file(), "missing {path}");
    }
    for path in [
        "cloud-init/router.yaml",
        "cloud-init/web.yaml",
        "cloud-init/docker.yaml",
    ] {
        let source = fs::read_to_string(example.join(path)).unwrap();
        let _: Value = serde_yaml::from_str(&source).unwrap();
        for forbidden in ["ssh_authorized_keys:", "password:", "token:"] {
            assert!(
                !source.contains(forbidden),
                "{path} must not contain {forbidden}"
            );
        }
    }

    let validation = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["validate", "-C"])
        .arg(&example)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(validation.status.success(), "{validation:?}");

    let state = state_directory();
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            let result = match request["method"].as_str() {
                Some("stack.inspect") => json!({"resources": [], "journal": null, "lease": null}),
                Some("net.list") => json!({"networks": [], "attachments": []}),
                _ => panic!("plan/diff issued a mutating or unexpected RPC: {request}"),
            };
            writeln!(
                stream,
                "{}",
                json!({
                    "jsonrpc": "2.0",
                    "result": result,
                    "id": request["id"],
                })
            )
            .unwrap();
        }
    });

    for command in ["plan", "diff"] {
        let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .arg(command)
            .args(["-C"])
            .arg(&example)
            .args(["--format", "json"])
            .env("VZCTL_STATE_DIR", &state)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["command"], command);
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
            "port",
            "certs",
            "ingress",
            "oidc",
            "resilience",
            "observability",
        ] {
            assert!(
                actions.iter().any(|action| action["kind"] == kind),
                "missing {kind}"
            );
        }
        assert!(actions.iter().all(|action| action["action"] == "create"));
    }

    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}
