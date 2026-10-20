use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct State(PathBuf);

impl State {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from("/private/tmp")
            .join(format!("vzce-{}-{nonce}-{sequence}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(args)
            .env("VZCTL_STATE_DIR", &self.0)
            .env("HOME", &self.0)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }

    fn assert_failure(&self, args: &[&str], command: &str, code: i32) {
        let output = self.run(args);
        assert_envelope(&output, command, "fail", code);
        assert!(
            serde_json::from_slice::<Value>(&output.stderr).is_err(),
            "{args:?}: stderr must not be a JSON data channel"
        );
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_envelope(output: &Output, command: &str, status: &str, code: i32) -> Value {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("{command}: expected one stdout JSON document: {error}; {output:?}")
    });
    assert_eq!(value["apiVersion"], "vzctl.dev/v1", "{value}");
    assert_eq!(value["command"], command, "{value}");
    assert_eq!(value["status"], status, "{value}");
    assert_eq!(value["exit_code"], code, "{value}");
    assert!(value["summary"].is_object(), "{value}");
    value
}

#[test]
fn json_parse_errors_work_before_and_after_format_without_side_effects() {
    let state = State::new();
    let cases = [
        ("version", "version", 2),
        ("doctor", "doctor", 3),
        ("stack init", "stack.init", 2),
        ("stack vm add", "stack.vm.add", 2),
        ("stack vm remove", "stack.vm.remove", 2),
        ("stack net add", "stack.net.add", 2),
        ("stack net remove", "stack.net.remove", 2),
        ("stack volume add", "stack.volume.add", 2),
        ("stack volume remove", "stack.volume.remove", 2),
        ("stack mount add", "stack.mount.add", 2),
        ("stack mount remove", "stack.mount.remove", 2),
        ("stack status", "stack.status", 2),
        ("status", "stack.status", 2),
        ("route plan", "route.plan", 2),
        ("route apply", "route.apply", 2),
        ("route status", "route.status", 2),
        ("port list", "port.list", 2),
        ("services status", "services.status", 2),
        ("services start", "services.start", 2),
        ("services stop", "services.stop", 2),
        ("services restart", "services.restart", 2),
        ("image rebake", "image.rebake", 2),
    ];
    for (input, command, code) in cases {
        for suffix in [
            ["--audit-unknown", "--format", "json"],
            ["--format", "json", "--audit-unknown"],
        ] {
            let mut args: Vec<_> = input.split_whitespace().collect();
            args.extend(suffix);
            state.assert_failure(&args, command, code);
        }
    }
    assert_eq!(fs::read_dir(&state.0).unwrap().count(), 0);
}

#[test]
fn namespace_and_service_target_errors_keep_usage_codes() {
    let state = State::new();
    for namespace in [
        "image",
        "port",
        "services",
        "route",
        "stack",
        "vm",
        "net",
        "dns",
        "debug",
        "certs",
        "oidc",
        "controller",
        "kubernetes",
    ] {
        state.assert_failure(&[namespace, "unknown", "--format", "json"], namespace, 2);
        state.assert_failure(&[namespace, "--format", "json"], namespace, 2);
    }
    state.assert_failure(&["unknown", "--format", "json"], "unknown", 2);
    for verb in ["start", "stop", "restart"] {
        state.assert_failure(
            &["services", verb, "unknown-target", "--format", "json"],
            &format!("services.{verb}"),
            3,
        );
    }
}

#[test]
fn stack_and_route_runtime_failures_obey_the_same_contract() {
    let state = State::new();
    state.assert_failure(
        &["stack", "vm", "remove", "missing", "--format", "json"],
        "stack.vm.remove",
        3,
    );
    fs::write(state.0.join("hypernetwork.config.yaml"), "invalid: [yaml").unwrap();
    state.assert_failure(
        &["stack", "vm", "remove", "missing", "--format", "json"],
        "stack.vm.remove",
        3,
    );
    state.assert_failure(&["route", "plan", "--format", "json"], "route.plan", 3);
    state.assert_failure(&["route", "status", "--format", "json"], "route.status", 10);
}

#[test]
fn port_list_and_route_rpc_errors_use_stdout_data_only() {
    let state = State::new();
    state.assert_failure(&["port", "list", "--format", "json"], "port.list", 10);
    for (args, method, reply, code) in [
        (
            vec!["port", "list", "--format", "json"],
            "port.list",
            json!({"result":{"ports":[{"bind":"127.0.0.1","host_port":8080}]}}),
            0,
        ),
        (
            vec!["port", "list", "--format", "json"],
            "port.list",
            json!({"error":{"code":10,"message":"fixture failure"}}),
            10,
        ),
        (
            vec!["route", "status", "--format", "json"],
            "route.status",
            json!({"error":{"code":18,"message":"fixture failure"}}),
            18,
        ),
    ] {
        let socket = state.0.join("vz.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["method"], method);
            writeln!(stream, "{reply}").unwrap();
        });
        let output = state.run(&args);
        server.join().unwrap();
        fs::remove_file(socket).unwrap();
        let value = assert_envelope(&output, method, if code == 0 { "ok" } else { "fail" }, code);
        if code == 0 {
            assert_eq!(value["summary"]["count"], 1);
            assert!(output.stderr.is_empty());
        } else {
            assert!(serde_json::from_slice::<Value>(&output.stderr).is_err());
        }
    }
}

