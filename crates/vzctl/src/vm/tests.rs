use super::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn temp_state() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = PathBuf::from("/private/tmp")
        .join(format!("vzctl-vm-{}-{nonce}-{seq}", std::process::id()));
    fs::create_dir_all(path.join("vms")).unwrap();
    path
}

fn write_bundle(state: &Path, id: &str, roles: &[&str]) {
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

fn write_bundle_with_resources(state: &Path, id: &str, cpus: u32, memory_mib: u64) {
    let bundle = state.join("vms").join(id);
    fs::create_dir_all(&bundle).unwrap();
    fs::write(
        bundle.join("vm.json"),
        serde_json::to_string_pretty(&json!({
            "apiVersion": "vzctl.dev/vm-bundle/v1",
            "managed-by": "vzctl",
            "vm_id": id,
            "roles": [],
            "resources": {
                "cpus": cpus,
                "memory_mib": memory_mib,
            },
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn console_socket_component_matches_daemon_contract() {
    assert_eq!(
        socket_file_component("monitos/monitos-main"),
        "44da7bb0f51beebe"
    );
}

#[test]
fn start_rpc_waits_for_supervisor_lock_handoff_beyond_old_client_deadline() {
    let state = temp_state();
    let socket = state.join("handoff.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&request).unwrap()["method"],
            "vm.start"
        );
        // Deliberately model the supervisor's two bounded lock phases. This
        // response arrives after the old 5s client deadline, not a test race.
        thread::sleep(Duration::from_millis(5500));
        let _ = writeln!(
            stream,
            "{}",
            json!({
                "jsonrpc": "2.0", "id": 1,
                "result": {"vm_id": "lab/ops", "state": "starting", "pid": 123}
            })
        );
    });
    let result = rpc(
        &socket,
        "vm.start",
        json!({"vm_id": "lab/ops", "bundle": "/unused"}),
    );
    server.join().unwrap();
    fs::remove_dir_all(state).unwrap();
    assert_eq!(result.unwrap()["state"], "starting");
}

#[test]
fn parse_lifecycle_commands() {
    let list = parse(
        ["list".into(), "--format".into(), "json".into()].into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(list.operation, Operation::List);
    assert_eq!(list.format, Format::Json);

    let probe = parse(
        [
            "probe".into(),
            "neti/neti-home".into(),
            "--target".into(),
            "main-node.core.neti.vz.test:4222".into(),
            "--via".into(),
            "both".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        probe.operation,
        Operation::Probe {
            id: "neti/neti-home".into(),
            target: "main-node.core.neti.vz.test:4222".into(),
            via: ProbeVia::Both,
            timeout_ms: 5_000,
        }
    );

    let health = parse(["health".into(), "web".into()].into_iter(), false).unwrap();
    assert_eq!(health.operation, Operation::Health { id: "web".into() });
    let stats = parse(["stats".into(), "web".into()].into_iter(), false).unwrap();
    assert_eq!(stats.operation, Operation::Stats { id: "web".into() });

    let start = parse(["start".into(), "web".into()].into_iter(), false).unwrap();
    assert_eq!(start.operation, Operation::Start { id: "web".into() });

    let restart = parse(["restart".into(), "web".into()].into_iter(), false).unwrap();
    assert_eq!(restart.operation, Operation::Restart { id: "web".into() });

    let stop = parse(
        ["stop".into(), "web".into(), "--wait".into(), "false".into()].into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        stop.operation,
        Operation::Stop {
            id: "web".into(),
            wait: false
        }
    );

    let delete = parse(
        ["delete".into(), "web".into(), "--force".into()].into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        delete.operation,
        Operation::Delete {
            id: "web".into(),
            force: true
        }
    );

    let modify = parse(
        [
            "modify".into(),
            "web".into(),
            "--cpus".into(),
            "4".into(),
            "--memory".into(),
            "2G".into(),
            "--format".into(),
            "json".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        modify.operation,
        Operation::Modify {
            id: "web".into(),
            cpus: Some(4),
            memory_mib: Some(2048),
        }
    );
    assert_eq!(modify.format, Format::Json);
}

#[test]
fn tty_exec_env_defaults_portable_term() {
    let empty = BTreeMap::new();
    assert_eq!(
        tty_exec_env(&empty).get("TERM").map(String::as_str),
        Some("xterm-256color")
    );
    let mut explicit = BTreeMap::new();
    explicit.insert("TERM".into(), "vt100".into());
    assert_eq!(
        tty_exec_env(&explicit).get("TERM").map(String::as_str),
        Some("vt100")
    );
}

#[test]
fn parse_agent_upgrade_accepts_vm_id_or_all() {
    let upgrade = parse(
        [
            "agent".into(),
            "upgrade".into(),
            "demo/web".into(),
            "--format".into(),
            "json".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        upgrade.operation,
        Operation::AgentUpgrade {
            all: false,
            id: Some("demo/web".into()),
        }
    );
    let all = parse(
        ["agent".into(), "upgrade".into(), "--all".into()].into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        all.operation,
        Operation::AgentUpgrade {
            all: true,
            id: None,
        }
    );
}

#[test]
fn parse_modify_requires_cpus_or_memory() {
    let err = parse(["modify".into(), "web".into()].into_iter(), false).unwrap_err();
    assert_eq!(err.code, EXIT_USAGE);
}

#[test]
fn modify_patches_manifest_resources() {
    let _guard = env_lock().lock().unwrap();
    let state = temp_state();
    write_bundle_with_resources(&state, "web", 2, 1024);
    std::env::set_var("VZCTL_STATE_DIR", &state);
    let socket = state.join("vz.sock");
    let envelope = modify_vm("web", Some(4), Some(2048), &socket).unwrap();
    assert_eq!(envelope["command"], "vm.modify");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["vm"]["resources"]["cpus"], 4);
    assert_eq!(envelope["vm"]["resources"]["memory_mib"], 2048);
    assert_eq!(envelope["restart_required"], false);
    assert_eq!(envelope["live"], false);

    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(state.join("vms/web/vm.json")).unwrap()).unwrap();
    assert_eq!(manifest["resources"]["cpus"], 4);
    assert_eq!(manifest["resources"]["memory_mib"], 2048);

    let expected: Value =
        serde_json::from_str(include_str!("../../tests/golden/vm-modify.json")).unwrap();
    assert_eq!(envelope["apiVersion"], expected["apiVersion"]);
    assert_eq!(envelope["command"], expected["command"]);
    assert_eq!(envelope["status"], expected["status"]);
    assert_eq!(envelope["exit_code"], expected["exit_code"]);
    assert_eq!(envelope["vm"]["resources"], expected["vm"]["resources"]);
    assert_eq!(envelope["live"], expected["live"]);
    assert_eq!(envelope["restart_required"], expected["restart_required"]);
    std::env::remove_var("VZCTL_STATE_DIR");
}

#[test]
fn list_merges_disk_and_runtime() {
    let _guard = env_lock().lock().unwrap();
    let state = temp_state();
    write_bundle(&state, "web", &[]);
    write_bundle(&state, "db", &["router"]);
    let socket = state.join("vz.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        for expected in ["vm.list", "net.list"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert!(request.contains(expected), "{request} missing {expected}");
            let body = if expected == "vm.list" {
                json!({
                    "jsonrpc": "2.0",
                    "result": [{
                        "vm_id": "web",
                        "state": "running",
                        "pid": 4242,
                        "bundle": "/state/vms/web",
                        "updated_at": "2026-01-01T00:00:00Z",
                    }],
                    "id": 1,
                })
            } else {
                json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "networks": [],
                        "attachments": [{
                            "vm_id": "web",
                            "network": "lan",
                            "ip": "10.70.0.10",
                        }],
                    },
                    "id": 1,
                })
            };
            writeln!(stream, "{body}").unwrap();
        }
    });

    std::env::set_var("VZCTL_STATE_DIR", &state);
    let envelope = list_vms("vm.list", &socket).unwrap();
    std::env::remove_var("VZCTL_STATE_DIR");
    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();

    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["summary"]["vms"], 2);
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
    assert_eq!(
        web["networks"],
        json!([{ "name": "lan", "ip": "10.70.0.10" }])
    );
    let db = envelope["vms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vm| vm["id"] == "db")
        .unwrap();
    assert_eq!(db["state"], "stopped");
    assert_eq!(db["roles"], json!(["router"]));
}

#[test]
fn start_stop_delete_roundtrip_with_mock_supervisor() {
    let _guard = env_lock().lock().unwrap();
    let state = temp_state();
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
                    "id": 1,
                }),
                "vm.stop" => json!({
                    "jsonrpc": "2.0",
                    "result": {"vm_id": "web", "state": "stopped"},
                    "id": 1,
                }),
                "vm.list" => json!({
                    "jsonrpc": "2.0",
                    "result": [],
                    "id": 1,
                }),
                "vm.purge" => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "vm_id": "web",
                        "purged": true,
                        "detached_networks": ["lan"],
                        "ports_removed": 0,
                    },
                    "id": 1,
                }),
                "net.list" => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "networks": [],
                        "attachments": [{
                            "vm_id": "web",
                            "network": "lan",
                        }],
                    },
                    "id": 1,
                }),
                "net.detach" => json!({
                    "jsonrpc": "2.0",
                    "result": {"vm_id": "web", "network": "lan"},
                    "id": 1,
                }),
                other => panic!("unexpected method {other}"),
            };
            writeln!(stream, "{body}").unwrap();
        };

        // start
        let (mut stream, _) = listener.accept().unwrap();
        respond(&mut stream);
        // stop + wait list
        let (mut stream, _) = listener.accept().unwrap();
        respond(&mut stream);
        let (mut stream, _) = listener.accept().unwrap();
        respond(&mut stream);
        // delete: vm.purge
        let (mut stream, _) = listener.accept().unwrap();
        respond(&mut stream);
    });

    std::env::set_var("VZCTL_STATE_DIR", &state);
    let started = start_vm("web", &socket).unwrap();
    assert_eq!(started["vm"]["state"], "starting");
    assert_eq!(started["vm"]["pid"], 99);

    let stopped = stop_vm("web", true, &socket).unwrap();
    assert_eq!(stopped["vm"]["state"], "stopped");

    let deleted = delete_vm("web", false, &socket).unwrap();
    assert_eq!(deleted["vm"]["deleted"], true);
    assert_eq!(deleted["vm"]["detached_networks"], json!(["lan"]));
    assert_eq!(deleted["vm"]["purged_runtime"], true);
    assert!(!state.join("vms/web").exists());
    std::env::remove_var("VZCTL_STATE_DIR");
    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}

