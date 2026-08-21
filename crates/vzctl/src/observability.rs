use crate::config::{
    config_path, validate_path, Environment, ObservabilityProbe, ProbeExpect, ValidationIssue,
    VmConfig,
};
use serde_json::{json, Value};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_DEGRADED: u8 = 1;
const EXIT_CRITICAL: u8 = 2;
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const DNS_GUEST_PROBE_FAIL: &str = "DNS_GUEST_PROBE_FAIL";
pub(crate) const DNS_HOST_PROBE_FAIL: &str = "DNS_HOST_PROBE_FAIL";
pub(crate) const AGENT_DEGRADED: &str = "AGENT_DEGRADED";
pub(crate) const AGENT_EXEC_TIMEOUT: &str = "AGENT_EXEC_TIMEOUT";
pub(crate) const VM_NOT_RUNNING: &str = "VM_NOT_RUNNING";
pub(crate) const AGENT_NOT_READY: &str = "AGENT_NOT_READY";
pub(crate) const ROUTE_STATUS_AMBIGUOUS: &str = "ROUTE_STATUS_AMBIGUOUS";
pub(crate) const INGRESS_UNREACHABLE: &str = "INGRESS_UNREACHABLE";
pub(crate) const MOUNT_MISSING: &str = "MOUNT_MISSING";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Health {
    Ok,
    Degraded,
    Critical,
}

impl Health {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Critical => "critical",
        }
    }

    fn exit(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Degraded => EXIT_DEGRADED,
            Self::Critical => EXIT_CRITICAL,
        }
    }

    fn raise(&mut self, other: Self) {
        let rank = |value: Self| match value {
            Self::Ok => 0,
            Self::Degraded => 1,
            Self::Critical => 2,
        };
        if rank(other) > rank(*self) {
            *self = other;
        }
    }
}

struct Warning {
    code: &'static str,
    severity: Health,
    message: String,
    vm_id: Option<String>,
    hint: String,
}

impl Warning {
    fn json(&self) -> Value {
        json!({
            "code": self.code,
            "severity": self.severity.as_str(),
            "message": self.message,
            "vm_id": self.vm_id,
            "hint": self.hint,
        })
    }
}

struct StatusOptions {
    directory: PathBuf,
    format: Format,
    verbose: bool,
}

pub(crate) fn stack_status_command(args: &[String], socket_path: &Path) -> ExitCode {
    match parse_status(args) {
        Ok(options) => emit_status(&options, socket_path),
        Err((message, code)) => {
            eprintln!("{message}");
            ExitCode::from(code)
        }
    }
}

pub(crate) fn status_alias(args: impl Iterator<Item = String>, socket_path: &Path) -> ExitCode {
    stack_status_command(&args.collect::<Vec<_>>(), socket_path)
}

fn parse_status(args: &[String]) -> Result<StatusOptions, (String, u8)> {
    let mut directory = PathBuf::from(".");
    let mut format = Format::Human;
    let mut verbose = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| ("-C requires a path".to_string(), EXIT_USAGE))?;
                directory = PathBuf::from(value);
                index += 2;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| ("--format requires human or json".to_string(), EXIT_USAGE))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err((format!("unsupported stack status format: {other}"), EXIT_USAGE))
                    }
                };
                index += 2;
            }
            "--verbose" => {
                verbose = true;
                index += 1;
            }
            "-h" | "--help" => {
                return Err((
                    "usage: vzctl stack status [-C dir] [--format human|json] [--verbose]".into(),
                    EXIT_USAGE,
                ))
            }
            other => return Err((format!("unknown stack status option: {other}"), EXIT_USAGE)),
        }
    }
    Ok(StatusOptions {
        directory,
        format,
        verbose,
    })
}

