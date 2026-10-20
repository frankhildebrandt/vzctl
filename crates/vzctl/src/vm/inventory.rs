use super::*;

pub(super) fn list_vms(command: &str, socket_path: &Path) -> Result<Value, Failure> {
    let mut by_id = scan_bundles()?;
    let mut warnings = Vec::new();
    match rpc(socket_path, "vm.list", json!({})) {
        Ok(records) => {
            for record in records.as_array().into_iter().flatten() {
                let Some(id) = record["vm_id"].as_str() else {
                    continue;
                };
                let entry = by_id.entry(id.to_string()).or_insert_with(|| {
                    json!({
                        "id": id,
                        "state": "stopped",
                        "pid": Value::Null,
                        "bundle": record["bundle"].clone(),
                        "managed-by": Value::Null,
                        "roles": [],
                        "ips": [],
                        "networks": [],
                    })
                });
                if let Some(state) = record["state"].as_str() {
                    entry["state"] = json!(state);
                }
                if let Some(pid) = record["pid"].as_u64() {
                    entry["pid"] = json!(pid);
                }
                if let Some(bundle) = record["bundle"].as_str() {
                    entry["bundle"] = json!(bundle);
                }
                if let Some(updated) = record["updated_at"].as_str() {
                    entry["updated_at"] = json!(updated);
                }
            }
        }
        Err(failure) if failure.code == EXIT_SUPERVISOR => {
            warnings.push(failure.message);
        }
        Err(failure) => return Err(failure),
    }

    let attachment_networks = match rpc(socket_path, "net.list", json!({})) {
        Ok(snapshot) => {
            let mut map: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
                let Some(vm_id) = attachment["vm_id"].as_str() else {
                    continue;
                };
                let Some(ip) = attachment["ip"].as_str() else {
                    continue;
                };
                let network = attachment["network"].as_str().unwrap_or("");
                map.entry(vm_id.to_string()).or_default().push(json!({
                    "name": network,
                    "ip": ip,
                }));
            }
            map
        }
        Err(failure) if failure.code == EXIT_SUPERVISOR => {
            if !warnings.iter().any(|message| message == &failure.message) {
                warnings.push(failure.message);
            }
            BTreeMap::new()
        }
        Err(failure) => return Err(map_network_failure(failure)),
    };

    for (id, entry) in by_id.iter_mut() {
        let networks = if let Some(networks) = attachment_networks.get(id) {
            networks.clone()
        } else {
            fallback_networks_from_entry(entry)
        };
        let ips = networks
            .iter()
            .filter_map(|network| network["ip"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        entry["networks"] = json!(networks);
        entry["ips"] = json!(ips);
        if let Some(object) = entry.as_object_mut() {
            object.remove("identity");
        }
    }

    let vms = by_id.into_values().collect::<Vec<_>>();
    let running = vms
        .iter()
        .filter(|vm| matches!(vm["state"].as_str(), Some("starting" | "running")))
        .count();
    let status = if warnings.is_empty() { "ok" } else { "warn" };
    let message = if warnings.is_empty() {
        format!("{} VM(s)", vms.len())
    } else {
        format!("{} VM(s); supervisor unavailable", vms.len())
    };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": command,
        "status": status,
        "exit_code": 0,
        "summary": {
            "message": message,
            "vms": vms.len(),
            "running": running,
            "warnings": warnings.len(),
        },
        "vms": vms,
        "warnings": warnings,
    }))
}

pub(super) fn fallback_networks_from_entry(entry: &Value) -> Vec<Value> {
    let Some(nics) = entry.get("identity").and_then(|value| value.get("nics")) else {
        return Vec::new();
    };
    nics.as_array()
        .into_iter()
        .flatten()
        .filter_map(|nic| {
            let address = nic["address"].as_str()?;
            if address.is_empty() || address == "dhcp" {
                return None;
            }
            Some(json!({
                "name": nic.get("network").cloned().unwrap_or(Value::Null),
                "ip": address,
            }))
        })
        .collect()
}

