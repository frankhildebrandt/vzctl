use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn state_directory() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = PathBuf::from("/private/tmp")
        .join(format!("vzctl-vm-cli-{}-{nonce}-{seq}", std::process::id()));
    fs::create_dir_all(path.join("vms")).unwrap();
    path
}

fn write_bundle(state: &PathBuf, id: &str, roles: &[&str]) {
    let bundle = state.join("vms").join(id);
    fs::create_dir_all(&bundle).unwrap();
    fs::write(
        bundle.join("vm.json"),
        serde_json::to_string_pretty(&json!({
            "apiVersion": "vzctl.dev/vm-bundle/v1",
            "managed-by": "vzctl",
            "vm_id": id,
            "roles": roles,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn vm_list_and_ps_use_cli_v1_envelope() {
    let state = state_directory();
    write_bundle(&state, "web", &[]);
    write_bundle(&state, "db", &["router"]);
    let socket = state.join("vz.sock");
    let web_bundle = state.join("vms/web").display().to_string();
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            let body = match request["method"].as_str() {
                Some("vm.list") => json!({
                    "jsonrpc": "2.0",
                    "result": [{
                        "vm_id": "web",
                        "state": "running",
                        "pid": 4242,
                        "bundle": web_bundle,
                        "updated_at": "2026-01-01T00:00:00Z",
                    }],
                    "id": request["id"],
                }),
                Some("net.list") => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "networks": [{"name": "lan"}],
                        "attachments": [{
                            "vm_id": "web",
                            "network": "lan",
                            "ip": "10.70.0.10",
                        }],
                    },
                    "id": request["id"],
                }),
                other => panic!("unexpected method {other:?}"),
            };
            writeln!(stream, "{body}").unwrap();
        }
    });

    for (args, command, expected_vms) in [
        (vec!["vm", "list", "--format", "json"], "vm.list", 2usize),
        (vec!["ps", "--format", "json"], "ps", 2usize),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(&args)
            .env("VZCTL_STATE_DIR", &state)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["apiVersion"], "vzctl.dev/v1");
        assert_eq!(envelope["command"], command);
        assert_eq!(envelope["status"], "ok");
        assert_eq!(envelope["exit_code"], 0);
        assert_eq!(envelope["summary"]["vms"], expected_vms);
        assert_eq!(envelope["summary"]["running"], 1);
        let web = envelope["vms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vm| vm["id"] == "web")
            .unwrap();
        assert_eq!(web["state"], "running");
        assert_eq!(web["pid"], 4242);
        assert_eq!(web["ips"], json!(["10.70.0.10"]));
    }

    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}

#[test]
fn vm_start_stop_delete_roundtrip() {
    let state = state_directory();
    write_bundle(&state, "web", &[]);
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let respond = |stream: &mut std::os::unix::net::UnixStream| {
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            let method = request["method"].as_str().unwrap();
            let body = match method {
                "vm.start" => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "vm_id": "web",
                        "state": "starting",
                        "pid": 99,
                        "bundle": request["params"]["bundle"],
                    },
                    "id": request["id"],
                }),
                "vm.stop" => json!({
                    "jsonrpc": "2.0",
                    "result": {"vm_id": "web", "state": "stopped"},
                    "id": request["id"],
                }),
                "vm.list" => json!({
                    "jsonrpc": "2.0",
                    "result": [],
                    "id": request["id"],
                }),
                "net.list" => json!({
                    "jsonrpc": "2.0",
                    "result": {"networks": [], "attachments": []},
                    "id": request["id"],
                }),
                other => panic!("unexpected method {other}"),
            };
            writeln!(stream, "{body}").unwrap();
        };

        for _ in 0..6 {
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream);
        }
    });

    let start = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "start", "web", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(start.status.success(), "{start:?}");
    let start_env: Value = serde_json::from_slice(&start.stdout).unwrap();
    assert_eq!(start_env["command"], "vm.start");
    assert_eq!(start_env["vm"]["state"], "starting");

    let stop = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "stop", "web", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(stop.status.success(), "{stop:?}");
    let stop_env: Value = serde_json::from_slice(&stop.stdout).unwrap();
    assert_eq!(stop_env["command"], "vm.stop");
    assert_eq!(stop_env["vm"]["state"], "stopped");

    let delete = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "delete", "web", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(delete.status.success(), "{delete:?}");
    let delete_env: Value = serde_json::from_slice(&delete.stdout).unwrap();
    assert_eq!(delete_env["command"], "vm.delete");
    assert_eq!(delete_env["vm"]["deleted"], true);
    assert!(!state.join("vms/web").exists());

    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}

#[test]
fn vm_start_missing_bundle_exits_three() {
    let state = state_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "start", "missing", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["command"], "vm.start");
    assert_eq!(envelope["status"], "fail");
    assert_eq!(envelope["exit_code"], 3);
    fs::remove_dir_all(&state).unwrap();
}

