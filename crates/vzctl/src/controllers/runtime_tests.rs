//! End-to-end runtime contracts using a private state directory and a fake UDS
//! supervisor. No real daemon, VM, database or process environment is modified.
use super::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::{atomic::AtomicU64, Mutex};
use std::thread::{self, JoinHandle};

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    config: PathBuf,
    environment: crate::config::Environment,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Keep Unix socket paths well below macOS's sockaddr_un limit.
        let root = PathBuf::from(format!(
            "/tmp/vzcrt-{}-{}-{}",
            std::process::id(),
            now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let state = root.join("state");
        let config = root.join("hypernetwork.config.yaml");
        let mut environment: crate::config::Environment = serde_yaml::from_str(include_str!(
            "../../tests/fixtures/validate/valid-kubernetes.yaml"
        ))
        .unwrap();
        environment.spec.kubernetes.clusters.clear();
        environment.spec.service_providers = serde_json::from_value(json!({
            "database":{"kind":"postgres","via":"kube-server","endpoint":"127.0.0.1:5432",
                "advertiseEndpoint":"db.lan.example.test:5432","adminUser":"postgres","passwordFile":"admin"}
        }))
        .unwrap();
        environment.spec.service_claims = serde_json::from_value(json!({
            "app-db":{"kind":"postgres","provider":"database","database":"app"}
        }))
        .unwrap();
        fs::write(&config, serde_yaml::to_string(&environment).unwrap()).unwrap();
        fs::write(root.join("admin"), "ADMIN-CREDENTIAL-DO-NOT-LOG").unwrap();
        Self {
            root,
            state,
            config,
            environment,
        }
    }
    fn controller_dir(&self) -> PathBuf {
        directory(&self.state, &self.environment.spec.project)
    }
    fn private_binding(&self) -> Value {
        serde_json::from_slice(
            &read_private(
                &self
                    .controller_dir()
                    .join("bindings")
                    .join(format!("{}.json", digest("app-db"))),
            )
            .unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Supervisor {
    socket: PathBuf,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Supervisor {
    fn new(
        fixture: &Fixture,
        mut respond: impl FnMut(&Value) -> Option<Value> + Send + 'static,
    ) -> Self {
        let socket = fixture.root.join("rpc.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // BSD/macOS may inherit O_NONBLOCK from the listener.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut line = String::new();
                        BufReader::new(stream.try_clone().unwrap())
                            .read_line(&mut line)
                            .unwrap();
                        let request: Value = serde_json::from_str(&line).unwrap();
                        recorded.lock().unwrap().push(request.clone());
                        if let Some(response) = respond(&request) {
                            writeln!(stream, "{response}").unwrap();
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("fake supervisor accept failed: {error}"),
                }
            }
        });
        Self {
            socket,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    fn methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_owned())
            .collect()
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let joined = self.thread.take().unwrap().join();
        if !thread::panicking() {
            assert!(joined.is_ok(), "fake supervisor thread failed");
        }
    }
}

fn rpc_result(result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":1,"result":result})
}
fn successful_postgres(request: &Value) -> Option<Value> {
    assert_eq!(request["method"], "vm.exec");
    assert_eq!(request["params"]["cmd"][0], "psql");
    let stdin = base64::engine::general_purpose::STANDARD
        .decode(request["params"]["stdin_b64"].as_str().unwrap())
        .unwrap();
    Some(rpc_result(json!({"exit":0,"stdout":
        if stdin==b"SELECT 'VZCTL_READY';\n" {"VZCTL_READY\n"}
        else {"VZCTL_RESULT:{\"changed\":true}\n"},
        "stderr":"ADMIN-CREDENTIAL-DO-NOT-LOG"})))
}

#[test]
fn reconciled_yaml_becomes_pending_after_claim_or_provider_change_without_writing_state() {
    let fixture = Fixture::new();
    let supervisor = Supervisor::new(&fixture, successful_postgres);
    let first = reconcile(
        &fixture.environment,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
    )
    .unwrap();
    assert_eq!(first["claims"][0]["state"], "Ready");
    let original_snapshot = read_private(&fixture.controller_dir().join("snapshot.json")).unwrap();
    let claim_record = fixture
        .controller_dir()
        .join(format!("{}.json", digest("app-db")));
    let original_record = read_private(&claim_record).unwrap();
    assert_eq!(
        status(&fixture.environment, &fixture.state).unwrap()["claims"][0]["state"],
        "Ready"
    );
    for change_provider in [false, true] {
        let mut changed = fixture.environment.clone();
        if change_provider {
            changed
                .spec
                .service_providers
                .get_mut("database")
                .unwrap()
                .advertise_endpoint = Some("new-db.lan.example.test:5432".into());
        } else {
            changed
                .spec
                .service_claims
                .get_mut("app-db")
                .unwrap()
                .database = Some("new_app".into());
        }
        let current = status(&changed, &fixture.state).unwrap();
        assert_eq!(current["claims"][0]["state"], "Pending");
        assert_eq!(
            current["claims"][0]["conditions"][0]["reason"],
            "DesiredStateChanged"
        );
    }
    assert_eq!(read_private(&claim_record).unwrap(), original_record);
    assert_eq!(
        read_private(&fixture.controller_dir().join("snapshot.json")).unwrap(),
        original_snapshot
    );
    assert_eq!(
        supervisor.methods(),
        vec!["vm.exec", "vm.exec"],
        "status must not contact a daemon"
    );
}

#[test]
fn removed_claim_is_retained_and_never_deletes_its_private_credential() {
    let fixture = Fixture::new();
    let supervisor = Supervisor::new(&fixture, successful_postgres);
    reconcile(
        &fixture.environment,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
    )
    .unwrap();
    let before = fixture.private_binding();
    let mut removed = fixture.environment.clone();
    removed.spec.service_claims.clear();
    let current = status(&removed, &fixture.state).unwrap();
    assert_eq!(current["claims"][0]["state"], "Retained");
    let reconciled = reconcile(
        &removed,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
    )
    .unwrap();
    assert_eq!(reconciled["claims"][0]["state"], "Retained");
    assert_eq!(fixture.private_binding(), before);
    assert_eq!(
        supervisor.methods(),
        vec!["vm.exec", "vm.exec"],
        "retaining data must not send destructive guest commands"
    );
}

#[test]
fn real_runtime_keeps_generated_credentials_out_of_status_and_snapshot() {
    let fixture = Fixture::new();
    let supervisor = Supervisor::new(&fixture, successful_postgres);
    let result = reconcile(
        &fixture.environment,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
    )
    .unwrap();
    let binding = fixture.private_binding();
    let secret = binding["secrets"]["password"].as_str().unwrap();
    assert!(
        secret.len() >= 32,
        "test must cover the actual generated credential"
    );
    for public in [
        result.to_string(),
        status(&fixture.environment, &fixture.state)
            .unwrap()
            .to_string(),
        String::from_utf8(read_private(&fixture.controller_dir().join("snapshot.json")).unwrap())
            .unwrap(),
    ] {
        assert!(
            !public.contains(secret),
            "generated password leaked into public state"
        );
        assert!(!public.contains("ADMIN-CREDENTIAL-DO-NOT-LOG"));
        assert!(!public.contains("\"secrets\""));
    }
    assert_eq!(
        fs::metadata(
            fixture
                .controller_dir()
                .join("bindings")
                .join(format!("{}.json", digest("app-db")))
        )
        .unwrap()
        .permissions()
        .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn failed_provider_and_discovery_diagnostics_are_redacted_without_blocking_other_claims() {
    let mut fixture = Fixture::new();
    let with_cluster: crate::config::Environment = serde_yaml::from_str(include_str!(
        "../../tests/fixtures/validate/valid-kubernetes.yaml"
    ))
    .unwrap();
    fixture.environment.spec.kubernetes = with_cluster.spec.kubernetes;
    let supervisor = Supervisor::new(&fixture, |request| {
        if request["params"]["cmd"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "kubectl")
        {
            Some(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,
                "message":"DISCOVERY-SECRET-DO-NOT-LOG"}}))
        } else {
            successful_postgres(request)
        }
    });
    let result = reconcile(
        &fixture.environment,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
    )
    .unwrap();
    assert_eq!(
        result["claims"][0]["state"], "Ready",
        "offline discovery must not block YAML provisioning"
    );
    assert!(!result["warnings"].as_array().unwrap().is_empty());
    assert!(!result.to_string().contains("DISCOVERY-SECRET-DO-NOT-LOG"));
    drop(supervisor);

    fs::remove_file(fixture.root.join("rpc.sock")).unwrap();
    fixture.environment.spec.kubernetes.clusters.clear();
    let supervisor = Supervisor::new(&fixture, |_| {
        Some(rpc_result(json!({
            "exit":1,"stdout":"PROVIDER-SECRET-DO-NOT-LOG",
            "stderr":"FATAL password=PROVIDER-SECRET-DO-NOT-LOG"
        })))
    });
    let result = reconcile(
        &fixture.environment,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
    )
    .unwrap();
    assert_eq!(result["claims"][0]["state"], "Degraded");
    assert!(!result.to_string().contains("PROVIDER-SECRET-DO-NOT-LOG"));
    assert!(!status(&fixture.environment, &fixture.state)
        .unwrap()
        .to_string()
        .contains("PROVIDER-SECRET-DO-NOT-LOG"));
}

#[test]
fn revoked_or_unverifiable_registration_sends_no_guest_mutation() {
    for response in [
        Some(rpc_result(
            json!({"enabled":false,"generation":"17","config_path":"/stack/config"}),
        )),
        Some(rpc_result(
            json!({"enabled":true,"generation":"18","config_path":"/stack/config"}),
        )),
        Some(rpc_result(
            json!({"enabled":true,"generation":"17","config_path":"/another/config"}),
        )),
        Some(
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"REGISTRATION-SECRET-DO-NOT-LOG"}}),
        ),
        None, // The socket accepts the request but closes before replying.
    ] {
        let fixture = Fixture::new();
        let supervisor = Supervisor::new(&fixture, move |_| response.clone());
        let registration = Registration {
            project: "kube-lab".into(),
            generation: "17".into(),
            config_path: "/stack/config".into(),
        };
        let guest = RpcGuest {
            socket: &supervisor.socket,
            registration: Some(&registration),
        };
        let error = match guest.run(
            "kube-lab/kube-server",
            &["mutating-command".into()],
            &BTreeMap::new(),
            b"private-input",
            1000,
        ) {
            Err(error) => error,
            Ok(_) => panic!("registration guard must reject the mutation"),
        };
        assert!(["RegistrationRevoked", "RegistrationUnavailable"].contains(&error.code));
        assert!(!error.to_string().contains("REGISTRATION-SECRET-DO-NOT-LOG"));
        assert_eq!(supervisor.methods(), vec!["controller.registration"]);
    }
}