pub(super) fn inspect_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let manifest = read_manifest(&bundle)?;
    let mut warnings = Vec::new();
    let mut state = json!("stopped");
    let mut pid = Value::Null;
    let mut updated_at = Value::Null;
    match rpc(socket_path, "vm.list", json!({})) {
        Ok(records) => {
            if let Some(record) = records
                .as_array()
                .into_iter()
                .flatten()
                .find(|record| record["vm_id"] == id)
            {
                if let Some(value) = record.get("state") {
                    state = value.clone();
                }
                if let Some(value) = record.get("pid") {
                    pid = value.clone();
                }
                if let Some(value) = record.get("updated_at") {
                    updated_at = value.clone();
                }
            }
        }
        Err(failure) if failure.code == EXIT_SUPERVISOR => warnings.push(failure.message),
        Err(failure) => return Err(failure),
    }

    let mut networks = Vec::new();
    match rpc(socket_path, "net.list", json!({})) {
        Ok(snapshot) => {
            for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
                if attachment["vm_id"] == id {
                    networks.push(json!({
                        "name": attachment.get("network").cloned().unwrap_or(Value::Null),
                        "ip": attachment.get("ip").cloned().unwrap_or(Value::Null),
                        "cidr": attachment.get("cidr").cloned().unwrap_or(Value::Null),
                    }));
                }
            }
        }
        Err(failure) if failure.code == EXIT_SUPERVISOR => {
            if !warnings.iter().any(|message| message == &failure.message) {
                warnings.push(failure.message);
            }
        }
        Err(failure) => return Err(map_network_failure(failure)),
    }
    if networks.is_empty() {
        networks = fallback_networks_from_manifest(&manifest);
    }

    let mut agent = json!({
        "state": "unavailable",
    });
    if state == "running" {
        match rpc(socket_path, "vm.agent.health", json!({ "vm_id": id })) {
            Ok(health) => {
                let version = rpc(socket_path, "vm.agent.version", json!({ "vm_id": id })).ok();
                let report_ip = rpc(socket_path, "vm.agent.report_ip", json!({ "vm_id": id })).ok();
                agent = json!({
                    "state": "ready",
                    "health": health.get("status").cloned().unwrap_or(Value::Null),
                    "health_detail": health,
                    "version": version
                        .as_ref()
                        .and_then(|value| value.get("agent_version"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "capabilities": version
                        .as_ref()
                        .and_then(|value| value.get("capabilities"))
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                    "interfaces": report_ip
                        .as_ref()
                        .and_then(|value| value.get("interfaces"))
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                });
            }
            Err(failure) => {
                agent = json!({
                    "state": "unavailable",
                    "error": failure.message,
                });
                warnings.push(failure.message);
            }
        }
    }

    let status = if warnings.is_empty() { "ok" } else { "warn" };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.inspect",
        "status": status,
        "exit_code": 0,
        "summary": {
            "message": format!("VM {id}"),
            "vm_id": id,
            "state": state,
            "warnings": warnings.len(),
        },
        "vm": {
            "id": id,
            "state": state,
            "pid": pid,
            "bundle": bundle,
            "managed-by": manifest.get("managed-by").cloned().unwrap_or(Value::Null),
            "roles": manifest.get("roles").cloned().unwrap_or_else(|| json!([])),
            "resources": manifest.get("resources").cloned().unwrap_or_else(|| {
                json!({ "cpus": 2, "memory_mib": 1024 })
            }),
            "updated_at": updated_at,
        },
        "identity": manifest.get("identity").cloned().unwrap_or(Value::Null),
        "disks": manifest.get("disks").cloned().unwrap_or(Value::Null),
        "networks": networks,
        "agent": agent,
        "logs": {
            "serial": serial_log_path(id).display().to_string(),
        },
        "warnings": warnings,
    }))
}