fn emit_status(options: &StatusOptions, socket_path: &Path) -> ExitCode {
    let started = Instant::now();
    match collect_status(&options.directory, socket_path) {
        Ok(report) => {
            let envelope = report.envelope(started.elapsed(), options.verbose);
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human => print_human(&envelope),
            }
            ExitCode::from(report.health.exit())
        }
        Err((message, code)) => {
            let envelope = json!({
                "apiVersion": API_VERSION,
                "command": "stack.status",
                "status": "fail",
                "exit_code": code,
                "summary": { "message": message },
            });
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human => eprintln!("{message}"),
            }
            ExitCode::from(code)
        }
    }
}

struct StatusReport {
    health: Health,
    project: String,
    vms: Vec<Value>,
    stack: Value,
    warnings: Vec<Warning>,
}

impl StatusReport {
    fn envelope(&self, elapsed: Duration, verbose: bool) -> Value {
        let criticals = self
            .warnings
            .iter()
            .filter(|warning| warning.severity == Health::Critical)
            .count();
        let mut stack = self.stack.clone();
        if !verbose {
            if let Some(dns) = stack.get_mut("dns") {
                if let Some(object) = dns.as_object_mut() {
                    object.remove("raw");
                }
            }
        }
        json!({
            "apiVersion": API_VERSION,
            "command": "stack.status",
            "status": match self.health {
                Health::Ok => "ok",
                Health::Degraded => "warn",
                Health::Critical => "fail",
            },
            "exit_code": self.health.exit(),
            "summary": {
                "message": format!("stack {} {}", self.project, self.health.as_str()),
                "health": self.health.as_str(),
                "warning_count": self.warnings.len(),
                "critical_count": criticals,
                "elapsed_ms": elapsed.as_millis() as u64,
            },
            "vms": self.vms,
            "stack": stack,
            "warnings": self.warnings.iter().map(Warning::json).collect::<Vec<_>>(),
        })
    }
}

fn collect_status(directory: &Path, socket_path: &Path) -> Result<StatusReport, (String, u8)> {
    let config_file = config_path(directory);
    let environment = validate_path(&config_file).map_err(|issues| {
        (format_issues(&config_file, &issues), EXIT_INVALID)
    })?;
    collect_status_from(&environment, socket_path)
}

fn collect_status_from(
    environment: &Environment,
    socket_path: &Path,
) -> Result<StatusReport, (String, u8)> {
    let project = environment.spec.project.clone();
    let runtime =
        crate::vm::supervisor_rpc_deadline(socket_path, "vm.list", json!({}), 3).unwrap_or(json!([]));
    let nets = crate::vm::supervisor_rpc_deadline(socket_path, "net.list", json!({}), 3)
        .unwrap_or(json!({ "attachments": [] }));
    let dns = crate::vm::supervisor_rpc_deadline(socket_path, "dns.status", json!({}), 3)
        .unwrap_or(json!({ "ok": false }));
    let services = crate::vm::supervisor_rpc_deadline(socket_path, "health", json!({}), 3)
        .unwrap_or(json!({ "ok": false }));

    let mut health = Health::Ok;
    let mut warnings = Vec::new();
    let mut vms = Vec::new();

    if dns["ok"].as_bool() != Some(true) {
        warnings.push(Warning {
            code: DNS_HOST_PROBE_FAIL,
            severity: Health::Critical,
            message: "host DNS listener is not healthy".into(),
            vm_id: None,
            hint: "vzctl dns install-resolver && vzctl dns install-bind-helper".into(),
        });
        health.raise(Health::Critical);
    }

    for (name, vm) in &environment.spec.vms {
        vms.push(inspect_declared_vm(
            environment,
            name,
            vm,
            &runtime,
            &nets,
            socket_path,
            &mut warnings,
            &mut health,
        ));
    }
    apply_yaml_probes(environment, socket_path, &mut vms, &mut warnings, &mut health);
    let routes = route_snapshot(environment, socket_path, &mut warnings, &mut health);
    let ingress = ingress_snapshot(environment, &mut warnings, &mut health);
    Ok(StatusReport {
        health,
        project,
        vms,
        stack: json!({
            "dns": dns_sections(&dns),
            "routes": routes,
            "ingress": ingress,
            "services": services,
        }),
        warnings,
    })
}