#[test]
fn force_delete_cleans_orphan_without_bundle() {
    let _guard = env_lock().lock().unwrap();
    let state = temp_state();
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
                "vm.purge" => json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "vm_id": "mos",
                        "purged": true,
                        "detached_networks": ["lan"],
                        "ports_removed": 1,
                    },
                    "id": 1,
                }),
                other => panic!("unexpected method {other}"),
            };
            writeln!(stream, "{body}").unwrap();
        };
        let (mut stream, _) = listener.accept().unwrap();
        respond(&mut stream);
    });

    std::env::set_var("VZCTL_STATE_DIR", &state);
    assert!(!state.join("vms/mos").exists());
    let missing = delete_vm("mos", false, &socket).unwrap_err();
    assert_eq!(missing.code, EXIT_INVALID);

    let deleted = delete_vm("mos", true, &socket).unwrap();
    assert_eq!(deleted["vm"]["deleted"], true);
    assert_eq!(deleted["vm"]["purged_bundle"], false);
    assert_eq!(deleted["vm"]["purged_runtime"], true);
    assert_eq!(deleted["vm"]["detached_networks"], json!(["lan"]));
    assert_eq!(deleted["vm"]["ports_removed"], 1);
    std::env::remove_var("VZCTL_STATE_DIR");
    server.join().unwrap();
    fs::remove_dir_all(&state).unwrap();
}

