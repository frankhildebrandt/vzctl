use super::*;

pub(super) fn start_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let manifest = read_manifest(&bundle)?;
    let result = rpc(
        socket_path,
        "vm.start",
        json!({ "vm_id": id, "bundle": bundle }),
    )?;
    let state = result["state"].as_str().unwrap_or("starting");
    let pid = result.get("pid").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.start",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("VM {id} {state}"),
            "vm_id": id,
            "state": state,
        },
        "vm": {
            "id": id,
            "state": state,
            "pid": pid,
            "bundle": bundle,
            "managed-by": manifest.get("managed-by").cloned().unwrap_or(Value::Null),
            "roles": manifest.get("roles").cloned().unwrap_or_else(|| json!([])),
        },
    }))
}

pub(super) fn stop_vm(id: &str, wait: bool, socket_path: &Path) -> Result<Value, Failure> {
    rpc(socket_path, "vm.stop", json!({ "vm_id": id }))?;
    if wait {
        wait_stopped(id, socket_path)?;
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.stop",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("VM {id} stopped"),
            "vm_id": id,
            "state": "stopped",
        },
        "vm": {
            "id": id,
            "state": "stopped",
        },
    }))
}

pub(super) fn restart_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    stop_vm(id, true, socket_path)?;
    let mut envelope = start_vm(id, socket_path)?;
    envelope["command"] = json!("vm.restart");
    if let Some(summary) = envelope.get_mut("summary").and_then(Value::as_object_mut) {
        let state = summary
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("starting");
        summary.insert(
            "message".into(),
            json!(format!("VM {id} restarted ({state})")),
        );
    }
    Ok(envelope)
}

pub(super) fn delete_vm(id: &str, force: bool, socket_path: &Path) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let manifest = match read_manifest(&bundle) {
        Ok(manifest) => Some(manifest),
        Err(failure) if force && failure.code == EXIT_INVALID => None,
        Err(failure) => return Err(failure),
    };
    if let Some(manifest) = &manifest {
        if manifest["managed-by"] != "vzctl" {
            return Err(Failure::new(
                EXIT_INVALID,
                format!(
                    "refusing to delete unmanaged VM bundle {}",
                    bundle.display()
                ),
            ));
        }
    }

    // Prefer supervisor purge (clears helper bookkeeping + SQLite attachments/ports).
    // Fall back to stop + detach when purge is unavailable (older supervisor).
    let mut detached = Vec::new();
    let mut ports_removed = 0u64;
    let mut purged_runtime = false;
    let purge_result = rpc(socket_path, "vm.purge", json!({ "vm_id": id }));
    match purge_result {
        Ok(result) => {
            purged_runtime = true;
            if let Some(networks) = result["detached_networks"].as_array() {
                for network in networks {
                    if let Some(name) = network.as_str() {
                        detached.push(name.to_string());
                    }
                }
            }
            ports_removed = result["ports_removed"].as_u64().unwrap_or(0);
        }
        Err(failure)
            if force
                || failure.message.contains("Method not found")
                || failure.message.contains("vm.purge") =>
        {
            match rpc(socket_path, "vm.stop", json!({ "vm_id": id })) {
                Ok(_) => {}
                Err(_) if force => {}
                Err(stop_failure) => return Err(stop_failure),
            }
            match wait_stopped(id, socket_path) {
                Ok(()) => {}
                Err(_) if force => {}
                Err(wait_failure) => return Err(wait_failure),
            }
            match rpc(socket_path, "net.list", json!({})) {
                Ok(snapshot) => {
                    for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
                        if attachment["vm_id"] == id {
                            let network = attachment
                                .get("network_id")
                                .unwrap_or(&attachment["network"])
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            match rpc(
                                socket_path,
                                "net.detach",
                                json!({ "vm_id": id, "network": network, "project": attachment["project"] }),
                            ) {
                                Ok(_) => detached.push(network),
                                Err(_) if force => {}
                                Err(detach_failure) => {
                                    return Err(map_network_failure(detach_failure))
                                }
                            }
                        }
                    }
                }
                Err(_) if force => {}
                Err(list_failure) => return Err(map_network_failure(list_failure)),
            }
        }
        Err(failure) => return Err(failure),
    }

    let purged = if bundle.is_dir() {
        fs::remove_dir_all(&bundle).map_err(|error| {
            Failure::new(EXIT_VM_DISK, format!("purge {}: {error}", bundle.display()))
        })?;
        true
    } else if manifest.is_some() {
        true
    } else {
        false
    };

    let message = if purged {
        format!("VM {id} deleted")
    } else {
        format!("VM {id} cleaned up (no bundle; runtime/DB purged)")
    };

    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.delete",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": message,
            "vm_id": id,
            "deleted": true,
            "purged_bundle": purged,
            "purged_runtime": purged_runtime,
            "detached": detached.len(),
            "ports_removed": ports_removed,
        },
        "vm": {
            "id": id,
            "deleted": true,
            "bundle": bundle,
            "purged_bundle": purged,
            "purged_runtime": purged_runtime,
            "detached_networks": detached,
            "ports_removed": ports_removed,
        },
    }))
}