fn inspect_declared_vm(
    environment: &Environment,
    name: &str,
    vm: &VmConfig,
    runtime: &Value,
    nets: &Value,
    socket_path: &Path,
    warnings: &mut Vec<Warning>,
    health: &mut Health,
) -> Value {
    let runtime_id = format!("{}/{}", environment.spec.project, name);
    let record = runtime
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["vm_id"] == runtime_id);
    let state = record
        .and_then(|item| item["state"].as_str())
        .unwrap_or("stopped");
    let pid = record
        .and_then(|item| item.get("pid").cloned())
        .unwrap_or(Value::Null);
    let ips = attachment_ips(nets, &runtime_id);
    let mut agent = json!({ "state": "unavailable" });
    let mut stats = Value::Null;
    let mut probes = Vec::new();

    if state != "running" {
        warnings.push(Warning {
            code: VM_NOT_RUNNING,
            severity: Health::Critical,
            message: format!("{runtime_id} is {state}"),
            vm_id: Some(runtime_id.clone()),
            hint: format!("vzctl vm start {runtime_id}"),
        });
        health.raise(Health::Critical);
    } else {
        match crate::vm::supervisor_rpc_deadline(
            socket_path,
            "vm.agent.health",
            json!({ "vm_id": runtime_id }),
            3,
        )
        {
            Ok(detail) => {
                let status = detail["status"].as_str().unwrap_or("unknown");
                agent = json!({
                    "state": if status == "down" { "unavailable" } else { "ready" },
                    "health": status,
                    "health_detail": detail,
                });
                match status {
                    "ok" => {}
                    "degraded" => {
                        push_warning(
                            warnings,
                            health,
                            AGENT_DEGRADED,
                            Health::Degraded,
                            format!("{runtime_id} agent is degraded"),
                            Some(runtime_id.clone()),
                            "check vm stats / exec backlog; vzctl vm agent upgrade",
                        );
                    }
                    _ => {
                        push_warning(
                            warnings,
                            health,
                            AGENT_NOT_READY,
                            Health::Critical,
                            format!("{runtime_id} agent health is {status}"),
                            Some(runtime_id.clone()),
                            &format!("vzctl vm inspect {runtime_id}"),
                        );
                    }
                }
            }
            Err((_, message)) if message.to_ascii_lowercase().contains("timeout") => {
                push_warning(
                    warnings,
                    health,
                    AGENT_EXEC_TIMEOUT,
                    Health::Degraded,
                    format!("{runtime_id} agent RPC timed out"),
                    Some(runtime_id.clone()),
                    "vzctl vm stats; inspect exec backlog",
                );
            }
            Err((_, message)) => {
                push_warning(
                    warnings,
                    health,
                    AGENT_NOT_READY,
                    Health::Critical,
                    format!("{runtime_id} agent unavailable: {message}"),
                    Some(runtime_id.clone()),
                    &format!("vzctl vm inspect {runtime_id}"),
                );
            }
        }
        if agent["state"] == "ready" {
            stats = crate::vm::supervisor_rpc_deadline(
                socket_path,
                "vm.agent.stats",
                json!({ "vm_id": runtime_id }),
                3,
            )
            .unwrap_or(Value::Null);
            probes.extend(builtin_peer_probes(
                environment,
                name,
                vm,
                &runtime_id,
                socket_path,
                warnings,
                health,
            ));
            if !vm.mounts.is_empty() {
                if let Ok(mounts) = crate::vm::supervisor_rpc_deadline(
                    socket_path,
                    "vm.mount.list",
                    json!({ "vm_id": runtime_id }),
                    3,
                ) {
                    let present = mounts["mounts"].as_array().map(Vec::len).unwrap_or(0);
                    if present < vm.mounts.len() {
                        push_warning(
                            warnings,
                            health,
                            MOUNT_MISSING,
                            Health::Degraded,
                            format!("{runtime_id} is missing virtiofs mounts"),
                            Some(runtime_id.clone()),
                            &format!("vzctl vm mounts {runtime_id}"),
                        );
                    }
                }
            }
        }
    }

    json!({
        "id": runtime_id,
        "name": name,
        "state": state,
        "pid": pid,
        "roles": vm.roles,
        "ips": ips,
        "agent": agent,
        "stats": stats,
        "probes": probes,
    })
}

