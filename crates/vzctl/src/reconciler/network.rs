use super::*;

fn network_changed(actual: &Value, desired: &config::NetworkConfig) -> bool {
    actual["cidr"] != desired.cidr
        || actual["mode"] != serde_json::to_value(desired.mode).unwrap_or(Value::Null)
        || actual["nat_egress"].as_bool().unwrap_or(true) != desired.nat_egress
        || actual["backend"].as_str().unwrap_or("vmnet")
            != match desired.backend {
                config::NetworkBackend::Vmnet => "vmnet",
                config::NetworkBackend::Docker => "docker",
            }
}

fn network_attachments<'a>(snapshot: &'a Value, network: &Value) -> Vec<&'a Value> {
    snapshot["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|attachment| match attachment["network_id"].as_str() {
            Some(id) => Some(id) == network.get("id").unwrap_or(&network["name"]).as_str(),
            // Old servers had globally unique names. Mixed/ambiguous records
            // are conservatively included so ownership validation fails closed.
            None => attachment["network"] == network["name"],
        })
        .collect()
}

pub(super) fn validate_network_ownership(
    environment: &Environment,
    snapshot: &Value,
) -> Result<(), Failure> {
    for (name, desired) in &environment.spec.networks {
        let desired_cidr = desired
            .cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|error| Failure::new(EXIT_INVALID, error.to_string()))?;
        for actual in snapshot["networks"].as_array().into_iter().flatten() {
            let owned = actual["name"] == *name && actual["project"] == environment.spec.project;
            if owned {
                if actual["stack"] != stack_id(environment) {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        format!(
                        "network {name} belongs to another stack; explicit adoption is required"
                    ),
                    ));
                }
                if network_changed(actual, desired) {
                    for attachment in network_attachments(snapshot, actual) {
                        if attachment["project"] != environment.spec.project
                            || attachment["stack"] != stack_id(environment)
                        {
                            return Err(Failure::new(EXIT_INVALID, format!(
                                "network {name} has foreign or unowned attachments; --force cannot detach them"
                            )));
                        }
                    }
                }
                continue;
            }
            if let Some(cidr) = actual["cidr"]
                .as_str()
                .and_then(|c| c.parse::<ipnet::Ipv4Net>().ok())
            {
                if desired_cidr.contains(&cidr.network()) || cidr.contains(&desired_cidr.network())
                {
                    return Err(Failure::new(EXIT_INVALID, format!(
                        "network {name} CIDR {} overlaps existing network {}/{} ({cidr}); --force cannot change another owner",
                        desired.cidr, actual["project"].as_str().unwrap_or("<global>"),
                        actual["name"].as_str().unwrap_or("<unknown>")
                    )));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_networks(
    environment: &Environment,
    force: bool,
    socket_path: &Path,
) -> Result<(), Failure> {
    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    validate_network_ownership(environment, &snapshot)?;
    let current = snapshot["networks"].as_array().cloned().unwrap_or_default();
    for (name, network) in &environment.spec.networks {
        let existing = current
            .iter()
            .find(|item| item["name"] == *name && item["project"] == environment.spec.project);
        if let Some(existing) = existing {
            if network_changed(existing, network) {
                if !force {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        format!("network {name} requires recreate; use --force"),
                    ));
                }
                for attachment in network_attachments(&snapshot, existing) {
                    rpc(
                        socket_path,
                        "net.detach",
                        json!({
                            "vm_id": attachment["vm_id"],
                            "network": existing.get("id").unwrap_or(&existing["name"]),
                            "project": environment.spec.project,
                        }),
                    )?;
                }
                rpc(
                    socket_path,
                    "net.delete",
                    json!({
                        "name": existing.get("id").unwrap_or(&existing["name"]),
                        "project": environment.spec.project,
                    }),
                )?;
            } else {
                continue;
            }
        }
        rpc(
            socket_path,
            "net.create",
            json!({
                "name": name,
                "cidr": network.cidr,
                "mode": "shared",
                "nat_egress": network.nat_egress,
                "backend": match network.backend {
                    config::NetworkBackend::Vmnet => "vmnet",
                    config::NetworkBackend::Docker => "docker",
                },
                "labels": {"managed-by": "vzctl"},
                "project": environment.spec.project,
                "stack": stack_id(environment),
            }),
        )?;
    }
    Ok(())
}

