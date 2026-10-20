use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn network_rpc_requests(
    snapshot: Value,
    work: impl FnOnce(&Path) -> Result<(), Failure>,
) -> (Result<(), Failure>, Vec<Value>) {
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    use std::os::unix::net::UnixListener;
    let path = PathBuf::from(format!(
        "/tmp/vz-net-{fixture}-{}-{}.sock",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = if request["method"] == "net.list" {
                snapshot.clone()
            } else {
                json!({})
            };
            writeln!(stream, "{}", json!({"result":result})).unwrap();
            if request["method"] == "test.done" {
                break;
            }
            requests.push(request);
        }
        requests
    });
    let result = work(&path);
    rpc(&path, "test.done", json!({})).unwrap();
    let requests = server.join().unwrap();
    fs::remove_file(path).unwrap();
    (result, requests)
}

fn scoped_network_environment() -> Environment {
    config::validate_path(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/validate/valid-kubernetes.yaml"),
    )
    .unwrap()
}

#[test]
fn stack_route_apply_always_transmits_its_project_scope() {
    let mut environment = scoped_network_environment();
    environment.spec.policies = serde_json::from_value(json!([{
        "name":"own-policy","network":"lan","forward":"deny-all","allow":[]
    }]))
    .unwrap();
    let (result, requests) =
        network_rpc_requests(json!({}), |socket| apply_routes(&environment, socket));
    result.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "route.apply");
    assert_eq!(requests[0]["params"]["project"], environment.spec.project);
}