pub(super) fn maybe_hint_agent_timeout(id: &str, socket_path: &Path, failure: &Failure) {
    if failure.code != EXIT_GUEST {
        return;
    }
    let message = failure.message.to_ascii_lowercase();
    if !message.contains("timeout") && !message.contains("timed out") {
        return;
    }
    eprintln!("guest agent operation timed out (exit 18); fetching health/stats for {id}");
    match rpc(socket_path, "vm.agent.health", json!({ "vm_id": id })) {
        Ok(health) => {
            eprintln!(
                "  agent health={} queue_depth={} p99_exec_ms={}",
                health["status"].as_str().unwrap_or("?"),
                health["queue_depth"],
                health["p99_exec_ms"]
            );
        }
        Err(error) => eprintln!("  agent health unavailable: {}", error.message),
    }
    match rpc(socket_path, "vm.agent.stats", json!({ "vm_id": id })) {
        Ok(stats) => {
            eprintln!(
                "  agent stats cpu={} mem_used_pct={} load1={}",
                stats["cpu"]["percent"],
                stats
                    .get("mem_used_pct")
                    .cloned()
                    .unwrap_or_else(|| stats["memory"]["percent"].clone()),
                stats["load1"]
            );
        }
        Err(error) => eprintln!("  agent stats unavailable: {}", error.message),
    }
}

pub(super) fn probe_vm(
    id: &str,
    target: &str,
    via: ProbeVia,
    timeout_ms: u64,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let connect_ip = resolve_probe_connect_ip(target, via);
    let mut params = json!({
        "vm_id": id,
        "target": target,
        "via": via.as_str(),
        "timeout_ms": timeout_ms,
    });
    if let Some(ip) = connect_ip.as_ref() {
        params["connect_ip"] = json!(ip);
    }
    let probe = rpc(socket_path, "vm.agent.network_probe", params)?;
    let dns_ok = probe["dns"]["ok"].as_bool();
    let ip_ok = probe["ip"]["ok"].as_bool();
    let chosen_ok = probe["error_stage"].is_null() && probe["chosen_ip"].as_str().is_some();
    let (status, exit_code, message) = match via {
        ProbeVia::Both => match (dns_ok, ip_ok) {
            (Some(true), Some(true)) => ("ok", 0, "dns ok, ip ok"),
            (Some(false), Some(true)) => ("warn", 0, "dns FAIL, ip OK"),
            (Some(true), Some(false)) => ("warn", 0, "dns OK, ip FAIL"),
            _ => ("fail", EXIT_GUEST, "probe failed"),
        },
        ProbeVia::Dns => {
            if dns_ok == Some(true) || (dns_ok.is_none() && chosen_ok) {
                ("ok", 0, "dns ok")
            } else {
                ("fail", EXIT_GUEST, "dns probe failed")
            }
        }
        ProbeVia::Ip => {
            if ip_ok == Some(true) || (ip_ok.is_none() && chosen_ok) {
                ("ok", 0, "ip ok")
            } else {
                ("fail", EXIT_GUEST, "ip probe failed")
            }
        }
    };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.probe",
        "status": status,
        "exit_code": exit_code,
        "summary": {
            "message": format!("{id}: {message}"),
            "vm_id": id,
            "target": target,
            "via": via.as_str(),
        },
        "probe": probe,
    }))
}

pub(super) fn resolve_probe_connect_ip(target: &str, via: ProbeVia) -> Option<String> {
    if !matches!(via, ProbeVia::Ip | ProbeVia::Both) {
        return None;
    }
    let host = target.rsplit_once(':').map(|(host, _)| host)?;
    let host = host.trim_matches(['[', ']']);
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Some(host.to_string());
    }
    crate::dns::lookup_a_addresses(host)
        .ok()
        .and_then(|ips| ips.into_iter().next())
}

pub(super) fn health_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let health = rpc(socket_path, "vm.agent.health", json!({ "vm_id": id }))?;
    let status = health["status"].as_str().unwrap_or("unknown");
    let (envelope_status, exit_code) = match status {
        "ok" => ("ok", 0),
        "degraded" => ("warn", 0),
        _ => ("fail", EXIT_GUEST),
    };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.health",
        "status": envelope_status,
        "exit_code": exit_code,
        "summary": {
            "message": format!("{id} health {status}"),
            "vm_id": id,
            "health": status,
        },
        "health": health,
    }))
}

pub(super) fn stats_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let stats = rpc(socket_path, "vm.agent.stats", json!({ "vm_id": id }))?;
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.stats",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("{id} stats"),
            "vm_id": id,
        },
        "stats": stats,
    }))
}