fn push_warning(
    warnings: &mut Vec<Warning>,
    health: &mut Health,
    code: &'static str,
    severity: Health,
    message: String,
    vm_id: Option<String>,
    hint: &str,
) {
    warnings.push(Warning {
        code,
        severity,
        message,
        vm_id,
        hint: hint.to_string(),
    });
    health.raise(severity);
}

fn attachment_ips(nets: &Value, vm_id: &str) -> Vec<String> {
    nets["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["vm_id"] == vm_id)
        .filter_map(|item| item["ip"].as_str().map(str::to_string))
        .collect()
}

fn builtin_peer_probes(
    environment: &Environment,
    name: &str,
    vm: &VmConfig,
    runtime_id: &str,
    socket_path: &Path,
    warnings: &mut Vec<Warning>,
    health: &mut Health,
) -> Vec<Value> {
    let Some((peer_name, peer_ip, net_name)) = first_peer(environment, name, vm) else {
        return Vec::new();
    };
    let fqdn = format!("{peer_name}.{net_name}.{}.vz.test", environment.spec.project);
    let target = format!("{fqdn}:22");
    match crate::vm::supervisor_rpc_deadline(
        socket_path,
        "vm.agent.network_probe",
        json!({
            "vm_id": runtime_id,
            "target": target,
            "via": "both",
            "connect_ip": peer_ip,
            "timeout_ms": PROBE_TIMEOUT.as_millis() as u64,
        }),
        4,
    ) {
        Ok(probe) => {
            record_dns_split(&probe, runtime_id, &fqdn, &peer_ip, warnings, health);
            vec![json!({ "name": "peer-ssh", "target": target, "result": probe })]
        }
        Err((_, message)) => vec![json!({
            "name": "peer-ssh",
            "target": target,
            "error": message,
        })],
    }
}

fn first_peer(
    environment: &Environment,
    name: &str,
    vm: &VmConfig,
) -> Option<(String, String, String)> {
    let net_name = vm.networks.first()?.name.clone();
    environment.spec.vms.iter().find_map(|(other, spec)| {
        if other == name {
            return None;
        }
        let ip = spec
            .networks
            .iter()
            .find(|net| net.name == net_name)
            .map(|net| net.ip.clone())?;
        Some((other.clone(), ip, net_name.clone()))
    })
}

fn record_dns_split(
    probe: &Value,
    runtime_id: &str,
    fqdn: &str,
    ip: &str,
    warnings: &mut Vec<Warning>,
    health: &mut Health,
) {
    if probe["dns"]["ok"] != true && probe["ip"]["ok"] == true {
        push_warning(
            warnings,
            health,
            DNS_GUEST_PROBE_FAIL,
            Health::Degraded,
            format!("{runtime_id} DNS probe to {fqdn} failed; IP {ip} ok"),
            Some(runtime_id.to_string()),
            "check guest nameserver .0:53 and vzctl dns status",
        );
    }
}

fn apply_yaml_probes(
    environment: &Environment,
    socket_path: &Path,
    vms: &mut [Value],
    warnings: &mut Vec<Warning>,
    health: &mut Health,
) {
    let project = &environment.spec.project;
    for probe in &environment.spec.observability.probes {
        let result = run_declared_probe(environment, socket_path, probe);
        if probe.from != "host" {
            let runtime_id = format!("{project}/{}", probe.from);
            if let Some(vm) = vms.iter_mut().find(|vm| vm["id"] == runtime_id) {
                if let Some(list) = vm["probes"].as_array_mut() {
                    list.push(json!({
                        "name": probe.name,
                        "target": probe.target,
                        "expect": expect_name(probe.expect),
                        "result": result,
                    }));
                }
            }
        }
        if result["dns"]["ok"] == false && result["ip"]["ok"] == true {
            push_warning(
                warnings,
                health,
                DNS_GUEST_PROBE_FAIL,
                Health::Degraded,
                format!("probe {} DNS failed; IP ok", probe.name),
                (probe.from != "host").then(|| format!("{project}/{}", probe.from)),
                "guest FQDN lookup/forward is broken",
            );
        } else if result["ok"] == false {
            let code = if probe.from == "host" {
                DNS_HOST_PROBE_FAIL
            } else {
                DNS_GUEST_PROBE_FAIL
            };
            push_warning(
                warnings,
                health,
                code,
                Health::Degraded,
                format!("probe {} failed", probe.name),
                (probe.from != "host").then(|| format!("{project}/{}", probe.from)),
                "check target reachability and vzctl vm probe",
            );
        }
    }
}

fn expect_name(expect: ProbeExpect) -> &'static str {
    match expect {
        ProbeExpect::Tcp => "tcp",
        ProbeExpect::Http2xx => "http_2xx",
        ProbeExpect::Dns => "dns",
    }
}

