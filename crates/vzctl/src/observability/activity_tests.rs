use super::tests::status_ingress_environment;
use super::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

fn probe(from: &str, target: &str) -> ObservabilityProbe {
    ObservabilityProbe {
        name: "reachability".into(),
        from: from.into(),
        target: target.into(),
        expect: ProbeExpect::Tcp,
    }
}

#[test]
fn stopped_stack_collects_neutral_report_without_dependent_rpc() {
    let mut environment = status_ingress_environment();
    environment
        .spec
        .vms
        .get_mut("web")
        .unwrap()
        .roles
        .push("router".into());
    environment.spec.observability.probes = vec![
        probe("web", "example.invalid:80"),
        probe("host", "app.svc.demo.vz.test:443"),
    ];
    let dir = std::env::temp_dir().join(format!("vz-stop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("rpc.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        // Only inventory is needed. Any guest, route or ingress lookup would
        // either violate this sequence or hit the closed socket and add warnings.
        for (method, result) in [
            ("vm.list", json!([])),
            ("net.list", json!({"attachments":[]})),
            ("dns.status", json!({"ok":false})),
            (
                "daemon.health",
                json!({"ok":false,"network_resilience":{"network_egress":{"demo/lan":{"classification":"offline"}}}}),
            ),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], method);
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":request["id"],"result":result})
            )
            .unwrap();
        }
    });
    let report =
        collect_status_from(&environment, &socket).unwrap_or_else(|error| panic!("{error:?}"));
    server.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    let result = report.envelope(Duration::ZERO, false);
    assert_eq!(result["summary"]["health"], "ok");
    assert_eq!(result["summary"]["checks"], "skipped");
    assert_eq!(
        result["summary"]["message"],
        "stack demo stopped; dependent checks skipped"
    );
    assert_eq!(result["summary"]["stopped_vms"], 1);
    assert_eq!(result["warnings"], json!([]));
    assert_eq!(result["vms"][0]["state"], "stopped");
    assert_eq!(result["vms"][0]["agent"]["status"], "skipped");
    assert_eq!(result["stack"]["dns"]["status"], "skipped");
    assert_eq!(result["stack"]["routes"]["routers"][0]["status"], "skipped");
    assert_eq!(result["stack"]["ingress"]["status"], "skipped");
    assert!(result["stack"]["probes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["result"]["status"] == "skipped"));
}

#[test]
fn unavailable_or_ambiguous_inventory_never_implies_stopped() {
    let error = collect_status_from(
        &status_ingress_environment(),
        Path::new("/tmp/vzctl-status-absent/rpc.sock"),
    )
    .err()
    .unwrap();
    assert!(error.0.contains("inventory unavailable"));
    for runtime in [
        json!(null),
        json!({}),
        json!([null]),
        json!([{"state":"running"}]),
        json!([{"vm_id":"demo/web"}]),
        json!([{"vm_id":"","state":"stopped"}]),
        json!([{"vm_id":"demo/web","state":"stopped"},{"vm_id":"demo/web","state":"running"}]),
    ] {
        assert!(validate_inventory(&runtime).is_err(), "{runtime}");
    }
    assert!(validate_inventory(&json!([])).is_ok());
    assert!(validate_inventory(&json!([{"vm_id":"demo/web","state":"unexpected"}])).is_ok());
}

#[test]
fn failed_unknown_and_running_with_unreachable_agent_remain_critical() {
    let environment = status_ingress_environment();
    for state in ["failed", "unknown", "unexpected", "running"] {
        let runtime = json!([{"vm_id":"demo/web","state":state}]);
        let mut health = Health::Ok;
        let mut warnings = Vec::new();
        let vm = inspect_declared_vm(
            &environment,
            "web",
            &environment.spec.vms["web"],
            &runtime,
            &json!({}),
            Path::new("/tmp/vzctl-status-absent/rpc.sock"),
            &mut warnings,
            &mut health,
        );
        assert_eq!(health, Health::Critical, "{state}");
        assert!(!warnings.is_empty());
        assert_ne!(vm["checks"]["status"], "skipped");
    }
}

#[test]
fn partial_stop_skips_only_matching_dependencies() {
    let mut environment = status_ingress_environment();
    let mut other = environment.spec.vms["web"].clone();
    other.networks[0].name = "other".into();
    other.networks[0].ip = "10.91.0.10".into();
    environment.spec.vms.insert("other".into(), other);
    let runtime = json!([{"vm_id":"demo/other","state":"running"}]);
    let activity = Activity::new(&environment, &runtime);
    assert!(!activity.all_stopped());
    assert!(!activity.checks_skipped());
    assert!(!activity.network_needed("lan"));
    assert!(activity.network_needed("other"));
    for (from, target) in [
        ("other", "web.lan.demo.vz.test:80"),
        ("host", "10.90.0.10:80"),
        ("host", "https://app.svc.demo.vz.test/"),
        ("web", "external.invalid:80"),
    ] {
        assert_eq!(
            activity.probe_dependency(&probe(from, target)).as_deref(),
            Some("web")
        );
    }
    assert_eq!(
        activity.probe_dependency(&probe("host", "external.invalid:80")),
        None
    );
    assert_eq!(
        activity.probe_dependency(&probe("host", "10.91.0.10:80")),
        None
    );
    let mut warnings = Vec::new();
    let mut health = Health::Ok;
    let result = builtin_peer_probes(
        &environment,
        "other",
        &environment.spec.vms["other"],
        "demo/other",
        Path::new("/tmp/vzctl-status-absent/rpc.sock"),
        &activity,
        &mut warnings,
        &mut health,
    );
    // Different networks have no builtin peer; explicit probes above retain the target identity.
    assert!(result.is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn host_oidc_and_unrelated_host_probe_remain_evaluated_when_vms_stop() {
    let mut environment = status_ingress_environment();
    // A VM named oidc does not own the reserved host OIDC upstream. requires
    // entries are capabilities, not VM names.
    environment
        .spec
        .vms
        .insert("oidc".into(), environment.spec.vms["web"].clone());
    let route = &mut environment.spec.ingress.as_mut().unwrap().routes[0];
    route.to = "oidc:5556".into();
    route.requires = vec!["oidc".into()];
    let runtime = json!([]);
    let activity = Activity::new(&environment, &runtime);
    assert!(activity.all_stopped());
    assert!(activity.host_ingress_needed());
    assert!(!activity.checks_skipped());
    assert!(activity
        .ingress_dependency(&environment.spec.ingress.as_ref().unwrap().routes[0])
        .is_none());
    let mut warnings = Vec::new();
    let mut health = Health::Ok;
    let report = ingress_snapshot(
        &environment,
        Path::new("/tmp/vzctl-status-absent/rpc.sock"),
        &json!({}),
        &activity,
        &mut warnings,
        &mut health,
    );
    assert_eq!(report["status"], "evaluated");
    assert_ne!(health, Health::Ok);
    assert!(!warnings.is_empty());

    environment.spec.ingress = None;
    // Invalid address fails deterministically without external network traffic.
    environment.spec.observability.probes = vec![probe("host", "127.0.0.1:0")];
    let activity = Activity::new(&environment, &runtime);
    assert!(!activity.checks_skipped());
    let mut health = Health::Ok;
    let mut warnings = Vec::new();
    let report = apply_yaml_probes(
        &environment,
        Path::new("/unused"),
        &activity,
        &mut [],
        &mut warnings,
        &mut health,
    );
    assert_eq!(report[0]["result"]["ok"], false);
    assert_eq!(health, Health::Degraded);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn stopped_peer_and_router_are_skipped_but_active_router_failure_is_reported() {
    let mut environment = status_ingress_environment();
    environment
        .spec
        .vms
        .get_mut("web")
        .unwrap()
        .roles
        .push("router".into());
    let mut other = environment.spec.vms["web"].clone();
    other.networks[0].ip = "10.90.0.11".into();
    environment.spec.vms.insert("other".into(), other);
    let runtime = json!([{"vm_id":"demo/other","state":"running"}]);
    let activity = Activity::new(&environment, &runtime);
    let mut warnings = Vec::new();
    let mut health = Health::Ok;
    let result = builtin_peer_probes(
        &environment,
        "other",
        &environment.spec.vms["other"],
        "demo/other",
        Path::new("/tmp/vzctl-status-absent/rpc.sock"),
        &activity,
        &mut warnings,
        &mut health,
    );
    assert_eq!(result[0]["result"]["status"], "skipped");
    assert_eq!(result[0]["result"]["vm"], "web");
    assert_eq!(health, Health::Ok);
    let mut queried = Vec::new();
    let routes = route_snapshot_with(
        &environment,
        Some(&activity),
        &mut warnings,
        &mut health,
        |id| {
            queried.push(id.to_owned());
            Err((1, "active router query failed".into()))
        },
    );
    assert_eq!(queried, vec!["demo/other"]);
    assert_eq!(routes["routers"][1]["status"], "skipped");
    assert_eq!(health, Health::Degraded);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].vm_id.as_deref(), Some("demo/other"));

    // Capability dependencies do not name VMs (even a stopped VM named oidc).
    environment
        .spec
        .vms
        .insert("oidc".into(), environment.spec.vms["web"].clone());
    let route = &mut environment.spec.ingress.as_mut().unwrap().routes[0];
    route.to = "other:80".into();
    route.requires = vec!["oidc".into()];
    assert!(Activity::new(&environment, &runtime)
        .ingress_dependency(&environment.spec.ingress.as_ref().unwrap().routes[0])
        .is_none());
}

#[test]
fn network_diagnostics_keep_declared_scope_and_active_members() {
    let mut environment = status_ingress_environment();
    let mut other = environment.spec.vms["web"].clone();
    other.networks[0].ip = "10.90.0.11".into();
    environment.spec.vms.insert("other".into(), other);
    // A VM with no explicit attachment uses the runtime default, which this
    // existing diagnostic does not claim to assess: only spec.networks is scoped.
    let mut default_vm = environment.spec.vms["web"].clone();
    default_vm.networks.clear();
    environment.spec.vms.insert("default-vm".into(), default_vm);
    let runtime = json!([{"vm_id":"demo/other","state":"running"},{"vm_id":"demo/default-vm","state":"running"}]);
    let activity = Activity::new(&environment, &runtime);
    let services = json!({"network_resilience":{"network_egress":{
        "demo/lan":{"classification":"offline"},"default":{"classification":"offline"},
        "foreign/lan":{"classification":"offline"}
    }}});
    let mut warnings = Vec::new();
    let mut health = Health::Ok;
    append_network_health(
        &environment,
        &services,
        Some(&activity),
        &mut warnings,
        &mut health,
    );
    assert_eq!(health, Health::Degraded);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("demo/lan"));
    let runtime = json!([{"vm_id":"demo/default-vm","state":"running"}]);
    warnings.clear();
    health = Health::Ok;
    append_network_health(
        &environment,
        &services,
        Some(&Activity::new(&environment, &runtime)),
        &mut warnings,
        &mut health,
    );
    assert!(
        warnings.is_empty(),
        "foreign/default networks must not be attributed to this declared network"
    );
    assert_eq!(health, Health::Ok);
}