#[test]
fn vm_exec_inspect_services_and_guest_ps() {
    let state = state_directory();
    write_bundle(&state, "web", &[]);
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        for _ in 0..8 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            let body = match request["method"].as_str() {
                Some("vm.exec") => {
                    let cmd0 = request["params"]["cmd"][0].as_str().unwrap_or("");
                    let stdout = if cmd0 == "ps" {
                        "1 root 0.0 0.1 /sbin/init\n"
                    } else if cmd0 == "systemctl" {
                        "ssh.service loaded active running OpenBSD Secure Shell server\n"
                    } else {
                        "hello\n"
                    };
                    json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "exit": 0,
                            "stdout": stdout,
                            "stderr": "",
                            "truncated": false,
                        },
                        "id": request["id"],
                    })
                }
                Some("vm.list") => json!({
                    "jsonrpc": "2.0",
                    "result": [{
                        "vm_id": "web",
                        "state": "running",
                        "pid": 42,
                        "bundle": "/state/vms/web",
                    }],
                    "id": request["id"],
                }),
                Some("net.list") => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "networks": [],
                        "attachments": [{
                            "vm_id": "web",
                            "network": "lan",
                            "ip": "10.70.0.10",
                        }],
                    },
                    "id": request["id"],
                }),
                Some("vm.agent.health") => json!({
                    "jsonrpc": "2.0",
                    "result": {"status": "ok"},
                    "id": request["id"],
                }),
                Some("vm.agent.version") => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "v": 1,
                        "agent_version": "0.1.0",
                        "capabilities": ["exec"],
                    },
                    "id": request["id"],
                }),
                Some("vm.agent.report_ip") => json!({
                    "jsonrpc": "2.0",
                    "result": {"interfaces": []},
                    "id": request["id"],
                }),
                other => panic!("unexpected method {other:?}"),
            };
            writeln!(stream, "{body}").unwrap();
        }
    });

    let exec = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args([
            "vm", "exec", "web", "--format", "json", "--", "echo", "hello",
        ])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(exec.status.success(), "{exec:?}");
    let exec_env: Value = serde_json::from_slice(&exec.stdout).unwrap();
    assert_eq!(exec_env["command"], "vm.exec");
    assert_eq!(exec_env["exec"]["stdout"], "hello\n");

    let inspect = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "inspect", "web", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(inspect.status.success(), "{inspect:?}");
    let inspect_env: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspect_env["command"], "vm.inspect");
    assert_eq!(inspect_env["networks"][0]["ip"], "10.70.0.10");
    assert_eq!(inspect_env["agent"]["state"], "ready");

    let services = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "services", "web", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(services.status.success(), "{services:?}");
    let services_env: Value = serde_json::from_slice(&services.stdout).unwrap();
    assert_eq!(services_env["command"], "vm.services");

    let guest_ps = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["vm", "ps", "web", "--format", "json"])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(guest_ps.status.success(), "{guest_ps:?}");
    let ps_env: Value = serde_json::from_slice(&guest_ps.stdout).unwrap();
    assert_eq!(ps_env["command"], "vm.ps");
    assert_eq!(ps_env["summary"]["processes"], 1);

    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}

#[test]
fn vm_transfer_push_and_pull() {
    let state = state_directory();
    write_bundle(&state, "web", &[]);
    let host_src = state.join("hello.txt");
    fs::write(&host_src, b"hello-guest").unwrap();
    let host_dst = state.join("pulled.txt");
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["method"], "vm.exec");
            let cmd0 = request["params"]["cmd"][0].as_str().unwrap();
            let body = if cmd0 == "tee" {
                assert!(request["params"]["stdin_b64"].as_str().is_some());
                json!({
                    "jsonrpc": "2.0",
                    "result": {"exit": 0, "stdout": "", "stderr": "", "truncated": false},
                    "id": request["id"],
                })
            } else {
                assert_eq!(cmd0, "base64");
                json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "exit": 0,
                        "stdout": "aGVsbG8tZ3Vlc3Q=",
                        "stderr": "",
                        "truncated": false,
                    },
                    "id": request["id"],
                })
            };
            writeln!(stream, "{body}").unwrap();
        }
    });

    let push = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args([
            "vm",
            "transfer",
            "web",
            host_src.to_str().unwrap(),
            "web:/tmp/hello.txt",
            "--format",
            "json",
        ])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(push.status.success(), "{push:?}");
    let push_env: Value = serde_json::from_slice(&push.stdout).unwrap();
    assert_eq!(push_env["command"], "vm.transfer");
    assert_eq!(push_env["transfer"]["direction"], "push");
    assert_eq!(push_env["transfer"]["bytes"], 11);

    let pull = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args([
            "vm",
            "transfer",
            "web",
            "web:/tmp/hello.txt",
            host_dst.to_str().unwrap(),
            "--format",
            "json",
        ])
        .env("VZCTL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(pull.status.success(), "{pull:?}");
    assert_eq!(fs::read_to_string(&host_dst).unwrap(), "hello-guest");

    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}