fn run_declared_probe(
    environment: &Environment,
    socket_path: &Path,
    probe: &ObservabilityProbe,
) -> Value {
    match (probe.expect, probe.from.as_str()) {
        (ProbeExpect::Tcp, "host") => host_tcp_probe(&probe.target),
        (ProbeExpect::Http2xx, "host") => host_tcp_probe(&http_socket_target(&probe.target)),
        (ProbeExpect::Dns, "host") => host_dns_probe(&probe.target),
        (ProbeExpect::Tcp | ProbeExpect::Dns, _) => {
            guest_connect_probe(environment, socket_path, &probe.from, &probe.target)
        }
        (ProbeExpect::Http2xx, _) => json!({
            "ok": false,
            "error": "http_2xx from a VM is not implemented; use from: host",
        }),
    }
}

fn guest_connect_probe(
    environment: &Environment,
    socket_path: &Path,
    from: &str,
    raw_target: &str,
) -> Value {
    let runtime_id = format!("{}/{}", environment.spec.project, from);
    let target = if raw_target.contains(':') {
        raw_target.to_string()
    } else {
        format!("{raw_target}:22")
    };
    let hostname = target.rsplit_once(':').map(|(host, _)| host).unwrap_or(&target);
    let mut params = json!({
        "vm_id": runtime_id,
        "target": target,
        "via": "both",
        "timeout_ms": PROBE_TIMEOUT.as_millis() as u64,
    });
    if let Ok(ips) = crate::dns::lookup_a_addresses(hostname) {
        if let Some(ip) = ips.into_iter().next() {
            params["connect_ip"] = json!(ip);
        }
    }
    match crate::vm::supervisor_rpc_deadline(socket_path, "vm.agent.network_probe", params, 4) {
        Ok(mut value) => {
            let ok = value["dns"]["ok"] == true || value["ip"]["ok"] == true;
            value["ok"] = json!(ok);
            value
        }
        Err((_, message)) => json!({ "ok": false, "error": message }),
    }
}

fn http_socket_target(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if hostport.contains(':') {
        hostport.to_string()
    } else if url.starts_with("https://") {
        format!("{hostport}:443")
    } else {
        format!("{hostport}:80")
    }
}