#[test]
fn console_detach_sequence_is_ctrl_p_ctrl_q() {
    let mut state = ConsoleDetachState::default();
    let mut forward = Vec::new();
    assert!(!consume_console_stdin(&mut state, b"hi", &mut forward));
    assert_eq!(forward, b"hi");
    assert!(!state.saw_ctrl_p);

    forward.clear();
    assert!(!consume_console_stdin(
        &mut state,
        &[CONSOLE_CTRL_P],
        &mut forward
    ));
    assert!(forward.is_empty());
    assert!(state.saw_ctrl_p);

    assert!(consume_console_stdin(
        &mut state,
        &[CONSOLE_CTRL_Q],
        &mut forward
    ));
    assert!(forward.is_empty());

    // Literal Ctrl-P: press it twice.
    state = ConsoleDetachState::default();
    forward.clear();
    assert!(!consume_console_stdin(
        &mut state,
        &[CONSOLE_CTRL_P, CONSOLE_CTRL_P, b'x'],
        &mut forward
    ));
    assert_eq!(forward, &[CONSOLE_CTRL_P, b'x']);
    assert!(!state.saw_ctrl_p);

    // Ctrl-C stays a guest keystroke.
    state = ConsoleDetachState::default();
    forward.clear();
    assert!(!consume_console_stdin(&mut state, &[0x03], &mut forward));
    assert_eq!(forward, &[0x03]);
}