pub(super) fn ensure_dns(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    if environment.spec.dns.host_resolver {
        let path = config::config_path(config_path)
            .to_string_lossy()
            .into_owned();
        let mut args: Vec<String> = [
            "dns",
            "install-resolver",
            "--config",
            &path,
            "--format",
            "json",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        // Runtime CIDRs (incl. the default network) scope the reverse resolver
        // files; the elevated child cannot query the user's supervisor itself.
        for cidr in runtime_project_cidrs(socket_path, &environment.spec.project) {
            args.push("--cidr".into());
            args.push(cidr);
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_self_privileged(&arg_refs, progress).map(|_| ())?;
    } else {
        run_self_privileged(
            &[
                "dns",
                "uninstall-resolver",
                "--project",
                &environment.spec.project,
                "--format",
                "json",
            ],
            progress,
        )
        .map(|_| ())?;
    }
    let resilience = &environment.spec.resilience.network;
    rpc(
        socket_path,
        "resilience.ensure",
        json!({
            "project": environment.spec.project,
            "stack": stack_id(environment),
            "probe_enabled": resilience.egress_probe.enabled,
            "probe_url": resilience.egress_probe.url,
            "restart_vms": resilience.restart_vms_on_stuck_egress,
        }),
    )
    .map(|_| ())
}

/// Active network CIDRs owned by `project` plus unowned (default) networks.
/// Best effort: resolver install still proceeds when the supervisor is down.
pub(super) fn runtime_project_cidrs(socket_path: &Path, project: &str) -> Vec<String> {
    let Ok(snapshot) = rpc(socket_path, "net.list", json!({})) else {
        return Vec::new();
    };
    let mut cidrs: Vec<String> = snapshot["networks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|network| network["runtime_state"].as_str() == Some("active"))
        .filter(|network| match network["project"].as_str() {
            Some(owner) => owner == project,
            None => true,
        })
        .filter_map(|network| network["cidr"].as_str())
        .map(str::to_string)
        .collect();
    cidrs.sort();
    cidrs.dedup();
    cidrs
}

pub(super) fn ensure_attachments(
    environment: &Environment,
    mode: Mode,
    socket_path: &Path,
) -> Result<(), Failure> {
    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    let desired = environment
        .spec
        .vms
        .iter()
        .flat_map(|(config_name, vm)| {
            let runtime_id = vm_runtime_id(environment, config_name);
            vm.networks
                .iter()
                .map(move |network| (runtime_id.clone(), network.name.clone(), network.ip.clone()))
        })
        .collect::<BTreeSet<_>>();
    // Up and apply both converge attachments. Detach stack-owned drift first so
    // IP moves (auto-allocated create → desired static IP) cannot collide.
    if matches!(mode, Mode::Apply | Mode::Up) {
        for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
            let current = (
                attachment["vm_id"].as_str().unwrap_or_default().to_string(),
                attachment["network"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                attachment["ip"].as_str().unwrap_or_default().to_string(),
            );
            if attachment["project"] == environment.spec.project
                && attachment["stack"] == stack_id(environment)
                && !desired.contains(&current)
            {
                rpc(
                    socket_path,
                    "net.detach",
                    json!({
                        "vm_id": attachment["vm_id"],
                        "network": attachment["network"],
                        "project": environment.spec.project,
                    }),
                )?;
            }
        }
    }
    for (config_name, vm) in &environment.spec.vms {
        let runtime_id = vm_runtime_id(environment, config_name);
        for attachment in &vm.networks {
            let exists = snapshot["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|item| {
                    item["vm_id"] == runtime_id
                        && item["project"] == environment.spec.project
                        && item["network"] == attachment.name
                        && item["ip"] == attachment.ip
                });
            // After detach above, re-check is stale — only skip when the original
            // snapshot already had the exact desired triple (no-op). Otherwise attach.
            if !exists {
                // If we detached this vm/network in the loop above, snapshot still
                // lists the old row; treat same vm+network as needing attach.
                let mut labels = serde_json::Map::new();
                labels.insert("managed-by".into(), json!("vzctl"));
                let is_docker_backend = environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend == config::NetworkBackend::Docker);
                if vm.roles.iter().any(|role| role == "docker") && !is_docker_backend {
                    labels.insert("vzctl.dev/dns-services".into(), json!("docker"));
                }
                rpc(
                    socket_path,
                    "net.attach",
                    json!({
                        "vm_id": runtime_id,
                        "network": attachment.name,
                        "ip": attachment.ip,
                        "labels": labels,
                        "project": environment.spec.project,
                        "stack": stack_id(environment),
                    }),
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn prune_networks(
    environment: &Environment,
    mode: Mode,
    plan: &Plan,
    socket_path: &Path,
) -> Result<(), Failure> {
    if mode != Mode::Apply {
        return Ok(());
    }
    let names = plan
        .actions
        .iter()
        .filter(|action| action.action == "delete" && action.kind == "network")
        .map(|action| action.name.as_str())
        .collect::<BTreeSet<_>>();
    if names.is_empty() {
        return Ok(());
    }
    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    for network in snapshot["networks"].as_array().into_iter().flatten() {
        let name = network["name"].as_str().unwrap_or_default();
        if names.contains(name)
            && network["project"] == environment.spec.project
            && network["stack"] == stack_id(environment)
            && network["labels"]["managed-by"] == "vzctl"
        {
            rpc(
                socket_path,
                "net.delete",
                json!({
                    "name": network.get("id").unwrap_or(&network["name"]),
                    "project": environment.spec.project,
                }),
            )?;
        }
    }
    Ok(())
}

pub(super) fn detach_networks(
    environment: &Environment,
    socket_path: &Path,
) -> Result<(), Failure> {
    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
        if attachment["project"] == environment.spec.project
            && attachment["stack"] == stack_id(environment)
        {
            rpc(
                socket_path,
                "net.detach",
                json!({
                    "vm_id": attachment["vm_id"],
                    "network": attachment["network"],
                    "project": environment.spec.project,
                }),
            )?;
        }
    }
    Ok(())
}

pub(super) fn ensure_ports(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let forwards = config::collect_port_forwards(environment).map_err(|issues| {
        Failure::new(
            EXIT_INVALID,
            issues
                .iter()
                .map(|issue| format!("{}: {}", issue.path, issue.message))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    if forwards.is_empty() {
        rpc(
            socket_path,
            "port.purge",
            json!({
                "project": environment.spec.project,
                "stack": stack_id(environment),
            }),
        )?;
        return Ok(());
    }

    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    let attachments = snapshot["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut desired = Vec::new();
    for forward in forwards {
        let runtime_id = vm_runtime_id(environment, &forward.vm);
        let guest_ip = attachments
            .iter()
            .find(|item| {
                item["vm_id"] == runtime_id
                    && item["project"] == environment.spec.project
                    && item["stack"] == stack_id(environment)
            })
            .and_then(|item| item["ip"].as_str())
            .ok_or_else(|| {
                Failure::new(
                    EXIT_STEP,
                    format!(
                        "port forward {} needs attachment IP for VM {}",
                        forward.source, runtime_id
                    ),
                )
            })?
            .to_string();
        desired.push(json!({
            "bind": forward.bind,
            "host_port": forward.host_port,
            "guest_ip": guest_ip,
            "guest_port": forward.guest_port,
            "vm_id": runtime_id,
            "source": forward.source,
            "project": environment.spec.project,
            "stack": stack_id(environment),
        }));
    }
    rpc(
        socket_path,
        "port.ensure",
        json!({
            "project": environment.spec.project,
            "stack": stack_id(environment),
            "ports": desired,
        }),
    )?;
    Ok(())
}

pub(super) fn purge_dns_records(
    environment: &Environment,
    socket_path: &Path,
) -> Result<(), Failure> {
    let _ = rpc(
        socket_path,
        "dns.records.purge",
        json!({ "project": environment.spec.project }),
    );
    let _ = rpc(
        socket_path,
        "resilience.remove",
        json!({ "project": environment.spec.project }),
    );
    Ok(())
}

pub(super) fn purge_ports(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    rpc(
        socket_path,
        "port.purge",
        json!({
            "project": environment.spec.project,
            "stack": stack_id(environment),
        }),
    )
    .map(|_| ())
}

pub(super) fn apply_routes(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    if environment.spec.routes.is_empty() && environment.spec.policies.is_empty() {
        return Ok(());
    }
    // Fresh clones may still be on the sealed agent unit (no_new_privs) until
    // cloud-final runs daemon-reload + restart from append_agent_privilege_files.
    // Docker-role VMs install packages in cloud-config first, so cloud-final can
    // lag several minutes behind helper/agent readiness.
    let has_docker = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "docker"));
    let budget = if has_docker { 600 } else { 180 };
    let deadline = Instant::now() + Duration::from_secs(budget);
    loop {
        match rpc(
            socket_path,
            "route.apply",
            json!({"project": environment.spec.project, "router": null, "policies": environment.spec.policies}),
        ) {
            Ok(_) => return Ok(()),
            Err(error) => {
                let retryable = error.message.contains("no new privileges")
                    || error.message.contains("guest agent unavailable")
                    || error.message.contains("Connection reset")
                    || error.message.contains("is not running");
                if !retryable || Instant::now() >= deadline {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}