fn host_tcp_probe(target: &str) -> Value {
    let ok = target
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .and_then(|addr| TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).ok())
        .is_some();
    json!({ "ok": ok, "target": target, "from": "host" })
}

fn host_dns_probe(name: &str) -> Value {
    match crate::dns::lookup_a_addresses(name) {
        Ok(ips) if !ips.is_empty() => json!({ "ok": true, "resolved_ips": ips }),
        Ok(_) => json!({ "ok": false, "error": "no answers" }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

pub(crate) fn dns_sections(dns: &Value) -> Value {
    let listeners = dns["listeners"].as_array().cloned().unwrap_or_default();
    let host_ok = listeners.iter().any(|item| {
        item.as_str()
            .is_some_and(|value| value.contains("127.0.0.1:") || value.ends_with(":15353"))
    }) && dns["ok"].as_bool() != Some(false);
    let bridge_ok = listeners.iter().any(|item| {
        item.as_str()
            .is_some_and(|value| value.contains(":53") && !value.contains(":15353"))
    });
    json!({
        "ok": dns.get("ok").cloned().unwrap_or(Value::Null),
        "host_resolver": { "ok": host_ok, "endpoint": "127.0.0.1:15353" },
        "bridge_dns": { "ok": bridge_ok, "listeners": listeners },
        "upstream": { "name": dns["upstream"], "ok": dns["upstream"].is_string() },
        "last_probe": dns.get("last_error").cloned().unwrap_or(Value::Null),
        "raw": dns,
    })
}

fn route_snapshot(
    environment: &Environment,
    socket_path: &Path,
    warnings: &mut Vec<Warning>,
    health: &mut Health,
) -> Value {
    let has_router = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "router"));
    if !has_router {
        return json!({ "routers": [] });
    }
    match crate::vm::supervisor_rpc_deadline(
        socket_path,
        "route.status",
        json!({ "policies": [] }),
        3,
    ) {
        Ok(status) => {
            for router in status["routers"].as_array().into_iter().flatten() {
                let rules = router["rules"].as_array().map(Vec::len).unwrap_or(0);
                let active = router["active"].as_bool().unwrap_or(false);
                if !active || rules == 0 {
                    push_warning(
                        warnings,
                        health,
                        ROUTE_STATUS_AMBIGUOUS,
                        Health::Degraded,
                        "route status has no extra allow rules; policy-drop may still be active".into(),
                        router["vm_id"].as_str().map(str::to_string),
                        "empty allow-list is valid for deny-all; use route status --format json",
                    );
                }
            }
            status
        }
        Err((_, message)) => json!({ "error": message }),
    }
}

fn ingress_snapshot(
    environment: &Environment,
    warnings: &mut Vec<Warning>,
    health: &mut Health,
) -> Value {
    let Some(ingress) = environment.spec.ingress.as_ref() else {
        return json!({ "enabled": false });
    };
    if !ingress.enabled {
        return json!({ "enabled": false });
    }
    let bind = format!("{}:{}", ingress.bind, ingress.https_port);
    let reachable = bind
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .and_then(|addr| TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).ok())
        .is_some();
    if !reachable {
        push_warning(
            warnings,
            health,
            INGRESS_UNREACHABLE,
            Health::Degraded,
            format!("host cannot reach ingress {bind}"),
            None,
            "vzctl services status; check spec.ingress.bind",
        );
    }
    json!({ "enabled": true, "bind": bind, "ok": reachable })
}

fn format_issues(path: &Path, issues: &[ValidationIssue]) -> String {
    let details = issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path, issue.message))
        .collect::<Vec<_>>()
        .join("; ");
    format!("invalid {}: {details}", path.display())
}