#[test]
fn exec_parses_interactive_tty_flags() {
    let options = parse(
        [
            "exec".into(),
            "web".into(),
            "-it".into(),
            "--".into(),
            "bash".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        options.operation,
        Operation::Exec {
            id: "web".into(),
            cmd: vec!["bash".into()],
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: 30_000,
            interactive: true,
            tty: true,
        }
    );
    let err = parse(
        ["exec".into(), "web".into(), "-i".into(), "bash".into()].into_iter(),
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, EXIT_INVALID);
}

#[test]
fn parse_logs_and_redact() {
    let options = parse(
        [
            "logs".into(),
            "web".into(),
            "--tail".into(),
            "10".into(),
            "--format".into(),
            "json".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert_eq!(
        options.operation,
        Operation::Logs {
            id: "web".into(),
            follow: false,
            tail: 10,
            source: "serial".into(),
            list_sources: false,
            q: None,
            min_level: None,
            group_field: None,
            group_value: None,
            filters: vec![],
            restart: false,
        }
    );
    assert_eq!(options.format, Format::Json);

    let follow = parse(
        ["logs".into(), "web".into(), "-f".into()].into_iter(),
        false,
    )
    .unwrap();
    assert!(matches!(
        follow.operation,
        Operation::Logs { follow: true, .. }
    ));

    let err = parse(
        [
            "logs".into(),
            "web".into(),
            "-f".into(),
            "--format".into(),
            "json".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, EXIT_INVALID);

    let filtered = parse(
        [
            "logs".into(),
            "web".into(),
            "--source".into(),
            "app".into(),
            "--q".into(),
            "error".into(),
            "--min-level".into(),
            "warn".into(),
            "--filter".into(),
            "msg=fail".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert!(matches!(
        filtered.operation,
        Operation::Logs {
            source: ref name,
            q: Some(ref query),
            ..
        } if name == "app" && query == "error"
    ));

    let serial_filter = parse(
        ["logs".into(), "web".into(), "--q".into(), "error".into()].into_iter(),
        false,
    )
    .unwrap_err();
    assert_eq!(serial_filter.code, EXIT_INVALID);

    let listed = parse(
        ["logs".into(), "web".into(), "--list-sources".into()].into_iter(),
        false,
    )
    .unwrap();
    assert!(matches!(
        listed.operation,
        Operation::Logs {
            list_sources: true,
            ..
        }
    ));

    let restart = parse(
        [
            "logs".into(),
            "web".into(),
            "--source".into(),
            "app".into(),
            "--restart".into(),
        ]
        .into_iter(),
        false,
    )
    .unwrap();
    assert!(matches!(
        restart.operation,
        Operation::Logs {
            restart: true,
            source: ref name,
            ..
        } if name == "app"
    ));

    let serial_restart = parse(
        ["logs".into(), "web".into(), "--restart".into()].into_iter(),
        false,
    )
    .unwrap_err();
    assert_eq!(serial_restart.code, EXIT_INVALID);

    assert_eq!(redact_log_line("boot ok"), ("boot ok".into(), false));
    assert_eq!(
        redact_log_line("root_password: secret"),
        ("[redacted]".into(), true)
    );
    assert_eq!(
        redact_log_line("chpasswd: list"),
        ("[redacted]".into(), true)
    );
    assert!(iwatch_logs_path(
        false,
        50,
        Some("error"),
        Some("warn"),
        None,
        None,
        &[("msg".into(), "fail".into())],
    )
    .starts_with("/api/logs?limit=50&q=error&minLevel=warn&filter.msg=fail"));
    assert!(iwatch_logs_path(true, 400, None, None, None, None, &[])
        .starts_with("/api/logs/sse?tail=400"));
}

#[test]
fn logs_reads_serial_tail() {
    let _guard = env_lock().lock().unwrap();
    let state = temp_state();
    let logs = state.join("logs");
    fs::create_dir_all(&logs).unwrap();
    write_bundle(&state, "web", &[]);
    let path = logs.join(format!("{}.serial.log", state_file_component("web")));
    fs::write(&path, "line1\npassword=secret\nline3\nline4\n").unwrap();

    std::env::set_var("VZCTL_STATE_DIR", &state);
    std::env::set_var("VZCTL_LOGS_DIR", &logs);
    let envelope = logs_vm(
        "web",
        false,
        3,
        "serial",
        false,
        None,
        None,
        None,
        None,
        &[],
        false,
        Path::new("/tmp/vzctl-missing.sock"),
    )
    .unwrap();
    std::env::remove_var("VZCTL_STATE_DIR");
    std::env::remove_var("VZCTL_LOGS_DIR");
    fs::remove_dir_all(&state).unwrap();

    assert_eq!(envelope["command"], "vm.logs");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["lines"], json!(["[redacted]", "line3", "line4"]));
    assert_eq!(envelope["summary"]["redacted"], 1);
    assert_eq!(envelope["log"]["path"], path.display().to_string());
}