#[test]
fn registration_is_rechecked_before_every_guest_mutation() {
    let fixture = Fixture::new();
    let mut checks = 0;
    let supervisor = Supervisor::new(&fixture, move |request| {
        if request["method"] == "controller.registration" {
            checks += 1;
            Some(rpc_result(
                json!({"enabled":checks==1,"generation":"17","config_path":"/stack/config"}),
            ))
        } else {
            Some(rpc_result(json!({"exit":0,"stdout":"","stderr":""})))
        }
    });
    let registration = Registration {
        project: "kube-lab".into(),
        generation: "17".into(),
        config_path: "/stack/config".into(),
    };
    let guest = RpcGuest {
        socket: &supervisor.socket,
        registration: Some(&registration),
    };
    guest
        .run(
            "kube-lab/kube-server",
            &["first-command".into()],
            &BTreeMap::new(),
            &[],
            1000,
        )
        .unwrap();
    let error = match guest.run(
        "kube-lab/kube-server",
        &["second-command".into()],
        &BTreeMap::new(),
        &[],
        1000,
    ) {
        Err(error) => error,
        Ok(_) => panic!("disabled registration must prevent the second command"),
    };
    assert_eq!(error.code, "RegistrationRevoked");
    assert_eq!(
        supervisor.methods(),
        vec![
            "controller.registration",
            "vm.exec",
            "controller.registration"
        ]
    );
}

#[test]
fn guarded_reconcile_rejects_disabled_registration_before_provisioning() {
    let fixture = Fixture::new();
    let supervisor = Supervisor::new(&fixture, |_| Some(rpc_result(json!({"enabled":false}))));
    let registration = Registration {
        project: "kube-lab".into(),
        generation: "17".into(),
        config_path: fixture.config.display().to_string(),
    };
    let error = reconcile_guarded(
        &fixture.environment,
        &fixture.config,
        &supervisor.socket,
        &fixture.state,
        Some(&registration),
    )
    .unwrap_err();
    assert_eq!(error.code, "RegistrationRevoked");
    assert_eq!(supervisor.methods(), vec!["controller.registration"]);
    assert!(!fixture.controller_dir().join("snapshot.json").exists());
    assert!(!fixture.controller_dir().join("bindings").exists());
}