#[test]
fn apply_reuses_pinned_sealed_image_with_old_or_missing_guest_toolkit() {
    let mut environment = scoped_network_environment();
    environment.spec.images = BTreeMap::from([(
        "base".into(),
        config::ImageConfig {
            from: "test-base".into(),
            role: config::ImageRole::Base,
            tag: "pinned".into(),
        },
    )]);
    let directory = std::env::temp_dir().join(format!(
        "vzctl-pinned-image-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(directory.join("sealed")).unwrap();
    fs::create_dir_all(directory.join("aliases")).unwrap();
    let raw = directory.join("sealed/test-base@pinned.raw");
    let marker = directory.join("pinned.sealed.json");
    fs::write(&raw, b"immutable sealed bytes").unwrap();
    fs::write(&marker, b"{}").unwrap();
    for toolkit in [json!("0.0.1"), Value::Null] {
        let manifest = json!({
            "apiVersion":"vzctl.dev/image-alias/v1",
            "tags":{"pinned":{
                "sealed":true,
                "baked_image":{"agent_version":toolkit},
                "sealed_image":{"path":"sealed/test-base@pinned.raw","marker":marker}
            }}
        });
        let alias = directory.join("aliases/test-base.json");
        let original = serde_json::to_vec(&manifest).unwrap();
        fs::write(&alias, &original).unwrap();
        vm::ensure_images_from_cache(
            &environment,
            &directory,
            &mut ProgressReporter::new(ProgressMode::Off, None, &[]),
            |_, _| panic!("pinned sealed image must not trigger pull, bake, or seal"),
        )
        .unwrap();
        assert_eq!(fs::read(&raw).unwrap(), b"immutable sealed bytes");
        assert_eq!(fs::read(&alias).unwrap(), original);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn forced_network_reconcile_never_mutates_another_project_with_same_name() {
    let environment = scoped_network_environment();
    let (result, requests) = network_rpc_requests(
        json!({
            "networks":[{"name":"lan","id":"lan","project":"iq-store","stack":"iq-store:iq-store",
                "cidr":"10.92.0.0/24","mode":"shared","nat_egress":true,"backend":"vmnet"}],
            "attachments":[{"vm_id":"iq-store/docker","network":"lan","project":"iq-store","ip":"10.92.0.2"}]
        }),
        |path| ensure_networks(&environment, true, path),
    );
    result.unwrap();
    let methods: Vec<_> = requests
        .iter()
        .map(|r| r["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, ["net.list", "net.create"]);
    assert_eq!(requests[1]["params"]["project"], "kube-lab");
}

#[test]
fn network_overlap_gate_rejects_foreign_cidr_before_any_forced_mutation() {
    let environment = scoped_network_environment();
    let (result, requests) = network_rpc_requests(
        json!({
            "networks":[{"name":"lan","project":"other","cidr":"10.90.0.0/24"}]
        }),
        |path| ensure_networks(&environment, true, path),
    );
    assert!(result.unwrap_err().message.contains("overlaps"));
    assert_eq!(requests.len(), 1);
}

#[test]
fn network_down_and_forced_recreate_only_address_scoped_owner() {
    let environment = scoped_network_environment();
    let snapshot = json!({
        "networks":[
            {"id":"lan","name":"lan","project":"iq-store","stack":"iq-store:iq-store","cidr":"10.92.0.0/24"},
            {"id":"kube-lab/lan","name":"lan","project":"kube-lab","stack":"kube-lab:kube-lab","cidr":"10.89.0.0/24",
             "mode":"shared","nat_egress":true,"backend":"vmnet","labels":{"managed-by":"vzctl"}}],
        "attachments":[
            {"vm_id":"iq-store/docker","network":"lan","network_id":"lan","project":"iq-store","stack":"iq-store:iq-store","ip":"10.92.0.2"},
            {"vm_id":"kube-lab/kube-server","network":"lan","network_id":"kube-lab/lan",
             "project":"kube-lab","stack":"kube-lab:kube-lab","ip":"10.89.0.10"}]
    });
    let (result, requests) = network_rpc_requests(snapshot.clone(), |path| {
        ensure_networks(&environment, true, path)
    });
    result.unwrap();
    assert_eq!(requests[1]["method"], "net.detach");
    assert_eq!(requests[1]["params"]["vm_id"], "kube-lab/kube-server");
    assert_eq!(requests[2]["method"], "net.delete");
    assert_eq!(requests[2]["params"]["name"], "kube-lab/lan");
    assert!(requests
        .iter()
        .skip(1)
        .all(|r| r["params"]["project"] == "kube-lab"));
    let mut no_vms = environment.clone();
    no_vms.spec.vms.clear();
    let (result, requests) = network_rpc_requests(snapshot, |path| {
        detach_networks(&no_vms, path)?;
        purge_managed(
            &no_vms,
            path,
            &mut ProgressReporter::new(ProgressMode::Off, None, &[]),
        )
    });
    result.unwrap();
    let mutations: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] != "net.list")
        .collect();
    assert_eq!(mutations.len(), 2);
    assert_eq!(mutations[0]["params"]["vm_id"], "kube-lab/kube-server");
    assert_eq!(mutations[1]["params"]["name"], "kube-lab/lan");
    assert!(mutations
        .iter()
        .all(|r| r["params"]["project"] == "kube-lab"));
}

#[test]
fn force_recreate_rejects_every_foreign_attachment_before_first_mutation() {
    let environment = scoped_network_environment();
    for stack in [json!("other-stack"), Value::Null] {
        let snapshot = json!({
            "networks":[{"id":"kube-lab/lan","name":"lan","project":"kube-lab",
                "stack":"kube-lab:kube-lab","cidr":"10.89.0.0/24","mode":"shared"}],
            "attachments":[
                {"vm_id":"kube-lab/kube-server","network":"lan","network_id":"kube-lab/lan",
                 "project":"kube-lab","stack":"kube-lab:kube-lab","ip":"10.89.0.10"},
                {"vm_id":"kube-lab/manual","network":"lan","network_id":"kube-lab/lan",
                 "project":"kube-lab","stack":stack,"ip":"10.89.0.11"}]
        });
        let (result, requests) =
            network_rpc_requests(snapshot, |path| ensure_networks(&environment, true, path));
        assert!(result
            .unwrap_err()
            .message
            .contains("foreign or unowned attachments"));
        assert_eq!(
            requests.len(),
            1,
            "must not detach even the earlier owned attachment"
        );
    }
}

#[test]
fn edge_reconcile_rpc_uses_extended_timeout() {
    assert_eq!(edge_reconcile_rpc_timeout_secs("ingress.ensure"), Some(90));
    assert_eq!(rpc_timeout_secs("ingress.ensure", &json!({})), 90);
    assert_eq!(rpc_timeout_secs("vm.list", &json!({})), 10);
    // Supervisor start can spend 5s releasing the old helper and another 5s
    // confirming its successor. The transport deadline must leave headroom.
    assert_eq!(rpc_timeout_secs("vm.start", &json!({})), 20);
}

#[test]
fn rpc_transient_failure_matches_ingress_timeout_and_edge_errors() {
    assert!(rpc_transient_failure(
        "ingress.ensure: Resource temporarily unavailable (os error 35)"
    ));
    assert!(rpc_transient_failure("vz-edge did not respond"));
    assert!(rpc_transient_failure(
        "dns-bind connect: Connection refused"
    ));
    assert!(!rpc_transient_failure("invalid Caddyfile"));
}

#[test]
fn ca_is_ensured_before_vm_creation() {
    let ca = APPLY_STEPS
        .iter()
        .position(|step| *step == "ensure_ca")
        .unwrap();
    let vms = APPLY_STEPS
        .iter()
        .position(|step| *step == "ensure_vms")
        .unwrap();
    assert!(ca < vms);
}

#[test]
fn guest_utils_runs_after_agents_before_mounts() {
    let agents = APPLY_STEPS
        .iter()
        .position(|step| *step == "await_agents")
        .unwrap();
    let utils = APPLY_STEPS
        .iter()
        .position(|step| *step == "ensure_guest_utils")
        .unwrap();
    let cloud_init = APPLY_STEPS
        .iter()
        .position(|step| *step == "await_cloud_init")
        .unwrap();
    let mounts = APPLY_STEPS
        .iter()
        .position(|step| *step == "ensure_docker_project_mount")
        .unwrap();
    assert!(agents < cloud_init);
    assert!(cloud_init < utils);
    assert!(utils < mounts);
}

#[test]
fn cloud_init_json_tracks_stages_and_strict_failure_codes() {
    let running = parse_cloud_init_json(
        0,
        &json!({
            "status": "running",
            "extended_status": "running",
            "stage": "modules-config",
            "datasource": "nocloud",
            "errors": [],
            "recoverable_errors": {},
        }),
    );
    assert_eq!(running.state, CloudInitState::Running);
    assert_eq!(running.stage.as_deref(), Some("modules-config"));
    assert_eq!(running.datasource.as_deref(), Some("nocloud"));

    let done = parse_cloud_init_json(
        0,
        &json!({"status": "done", "errors": [], "recoverable_errors": {}}),
    );
    assert_eq!(done.state, CloudInitState::Done);

    let degraded = parse_cloud_init_json(
        2,
        &json!({"status": "done", "recoverable_errors": {"warning": ["x"]}}),
    );
    assert!(matches!(degraded.state, CloudInitState::Failed(_)));
}

#[test]
fn cloud_init_long_fallback_handles_running_disabled_and_failure() {
    let running = parse_cloud_init_long(
        0,
        "status: running\ndetail:\nRunning in stage: modules-final\n",
        "",
    );
    assert_eq!(running.state, CloudInitState::Running);
    assert_eq!(running.stage.as_deref(), Some("modules-final"));
    assert!(matches!(
        parse_cloud_init_long(0, "status: disabled", "").state,
        CloudInitState::Failed(_)
    ));
    assert!(matches!(
        parse_cloud_init_long(1, "status: error", "").state,
        CloudInitState::Failed(_)
    ));
}

#[test]
fn cloud_init_monitor_emits_all_four_stages_and_completion() {
    let stages = ["init-local", "init", "modules-config", "modules-final"];
    let mut observations = stages
        .iter()
        .map(|stage| CloudInitObservation {
            state: CloudInitState::Running,
            stage: Some((*stage).to_string()),
            datasource: Some("NoCloud".to_string()),
        })
        .chain(std::iter::once(CloudInitObservation {
            state: CloudInitState::Done,
            stage: None,
            datasource: None,
        }));
    let (tx, rx) = mpsc::channel();
    monitor_cloud_init_with(
        "demo/web",
        Duration::from_secs(1),
        Duration::ZERO,
        tx,
        || Ok(observations.next().expect("finite cloud-init sequence")),
    );
    let events = rx.into_iter().collect::<Vec<_>>();
    let reported = events
        .iter()
        .filter_map(|event| match event {
            CloudInitEvent::Update { stage, .. } => stage.as_deref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reported, stages);
    assert!(matches!(events.last(), Some(CloudInitEvent::Done { .. })));
}

#[test]
fn cloud_init_monitors_multiple_vms_in_parallel() {
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for vm_id in ["demo/web", "demo/db"] {
            let tx = tx.clone();
            scope.spawn(move || {
                monitor_cloud_init_with(vm_id, Duration::from_secs(1), Duration::ZERO, tx, || {
                    Ok(CloudInitObservation {
                        state: CloudInitState::Done,
                        stage: None,
                        datasource: None,
                    })
                });
            });
        }
    });
    drop(tx);
    let completed = rx
        .into_iter()
        .filter_map(|event| match event {
            CloudInitEvent::Done { vm_id } => Some(vm_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        completed,
        BTreeSet::from(["demo/db".into(), "demo/web".into()])
    );
}

#[test]
fn cloud_init_monitor_reports_timeout_and_missing_command() {
    let (tx, rx) = mpsc::channel();
    monitor_cloud_init_with("demo/web", Duration::ZERO, Duration::ZERO, tx, || {
        Err(Failure::new(EXIT_STEP, "agent antwortet nicht"))
    });
    let event = rx.recv().unwrap();
    assert!(matches!(
        event,
        CloudInitEvent::Failed { message, .. }
            if message.contains("Timeout") && message.contains("agent antwortet nicht")
    ));
    assert!(cloud_init_command_missing(127, ""));
    assert!(cloud_init_command_missing(
        1,
        "cloud-init: command not found"
    ));
    assert!(cloud_init_command_missing(1, "No such file or directory"));
    assert!(!cloud_init_command_missing(1, "status: error"));
}

#[test]
fn cloud_init_diagnostics_redact_sensitive_values() {
    assert_eq!(
        safe_status_text("authorization token=top-secret"),
        "[sensible Diagnose ausgeblendet]"
    );
    assert_eq!(safe_status_text("DataSourceNoCloud"), "DataSourceNoCloud");
}

#[test]
fn failed_helper_diagnostics_include_vm_and_safe_error() {
    let wanted = BTreeSet::from(["monitos/monitos-main".to_string()]);
    let diagnostics = failed_helper_diagnostics(
        &json!([{
            "vm_id": "monitos/monitos-main",
            "state": "failed",
            "last_error": "console socket path is too long",
        }]),
        &wanted,
    );

    assert_eq!(
        diagnostics,
        vec!["monitos/monitos-main: console socket path is too long"]
    );
}

#[test]
fn progress_defaults_preserve_json_and_down_contracts() {
    let json = parse(
        "apply",
        ["--format", "json"].into_iter().map(str::to_string),
    )
    .unwrap();
    assert_eq!(json.progress, ProgressMode::Off);

    let down = parse("down", std::iter::empty()).unwrap();
    assert_eq!(down.progress, ProgressMode::Plain);

    let explicit = parse(
        "apply",
        ["--format", "json", "--progress", "plain"]
            .into_iter()
            .map(str::to_string),
    )
    .unwrap();
    assert_eq!(explicit.progress, ProgressMode::Plain);

    let missing = parse("apply", ["--progress"].into_iter().map(str::to_string)).unwrap_err();
    assert_eq!(missing.code, EXIT_USAGE);
    assert!(missing.message.contains("requires plain, ui, or off"));
}

#[test]
fn ca_rollout_uses_primary_agent_method_without_fallback() {
    let targets = vec!["demo/web".to_string()];
    let fingerprint = "a".repeat(64);
    let mut methods = Vec::new();
    rollout_ca_to_vms(&targets, "pem", &fingerprint, |method, _| {
        methods.push(method.to_string());
        Ok(json!({"installed": true, "fingerprint": fingerprint}))
    })
    .unwrap();
    assert_eq!(methods, vec!["vm.agent.ca_inject"]);
}

#[test]
fn ca_rollout_falls_back_and_aggregates_vm_failures() {
    let targets = vec!["demo/web".to_string(), "demo/db".to_string()];
    let fingerprint = "b".repeat(64);
    let mut calls = Vec::new();
    let failure = rollout_ca_to_vms(&targets, "pem", &fingerprint, |method, params| {
        let vm_id = params["vm_id"].as_str().unwrap().to_string();
        calls.push((vm_id.clone(), method.to_string()));
        if method == "vm.agent.ca_inject" {
            return Err(Failure::new(EXIT_STEP, "old agent"));
        }
        if vm_id == "demo/web" {
            Ok(json!({"exit": 1, "stderr": "update failed"}))
        } else {
            Ok(json!({"exit": 0, "stdout": "ok", "stderr": ""}))
        }
    })
    .unwrap_err();
    assert!(failure.message.contains("demo/web"));
    assert!(failure.message.contains("update failed"));
    assert!(!failure.message.contains("demo/db:"));
    assert_eq!(calls.len(), 4);
}

fn resource(kind: &str, name: &str, spec: &str, state: &str) -> Resource {
    Resource {
        kind: kind.to_string(),
        name: name.to_string(),
        labels: BTreeMap::from([("spec".to_string(), spec.to_string())]),
        state: state.to_string(),
    }
}

#[test]
fn complete_config_and_referenced_files_change_resume_hash_and_plan() {
    let root = std::env::temp_dir().join(format!(
        "vz-inputs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/edge-dmz/hypernetwork.config.yaml");
    let mut environment = config::validate_path(&fixture).unwrap();
    let first_vm = environment.spec.vms.values_mut().next().unwrap();
    first_vm.cloud_init = Some("cloud.yaml".into());
    first_vm.compose_files = vec!["compose.yaml".into()];
    fs::write(root.join("cloud.yaml"), "old cloud data").unwrap();
    fs::write(
        root.join("compose.yaml"),
        "services:\n  web:\n    image: nginx\n    env_file: runtime.env\n",
    )
    .unwrap();
    fs::write(root.join("runtime.env"), "PASSWORD=secret-one").unwrap();
    let desired = |env: &Environment| {
        let mut resources = desired_resources(env).unwrap();
        append_input_resources(env, &root, &root.join("state"), &mut resources).unwrap();
        resources
    };
    let initial = desired(&environment);
    let hash = desired_hash(&environment, &initial).unwrap();
    assert!(!serde_json::to_string(&initial)
        .unwrap()
        .contains("secret-one"));
    fs::write(root.join("runtime.env"), "PASSWORD=secret-two").unwrap();
    let changed = desired(&environment);
    assert_ne!(hash, desired_hash(&environment, &changed).unwrap());
    let plan = build_plan(Mode::Apply, changed, initial.clone());
    assert!(plan
        .actions
        .iter()
        .any(|a| a.kind == "input" && a.action == "update"));

    let baseline = serde_json::to_value(&environment).unwrap();
    for (key, value) in [
        ("ports", json!(["65531:web:80"])),
        ("volumes", json!({"data": "/tmp"})),
        ("certs", json!({"enabled": false})),
        ("ingress", json!({"enabled": false, "routes": []})),
        (
            "oidc",
            json!({"enabled": false, "issuer": "https://auth.svc.demo.vz.test"}),
        ),
        (
            "observability",
            json!({"probes": [{"name": "web", "from": "host", "target": "https://web", "expect": "http2xx"}]}),
        ),
        (
            "resilience",
            json!({"network": {"restartVMsOnStuckEgress": true}}),
        ),
    ] {
        // Mutate serialized values from each fixture category, preserving its shape.
        let mut document = baseline.clone();
        document["spec"][key] = value;
        let updated = serde_json::from_value::<Environment>(document).unwrap();
        let resources = desired_resources(&updated).unwrap();
        let baseline_resources = desired_resources(&environment).unwrap();
        assert_ne!(
            desired_hash(&environment, &baseline_resources).unwrap(),
            desired_hash(&updated, &resources).unwrap(),
            "{key}"
        );
        let plan = build_plan(Mode::Apply, resources, baseline_resources);
        assert!(
            plan.actions
                .iter()
                .any(|a| a.kind == key.trim_end_matches('s') || a.kind == key),
            "{key}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn heartbeat_renews_during_work_and_reports_loss() {
    use std::os::unix::net::UnixListener;
    let path = std::env::temp_dir().join(format!("vz-heartbeat-{}.sock", std::process::id()));
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        for index in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "stack.heartbeat");
            assert_eq!(request["params"]["id"], "journal");
            if index == 2 {
                writeln!(
                    stream,
                    "{}",
                    json!({"error": {"code": 6, "message": "lease lost"}})
                )
                .unwrap();
            } else {
                writeln!(stream, "{}", json!({"result": {"renewed": true}})).unwrap();
            }
        }
    });
    let heartbeat =
        LeaseHeartbeat::start(&path, "journal", "stack", "owner", Duration::from_millis(5))
            .unwrap();
    server.join().unwrap();
    let failure = heartbeat.stop().unwrap_err();
    assert_eq!(failure.code, EXIT_LEASE);
    fs::remove_file(path).unwrap();
}

#[test]
fn desired_vm_resources_use_project_runtime_ids() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/edge-dmz/hypernetwork.config.yaml");
    let environment = config::validate_path(&path).expect("edge-dmz config");
    let desired = desired_resources(&environment).unwrap();
    let vm_names = desired
        .iter()
        .filter(|resource| resource.kind == "vm")
        .map(|resource| resource.name.as_str())
        .collect::<BTreeSet<_>>();
    assert!(vm_names.contains("edge-dmz/web"));
    assert!(vm_names.contains("edge-dmz/router"));
    assert!(!vm_names.contains("web"));
    assert!(!vm_names.contains("router"));
}

#[test]
fn idempotent_plan_has_no_actions() {
    let desired = vec![resource("network", "lan", "{}", "active")];
    let plan = build_plan(Mode::Apply, desired.clone(), desired);
    assert!(plan.actions.is_empty());
}

#[test]
fn up_only_creates_and_starts() {
    let desired = vec![resource("vm", "web", "{\"disk\":2}", "running")];
    let actual = vec![
        resource("vm", "web", "{\"disk\":1}", "stopped"),
        resource("network", "old", "{}", "active"),
    ];
    let plan = build_plan(Mode::Up, desired, actual);
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].action, "start");
}

#[test]
fn apply_marks_vm_and_network_updates_breaking() {
    let desired = vec![resource("vm", "web", "{\"disk\":2}", "running")];
    let actual = vec![resource("vm", "web", "{\"disk\":1}", "running")];
    let plan = build_plan(Mode::Apply, desired, actual);
    assert!(plan.actions[0].breaking);
}

#[test]
fn parses_gib_and_tib() {
    assert_eq!(disk_gib("4G").unwrap(), 4);
    assert_eq!(disk_gib("2TiB").unwrap(), 2048);
    assert!(disk_gib("512M").is_err());
}

#[test]
fn bundle_match_rejects_stale_root_capacity() {
    let environment = config::validate_source(include_str!(
        "../../tests/fixtures/validate/valid-full.yaml"
    ))
    .unwrap();
    let vm = &environment.spec.vms["web"];
    let bundle = std::env::temp_dir().join(format!(
        "vzctl-bundle-match-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&bundle).unwrap();
    fs::write(
        bundle.join("vm.json"),
        serde_json::to_vec(&json!({
            "disks": {"root": {"size_gib": 20}},
            "roles": [],
            "identity": {"nics": [{"address": "10.80.0.10"}]}
        }))
        .unwrap(),
    )
    .unwrap();

    assert!(!bundle_matches_vm(&bundle, vm).unwrap());
    fs::remove_dir_all(bundle).unwrap();
}

#[test]
fn resume_keeps_original_down_purge_context() {
    let state = json!({
        "journal": {
            "payload": r#"{"desired_hash":"abc","mode":"down","purge":true}"#
        }
    });
    assert_eq!(journal_context(&state).unwrap(), (Mode::Down, true));
}

#[test]
fn stale_helper_lock_is_safe_when_flock_free() {
    let path = std::env::temp_dir().join(format!(
        "vzctl-adopt-lock-{}-{}.lock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, "999999").unwrap();
    assert!(is_safe_stale_helper_lock(&path));
    fs::remove_file(&path).unwrap();
}

#[test]
fn kubernetes_removal_never_plans_implicit_cluster_prune() {
    let plan = build_plan(
        Mode::Apply,
        vec![],
        vec![resource("kubernetes", "dev", "{}", "ready")],
    );
    assert_eq!(plan.actions[0].action, "retain");
    assert!(!plan.actions[0].breaking);
}

#[test]
fn kubernetes_manifests_change_resume_hash_and_input_diff() {
    let root = std::env::temp_dir().join(format!(
        "vz-k-input-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let source = include_str!("../../tests/fixtures/validate/valid-kubernetes.yaml")
        .replace("kubernetes/workloads.yaml", "app.yaml");
    fs::write(root.join("hypernetwork.config.yaml"), source).unwrap();
    fs::write(
        root.join("app.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata: {name: x}\ndata: {x: one}\n",
    )
    .unwrap();
    let environment = load_config(&root).unwrap();
    let mut before = desired_resources(&environment).unwrap();
    append_input_resources(&environment, &root, &root, &mut before).unwrap();
    fs::write(
        root.join("app.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata: {name: x}\ndata: {x: two}\n",
    )
    .unwrap();
    let mut after = desired_resources(&environment).unwrap();
    append_input_resources(&environment, &root, &root, &mut after).unwrap();
    assert_ne!(
        desired_hash(&environment, &before).unwrap(),
        desired_hash(&environment, &after).unwrap()
    );
    assert!(build_plan(Mode::Diff, after, before)
        .actions
        .iter()
        .any(|a| a.kind == "input" && a.action == "update"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn claims_follow_kubernetes_and_precede_consumer_start() {
    let index = |step| APPLY_STEPS.iter().position(|s| *s == step).unwrap();
    assert!(index("ensure_ingress") < index("ensure_service_claims"));
    assert!(index("apply_routes_policies") < index("ensure_kubernetes"));
    assert!(index("ensure_kubernetes") < index("ensure_service_claims"));
    assert!(index("ensure_service_claims") < index("ensure_containers"));
}

#[test]
fn degraded_claims_fail_apply_without_exposing_binding_values() {
    assert!(
        services::check_claim_readiness(&json!({"claims":[{"name":"db","state":"Ready"}]})).is_ok()
    );
    assert!(services::check_claim_readiness(&json!({})).is_err());
    assert!(services::check_claim_readiness(&json!({"claims":[{"state":"Retained"}]})).is_ok());
    assert!(
        services::check_claim_readiness(&json!({"claims":[],"warnings":["DiscoveryFailed"]}))
            .is_err()
    );
    let failure = services::check_claim_readiness(&json!({"claims":[{"name":"db","state":"Conflict","conditions":[{"type":"Ready","status":"False","reason":"OwnershipConflict","message":"private message"}],"secrets":{"password":"topsecret"}}]})).unwrap_err();
    assert_eq!(failure.code, EXIT_STEP);
    assert!(failure.message.contains("OwnershipConflict"));
    assert!(!failure.message.contains("private message"));
    assert!(!failure.message.contains("topsecret"));
}

#[test]
fn binding_mount_changes_are_planned_as_container_recreation() {
    let resource = |claims| Resource {
        kind: "container_binding".into(),
        name: "docker/api".into(),
        state: "mounted".into(),
        labels: BTreeMap::from([("spec".into(), claims)]),
    };
    let old = resource("[db]".into());
    let new = resource("[db,bucket]".into());
    let plan = build_plan(Mode::Apply, vec![new], vec![old.clone()]);
    assert_eq!(plan.actions[0].action, "recreate");
    assert!(plan.actions[0].reason.contains("binding mounts changed"));
    let removed = build_plan(Mode::Apply, vec![], vec![old]);
    assert_eq!(removed.actions[0].action, "recreate");
    assert!(removed.actions[0].reason.contains("removed"));
}

#[test]
fn down_disables_controllers_before_any_resource_mutation() {
    assert_eq!(DOWN_STEPS.first(), Some(&"disable_service_controllers"));
}

#[test]
fn revoked_edge_guard_prevents_initial_mutations() {
    let environment: Environment = serde_yaml::from_str(include_str!(
        "../../tests/fixtures/validate/valid-kubernetes.yaml"
    ))
    .unwrap();
    let result = reconcile_claim_edge(
        &environment,
        Path::new("/nonexistent"),
        Path::new("/nonexistent"),
        crate::controllers::ServiceKind::Ingress,
        &|| Err("RegistrationRevoked".into()),
    );
    assert_eq!(result.unwrap_err(), "RegistrationRevoked");
}

#[test]
fn transient_edge_retry_rechecks_registration_before_next_rpc() {
    use std::os::unix::net::UnixListener;
    let path = PathBuf::from(format!("/tmp/vz-guard-{}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        writeln!(stream,"{}",json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"vz-edge unavailable"}})).unwrap();
    });
    let checks = std::cell::Cell::new(0);
    let result = rpc_retrying(&path, "ingress.ensure", json!({}), &|| {
        checks.set(checks.get() + 1);
        if checks.get() > 1 {
            Err(Failure::new(EXIT_STEP, "RegistrationRevoked"))
        } else {
            Ok(())
        }
    });
    assert_eq!(result.unwrap_err().message, "RegistrationRevoked");
    assert_eq!(checks.get(), 2);
    server.join().unwrap();
    fs::remove_file(path).unwrap();
}