pub(super) fn fallback_networks_from_manifest(manifest: &Value) -> Vec<Value> {
    fallback_networks_from_entry(&json!({ "identity": manifest.get("identity") }))
}

pub(super) fn scan_bundles() -> Result<BTreeMap<String, Value>, Failure> {
    let root = crate::state_dir().join("vms");
    let mut vms = BTreeMap::new();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vms),
        Err(error) => {
            return Err(Failure::new(
                EXIT_VM_DISK,
                format!("read {}: {error}", root.display()),
            ));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            Failure::new(EXIT_VM_DISK, format!("read {}: {error}", root.display()))
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let top_name = entry.file_name().to_string_lossy().to_string();
        let manifest_path = path.join("vm.json");
        if manifest_path.is_file() {
            insert_bundle_entry(&mut vms, &top_name, &path, &manifest_path)?;
            continue;
        }
        // Nested project/vm bundles: vms/{project}/{vm}/vm.json
        let children = match fs::read_dir(&path) {
            Ok(children) => children,
            Err(error) => {
                return Err(Failure::new(
                    EXIT_VM_DISK,
                    format!("read {}: {error}", path.display()),
                ));
            }
        };
        for child in children {
            let child = child.map_err(|error| {
                Failure::new(EXIT_VM_DISK, format!("read {}: {error}", path.display()))
            })?;
            let child_path = child.path();
            if !child_path.is_dir() {
                continue;
            }
            let nested_manifest = child_path.join("vm.json");
            if !nested_manifest.is_file() {
                continue;
            }
            let child_name = child.file_name().to_string_lossy().to_string();
            let id = format!("{top_name}/{child_name}");
            insert_bundle_entry(&mut vms, &id, &child_path, &nested_manifest)?;
        }
    }
    Ok(vms)
}

pub(super) fn insert_bundle_entry(
    vms: &mut BTreeMap<String, Value>,
    fallback_id: &str,
    path: &Path,
    manifest_path: &Path,
) -> Result<(), Failure> {
    let manifest = read_manifest_file(manifest_path)?;
    let id = manifest["vm_id"]
        .as_str()
        .unwrap_or(fallback_id)
        .to_string();
    let mut entry = json!({
        "id": id,
        "state": "stopped",
        "pid": Value::Null,
        "bundle": path,
        "managed-by": manifest.get("managed-by").cloned().unwrap_or(Value::Null),
        "roles": manifest.get("roles").cloned().unwrap_or_else(|| json!([])),
        "ips": [],
        "networks": [],
        "identity": manifest.get("identity").cloned().unwrap_or(Value::Null),
    });
    let networks = fallback_networks_from_entry(&entry);
    let ips = networks
        .iter()
        .filter_map(|network| network["ip"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    entry["networks"] = json!(networks);
    entry["ips"] = json!(ips);
    vms.insert(id, entry);
    Ok(())
}

pub(super) fn bundle_path(id: &str) -> PathBuf {
    crate::state_dir().join("vms").join(id)
}

pub(super) fn helper_is_running(vm_id: &str, socket_path: &Path) -> Result<bool, Failure> {
    match rpc(socket_path, "vm.list", json!({})) {
        Ok(records) => Ok(records.as_array().into_iter().flatten().any(|record| {
            // Live mount/exec need helper.state == running — "starting" is too early.
            record["vm_id"] == vm_id && record["state"].as_str() == Some("running")
        })),
        Err(failure) if failure.code == EXIT_SUPERVISOR => Ok(false),
        Err(failure) => Err(failure),
    }
}

pub(super) fn read_manifest(bundle: &Path) -> Result<Value, Failure> {
    let manifest = bundle.join("vm.json");
    if !manifest.is_file() {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("VM bundle not found: {}", bundle.display()),
        ));
    }
    read_manifest_file(&manifest)
}

pub(super) fn read_manifest_file(path: &Path) -> Result<Value, Failure> {
    let bytes = fs::read(path)
        .map_err(|error| Failure::new(EXIT_VM_DISK, format!("read {}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| Failure::new(EXIT_VM_DISK, format!("parse {}: {error}", path.display())))
}