#[test]
fn rebake_missing_input_keeps_its_own_command_name() {
    let state = State::new();
    let output = state.run(&[
        "image",
        "rebake",
        "does-not-exist",
        "--tag",
        "contract",
        "--format",
        "json",
    ]);
    let code = output.status.code().unwrap();
    assert_ne!(code, 0);
    assert_envelope(&output, "image.rebake", "fail", code);
}

#[test]
fn unsupported_identity_verbs_remain_explicit_failures() {
    let state = State::new();
    for (args, command) in [
        (
            vec!["certs", "rollout", "--format", "json"],
            "certs.rollout",
        ),
        (vec!["certs", "verify", "--format", "json"], "certs.verify"),
        (vec!["oidc", "token", "--format", "json"], "oidc.token"),
    ] {
        state.assert_failure(&args, command, 2);
    }
    assert_eq!(fs::read_dir(&state.0).unwrap().count(), 0);
}

#[test]
fn controller_status_distinguishes_ready_and_supervisor_warning_without_writes() {
    let root = State::new();
    let mut config: Value =
        serde_yaml::from_str(include_str!("fixtures/validate/valid-kubernetes.yaml")).unwrap();
    config["spec"].as_object_mut().unwrap().remove("kubernetes");
    fs::write(
        root.0.join("hypernetwork.config.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();
    let runtime = root.0.join("runtime");
    let query = || {
        Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args(["controller", "status", "--format", "json", "-C"])
            .arg(&root.0)
            .env("VZCTL_STATE_DIR", &runtime)
            .env("HOME", &root.0)
            .output()
            .unwrap()
    };
    let value = assert_envelope(&query(), "controller.status", "warn", 24);
    assert_eq!(
        value["data"]["warnings"][0]["code"],
        "SupervisorUnavailable"
    );
    assert!(!runtime.exists());
    fs::create_dir(&runtime).unwrap();
    let listener = UnixListener::bind(runtime.join("vz.sock")).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["method"], "controller.registration");
        writeln!(stream, "{}", json!({"result":{"enabled":false}})).unwrap();
    });
    let output = query();
    server.join().unwrap();
    let value = assert_envelope(&output, "controller.status", "ok", 0);
    assert_eq!(value["summary"]["claims"], 0);
    assert_eq!(
        fs::read_dir(runtime).unwrap().count(),
        1,
        "only the fixture socket may exist"
    );
}

struct WatchedChild(Child);
impl Drop for WatchedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn controller_watch_keeps_cycle_failures_separate_from_stream_exit() {
    let state = State::new();
    // Missing config fails before reconciliation, so no guest, storage or service changes.
    let mut child = WatchedChild(
        Command::new(env!("CARGO_BIN_EXE_vzctl"))
            .args([
                "controller",
                "watch",
                "--interval",
                "5",
                "--format",
                "json",
                "-C",
            ])
            .arg(&state.0)
            .env("VZCTL_STATE_DIR", &state.0)
            .env("HOME", &state.0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().take(2) {
            sender.send(line.unwrap()).unwrap();
        }
    });
    for _ in 0..2 {
        let line = receiver.recv_timeout(Duration::from_secs(8)).unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["command"], "controller.watch");
        assert_eq!(value["status"], "fail");
        assert_eq!(value["exit_code"], 24);
        assert!(value["summary"].is_object());
    }
    reader.join().unwrap();
    // Only signal the process created by this fixture.
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGINT) }, 0);
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "watch did not stop after SIGINT");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
    assert_eq!(fs::read_dir(&state.0).unwrap().count(), 0);
}