fn print_human(envelope: &Value) {
    println!(
        "{}",
        envelope["summary"]["message"]
            .as_str()
            .unwrap_or("stack status")
    );
    for vm in envelope["vms"].as_array().into_iter().flatten() {
        println!(
            "  {:<24} {:<10} agent={} ips={}",
            vm["id"].as_str().unwrap_or("?"),
            vm["state"].as_str().unwrap_or("?"),
            vm["agent"]["health"].as_str().unwrap_or("-"),
            vm["ips"]
                .as_array()
                .map(|ips| ips
                    .iter()
                    .filter_map(|ip| ip.as_str())
                    .collect::<Vec<_>>()
                    .join(","))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".into())
        );
    }
    for warning in envelope["warnings"].as_array().into_iter().flatten() {
        eprintln!(
            "  [{}] {}",
            warning["code"].as_str().unwrap_or("?"),
            warning["message"].as_str().unwrap_or("")
        );
        if let Some(hint) = warning["hint"].as_str().filter(|value| !value.is_empty()) {
            eprintln!("    hint: {hint}");
        }
    }
}

pub(crate) fn doctor_stack_checks(directory: &Path, socket_path: &Path) -> Vec<crate::Check> {
    match collect_status(directory, socket_path) {
        Ok(report) if report.warnings.is_empty() => vec![crate::Check {
            id: "stack.status".into(),
            status: crate::CheckStatus::Ok,
            message: format!("stack {} ok", report.project),
            details: json!({ "health": "ok" }),
        }],
        Ok(report) => report
            .warnings
            .iter()
            .map(|warning| crate::Check {
                id: match warning.code {
                    DNS_GUEST_PROBE_FAIL => "stack.dns_guest_probe_fail",
                    DNS_HOST_PROBE_FAIL => "stack.dns_host_probe_fail",
                    AGENT_DEGRADED => "stack.agent_degraded",
                    AGENT_EXEC_TIMEOUT => "stack.agent_exec_timeout",
                    VM_NOT_RUNNING => "stack.vm_not_running",
                    AGENT_NOT_READY => "stack.agent_not_ready",
                    ROUTE_STATUS_AMBIGUOUS => "stack.route_status_ambiguous",
                    INGRESS_UNREACHABLE => "stack.ingress_unreachable",
                    MOUNT_MISSING => "stack.mount_missing",
                    _ => "stack.warning",
                },
                status: match warning.severity {
                    Health::Ok => crate::CheckStatus::Ok,
                    Health::Degraded => crate::CheckStatus::Warn,
                    Health::Critical => crate::CheckStatus::Fail,
                },
                message: format!("{} — {}", warning.message, warning.hint),
                details: warning.json(),
            })
            .collect(),
        Err((message, _)) => vec![crate::Check {
            id: "stack.status".into(),
            status: crate::CheckStatus::Fail,
            message,
            details: json!({}),
        }],
    }
}