pub(super) fn map_network_failure(failure: Failure) -> Failure {
    if failure.code == EXIT_VM_OP {
        Failure::new(EXIT_NETWORK, failure.message)
    } else {
        failure
    }
}

pub(super) fn modify_vm(
    id: &str,
    cpus: Option<u32>,
    memory_mib: Option<u64>,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let mut manifest = read_manifest(&bundle)?;
    let root = manifest
        .as_object_mut()
        .ok_or_else(|| Failure::new(EXIT_VM_DISK, "VM manifest is not an object"))?;
    let resources = root
        .entry("resources".to_string())
        .or_insert_with(|| json!({ "cpus": 2, "memory_mib": 1024 }));
    let resources_obj = resources
        .as_object_mut()
        .ok_or_else(|| Failure::new(EXIT_VM_DISK, "VM manifest resources is not an object"))?;
    if let Some(cpus) = cpus {
        resources_obj.insert("cpus".to_string(), json!(cpus));
    }
    if let Some(memory_mib) = memory_mib {
        resources_obj.insert("memory_mib".to_string(), json!(memory_mib));
    }
    let cpus_value = resources_obj
        .get("cpus")
        .and_then(Value::as_u64)
        .unwrap_or(2);
    let memory_value = resources_obj
        .get("memory_mib")
        .and_then(Value::as_u64)
        .unwrap_or(1024);
    let pretty = serde_json::to_string_pretty(&manifest).map_err(|error| {
        Failure::new(
            EXIT_VM_DISK,
            format!("cannot serialize VM manifest: {error}"),
        )
    })?;
    let path = bundle.join("vm.json");
    fs::write(&path, format!("{pretty}\n")).map_err(|error| {
        Failure::new(
            EXIT_VM_DISK,
            format!("cannot write {}: {error}", path.display()),
        )
    })?;

    let restart_required = helper_is_running(id, socket_path)?;
    let message = if restart_required {
        "resources updated (restart required)"
    } else {
        "resources updated (applied on next start)"
    };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.modify",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": message,
            "vm_id": id,
            "live": false,
            "restart_required": restart_required,
        },
        "vm": {
            "id": id,
            "bundle": bundle,
            "resources": {
                "cpus": cpus_value,
                "memory_mib": memory_value,
            },
        },
        "live": false,
        "restart_required": restart_required,
    }))
}

pub(super) fn agent_upgrade_vm(
    all: bool,
    id: Option<&str>,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let targets = if all {
        let records = rpc(socket_path, "vm.list", json!({}))?;
        records
            .as_array()
            .into_iter()
            .flatten()
            .filter(|record| record["state"].as_str() == Some("running"))
            .filter_map(|record| {
                record["vm_id"]
                    .as_str()
                    .or_else(|| record["id"].as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
    } else {
        vec![id.unwrap_or_default().to_string()]
    };
    if targets.is_empty() {
        return Err(Failure::new(
            EXIT_INVALID,
            "no running VMs selected for guest utils upgrade",
        ));
    }
    let bundle = crate::guest_utils::ensure_cached_bundle(&crate::state_dir())
        .map_err(|error| Failure::new(EXIT_UNAVAILABLE, error.message))?;
    let results = crate::guest_utils::rollout_targets(&targets, &bundle, &mut |method, params| {
        rpc(socket_path, method, params).map_err(|failure| failure.message)
    })
    .map_err(|error| Failure::new(EXIT_GUEST, error.message))?;
    let upgraded = results
        .iter()
        .filter(|result| result["status"].as_str() == Some("upgraded"))
        .count();
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.agent.upgrade",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if upgraded > 0 {
                format!("guest utils upgraded on {upgraded} VM(s)")
            } else {
                "guest utils already current".to_string()
            },
            "bundle_id": bundle.bundle_id,
            "agent_version": bundle.agent_version,
            "upgraded": upgraded,
            "total": results.len(),
        },
        "results": results,
    }))
}

pub(super) fn wait_stopped(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let records = rpc(socket_path, "vm.list", json!({}))?;
        let active = records.as_array().into_iter().flatten().any(|record| {
            record["vm_id"] == vm_id
                && matches!(record["state"].as_str(), Some("starting" | "running"))
        });
        if !active {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Failure::new(
                EXIT_VM_OP,
                format!("VM {vm_id} did not stop before timeout"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