pub(crate) fn stack_watch_command(args: &[String], socket_path: &Path) -> ExitCode {
    let mut directory = PathBuf::from(".");
    let mut filter = None;
    let mut interval = 5_u64;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("-C requires a path");
                    return ExitCode::from(EXIT_USAGE);
                };
                directory = PathBuf::from(value);
                index += 2;
            }
            "--filter" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("--filter requires a glob");
                    return ExitCode::from(EXIT_USAGE);
                };
                filter = Some(value.clone());
                index += 2;
            }
            "--interval" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("--interval requires seconds");
                    return ExitCode::from(EXIT_USAGE);
                };
                match value.parse::<u64>() {
                    Ok(parsed) if parsed > 0 => interval = parsed,
                    _ => {
                        eprintln!("--interval must be a positive integer");
                        return ExitCode::from(EXIT_INVALID);
                    }
                }
                index += 2;
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: vzctl stack watch [-C dir] [--filter glob] [--interval sec]"
                );
                return ExitCode::from(EXIT_USAGE);
            }
            other => {
                eprintln!("unknown stack watch option: {other}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }
    run_watch(&directory, socket_path, filter.as_deref(), interval)
}

fn run_watch(
    directory: &Path,
    socket_path: &Path,
    filter: Option<&str>,
    interval: u64,
) -> ExitCode {
    use std::io::{self, BufRead, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let _ = ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst));
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut last_codes: Vec<String> = Vec::new();
    let mut next_tick = Instant::now();
    let mut events = subscribe_events(socket_path);

    while !stop.load(Ordering::SeqCst) {
        if Instant::now() >= next_tick {
            if let Ok(report) = collect_status(directory, socket_path) {
                let ts = chrono_like_now();
                for warning in &report.warnings {
                    let event_type = format!("probe.{}", warning.code.to_ascii_lowercase());
                    if watch_match(filter, &event_type)
                        && !last_codes.iter().any(|code| code == warning.code)
                    {
                        let _ = writeln!(
                            out,
                            "{}",
                            json!({
                                "v": 1,
                                "type": event_type,
                                "ts": ts,
                                "vm_id": warning.vm_id,
                                "code": warning.code,
                                "message": warning.message,
                            })
                        );
                    }
                }
                last_codes = report
                    .warnings
                    .iter()
                    .map(|warning| warning.code.to_string())
                    .collect();
                if watch_match(filter, "stack.status") {
                    let _ = writeln!(
                        out,
                        "{}",
                        json!({
                            "v": 1,
                            "type": "stack.status",
                            "ts": ts,
                            "code": report.health.as_str(),
                            "message": format!("stack {} {}", report.project, report.health.as_str()),
                            "warnings": report.warnings.iter().map(Warning::json).collect::<Vec<_>>(),
                        })
                    );
                }
                let _ = out.flush();
            }
            next_tick = Instant::now() + Duration::from_secs(interval);
        }

        if let Some(reader) = events.as_mut() {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => events = None,
                Ok(_) => {
                    if let Ok(event) = serde_json::from_str::<Value>(&line) {
                        let event_type = event["type"].as_str().unwrap_or("");
                        if watch_match(filter, event_type) {
                            let _ = out.write_all(line.as_bytes());
                            let _ = out.flush();
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => events = None,
            }
        } else {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    ExitCode::SUCCESS
}

fn subscribe_events(
    socket_path: &Path,
) -> Option<std::io::BufReader<std::os::unix::net::UnixStream>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": "events.subscribe",
        "params": { "filter": Value::Null },
        "id": 1
    });
    writeln!(stream, "{request}").ok()?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).ok()?;
    let parsed: Value = serde_json::from_str(&response).ok()?;
    if parsed["result"]["ok"] != true {
        return None;
    }
    reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(250)))
        .ok()?;
    Some(reader)
}

fn watch_match(filter: Option<&str>, event_type: &str) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    filter.split(',').map(str::trim).any(|pattern| {
        if let Some(prefix) = pattern.strip_suffix('*') {
            event_type.starts_with(prefix)
        } else {
            event_type == pattern
        }
    })
}

fn chrono_like_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", now.as_secs(), now.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_rank_raises_to_critical() {
        let mut health = Health::Ok;
        health.raise(Health::Degraded);
        health.raise(Health::Critical);
        assert_eq!(health, Health::Critical);
        assert_eq!(health.exit(), 2);
    }

    #[test]
    fn dns_sections_split_host_and_bridge() {
        let sections = dns_sections(&json!({
            "ok": true,
            "listeners": ["127.0.0.1:15353", "10.90.0.0:53"],
            "upstream": "system",
            "last_error": Value::Null
        }));
        assert_eq!(sections["host_resolver"]["ok"], true);
        assert_eq!(sections["bridge_dns"]["ok"], true);
        assert_eq!(sections["upstream"]["name"], "system");
    }

    #[test]
    fn watch_filter_matches_prefix() {
        assert!(watch_match(Some("probe.*,stack.status"), "probe.dns_guest_probe_fail"));
        assert!(!watch_match(Some("vm.*"), "stack.status"));
    }
}
