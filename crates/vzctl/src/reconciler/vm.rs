use super::*;

pub(super) fn ensure_images(
    environment: &Environment,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    ensure_images_from_cache(
        environment,
        &crate::images_dir(),
        progress,
        |args, progress| run_self(args, progress).map(|_| ()),
    )
}

pub(super) fn ensure_images_from_cache(
    environment: &Environment,
    images_dir: &Path,
    progress: &mut ProgressReporter,
    mut run: impl FnMut(&[&str], &mut ProgressReporter) -> Result<(), Failure>,
) -> Result<(), Failure> {
    for image in environment.spec.images.values() {
        if crate::image::tagged_seal_ready(images_dir, &image.from, &image.tag)
            .map_err(|error| Failure::new(EXIT_STEP, error))?
        {
            continue;
        }
        let existing = crate::image::resolve_alias_pulled(images_dir, &image.from)
            .map_err(|error| Failure::new(EXIT_STEP, error))?
            .is_some();
        if !existing {
            progress.log(format!("pull image {}", image.from));
            run(
                &["image", "pull", &image.from, "--format", "json"],
                progress,
            )?;
        }
        progress.log(format!("bake {}:{}", image.from, image.tag));
        run(
            &[
                "image",
                "bake",
                &image.from,
                "--tag",
                &image.tag,
                "--format",
                "json",
            ],
            progress,
        )?;
        progress.log(format!("seal {}:{}", image.from, image.tag));
        run(
            &[
                "image",
                "seal",
                &image.from,
                "--tag",
                &image.tag,
                "--format",
                "json",
            ],
            progress,
        )?;
    }
    Ok(())
}

pub(super) fn ensure_vms(
    environment: &Environment,
    force: bool,
    plan: &Plan,
    config_path: &Path,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    if options_apply_deletes(plan) {
        for action in plan
            .actions
            .iter()
            .filter(|action| action.action == "delete" && action.kind == "vm")
        {
            stop_one(&action.name, socket_path)?;
            wait_stopped(&action.name, socket_path)?;
            remove_managed_vm(&action.name)?;
        }
    }
    let order = dependency_order(&environment.spec.vms)?;
    let config_dir = config::config_path(config_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    for name in order {
        let runtime_id = vm_runtime_id(environment, &name);
        let create_job = format!("vm:{runtime_id}:create");
        let vm = &environment.spec.vms[&name];
        let action = plan
            .actions
            .iter()
            .find(|action| action.kind == "vm" && action.name == runtime_id);
        if action.is_some_and(|action| action.action == "update" && action.breaking) {
            if !force {
                return Err(Failure::new(EXIT_INVALID, "VM recreate requires --force"));
            }
            let bundle = crate::state_dir().join("vms").join(&runtime_id);
            // After a failed later step, stack resources stay stale and plan keeps
            // asking for a breaking update. Don't wipe a bundle that already matches.
            if bundle_matches_vm(&bundle, vm)? {
                progress.job_skip(&create_job, "bereits im vorherigen Lauf ersetzt");
                continue;
            }
            stop_one(&runtime_id, socket_path)?;
            wait_stopped(&runtime_id, socket_path)?;
            remove_managed_vm(&runtime_id)?;
        }
        let bundle = crate::state_dir().join("vms").join(&runtime_id);
        if bundle.join("vm.json").is_file() {
            progress.log(format!("vm {runtime_id} already exists"));
            progress.job_skip(&create_job, "bereits vorhanden");
            continue;
        }
        let image_cfg = &environment.spec.images[&vm.from];
        let sealed_path =
            crate::image::resolve_alias_tag(&crate::images_dir(), &image_cfg.from, &image_cfg.tag)
                .map_err(|error| Failure::new(EXIT_STEP, error))?
                .ok_or_else(|| {
                    Failure::new(
                        EXIT_STEP,
                        format!(
                            "sealed image {}:{} missing after ensure_images",
                            image_cfg.from, image_cfg.tag
                        ),
                    )
                })?;
        let size = disk_gib(&vm.disk)?;
        let mut owned = vec![
            "vm".to_string(),
            "create".to_string(),
            runtime_id.clone(),
            "--from".to_string(),
            sealed_path.to_string_lossy().into_owned(),
            "--disk".to_string(),
            size.to_string(),
            "--project".to_string(),
            environment.spec.project.clone(),
        ];
        if let Some(cpus) = vm.cpus {
            if cpus == 0 {
                return Err(Failure::new(
                    EXIT_INVALID,
                    format!("VM {name} cpus must be greater than zero"),
                ));
            }
            owned.extend(["--cpus".to_string(), cpus.to_string()]);
        }
        if let Some(memory) = &vm.memory {
            let mib = memory_mib(memory)?;
            owned.extend(["--memory".to_string(), format!("{mib}MiB")]);
        }
        if let Some(network) = vm.networks.first() {
            owned.extend(["--network".to_string(), network.name.clone()]);
        }
        for role in &vm.roles {
            if role == "router" || role == "docker" {
                owned.extend(["--role".to_string(), role.clone()]);
            }
        }
        if let Some(cloud_init) = &vm.cloud_init {
            let path = if Path::new(cloud_init).is_absolute() {
                PathBuf::from(cloud_init)
            } else {
                config_dir.join(cloud_init)
            };
            owned.extend([
                "--cloud-init".to_string(),
                path.to_string_lossy().into_owned(),
            ]);
        }
        for mount in &vm.mounts {
            let Some(volume_path) = environment.spec.volumes.get(&mount.source) else {
                return Err(Failure::new(
                    EXIT_INVALID,
                    format!("VM {name} mount source {:?} is unknown", mount.source),
                ));
            };
            let resolved =
                crate::config::resolve_volume_path(volume_path, Some(config_dir.as_path()))
                    .unwrap_or_else(|| PathBuf::from(volume_path));
            let mut flag = format!(
                "tag={},source={},target={}",
                mount.source,
                resolved.display(),
                mount.target
            );
            if mount.read_only {
                flag.push_str(",ro");
            }
            owned.extend(["--mount".to_string(), flag]);
        }
        if vm.roles.iter().any(|role| role == "docker") {
            let project_dir = crate::mounts::resolve_project_dir(config_path)
                .map_err(|message| Failure::new(EXIT_INVALID, message))?;
            let project_target = project_dir.to_string_lossy().into_owned();
            let has_project = vm.mounts.iter().any(|mount| {
                mount.source == crate::mounts::DOCKER_PROJECT_MOUNT_TAG
                    || mount.target == project_target
            });
            if !has_project {
                let flag = crate::mounts::docker_project_mount_flag(&project_dir)
                    .map_err(|message| Failure::new(EXIT_INVALID, message))?;
                owned.extend(["--mount".to_string(), flag]);
            }
        }
        owned.extend(["--format".to_string(), "json".to_string()]);
        let refs = owned.iter().map(String::as_str).collect::<Vec<_>>();
        progress.log(format!("create vm {runtime_id}"));
        progress.job_start(&create_job);
        match run_self(&refs, progress) {
            Ok(result) => {
                add_cloud_init_summary_details(
                    progress,
                    &format!("vm:{runtime_id}:cloud-init"),
                    &result,
                );
                progress.job_done(&create_job);
            }
            Err(failure) => {
                progress.job_fail(&create_job, &failure.message);
                return Err(failure);
            }
        }
    }
    Ok(())
}

pub(super) fn start_helpers(
    environment: &Environment,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    for name in dependency_order(&environment.spec.vms)? {
        let runtime_id = vm_runtime_id(environment, &name);
        let job_id = format!("vm:{runtime_id}:start");
        let bundle = crate::state_dir().join("vms").join(&runtime_id);
        progress.log(format!("start vm {runtime_id}"));
        progress.job_start(&job_id);
        if let Err(failure) = rpc(
            socket_path,
            "vm.start",
            json!({"vm_id": runtime_id, "bundle": bundle}),
        ) {
            progress.job_fail(&job_id, &failure.message);
            return Err(failure);
        }
        progress.job_done(&job_id);
    }
    Ok(())
}

pub(super) fn await_helpers(
    environment: &Environment,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    let wanted = environment
        .spec
        .vms
        .keys()
        .map(|name| vm_runtime_id(environment, name))
        .collect::<BTreeSet<_>>();
    // Boot + vsock agent; docker-role cloud-init can lag, but agent itself is early.
    let has_docker = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "docker"));
    let budget = if has_docker { 180 } else { 120 };
    let deadline = Instant::now() + Duration::from_secs(budget);
    let mut last_status = String::new();
    let mut last_report = Instant::now() - Duration::from_secs(5);
    let mut completed = BTreeSet::new();
    for vm_id in &wanted {
        progress.job_start(&format!("vm:{vm_id}:agent"));
    }
    loop {
        let records = rpc(socket_path, "vm.list", json!({}))?;
        let failed = failed_helper_diagnostics(&records, &wanted);
        if !failed.is_empty() {
            for diagnostic in &failed {
                if let Some((vm_id, message)) = diagnostic.split_once(": ") {
                    progress.job_fail(&format!("vm:{vm_id}:agent"), message);
                }
            }
            return Err(Failure::new(
                EXIT_STEP,
                format!("helpers failed: {}", failed.join(", ")),
            ));
        }
        let running = records
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
            .collect::<BTreeSet<_>>();
        let mut ready = BTreeSet::new();
        let mut pending = Vec::new();
        for vm_id in &wanted {
            if !running.contains(vm_id) {
                pending.push(format!("{vm_id} (helper)"));
                continue;
            }
            match rpc(socket_path, "vm.agent.health", json!({ "vm_id": vm_id })) {
                Ok(_) => {
                    ready.insert(vm_id.clone());
                    if completed.insert(vm_id.clone()) {
                        progress.job_done(&format!("vm:{vm_id}:agent"));
                    }
                }
                Err(_) => pending.push(format!("{vm_id} (agent)")),
            }
        }
        if ready.len() == wanted.len() {
            progress.log(format!("agents ready ({})", ready.len()));
            return Ok(());
        }
        let status = format!("waiting: {}", pending.join(", "));
        if status != last_status || last_report.elapsed() >= Duration::from_secs(5) {
            progress.log(status.clone());
            last_status = status;
            last_report = Instant::now();
        }
        if Instant::now() >= deadline {
            for vm_id in wanted.difference(&ready) {
                progress.job_fail(
                    &format!("vm:{vm_id}:agent"),
                    "Guest Agent nicht rechtzeitig bereit",
                );
            }
            return Err(Failure::new(
                EXIT_STEP,
                format!("helpers/agents not ready: {}", pending.join(", ")),
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub(super) fn failed_helper_diagnostics(records: &Value, wanted: &BTreeSet<String>) -> Vec<String> {
    records
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| record["state"].as_str() == Some("failed"))
        .filter_map(|record| {
            let vm_id = record["vm_id"].as_str().or_else(|| record["id"].as_str())?;
            if !wanted.contains(vm_id) {
                return None;
            }
            let message = record["last_error"]
                .as_str()
                .map(safe_status_text)
                .unwrap_or_else(|| "helper exited unexpectedly".to_string());
            Some(format!("{vm_id}: {message}"))
        })
        .collect()
}

pub(super) fn stop_helpers(
    environment: &Environment,
    socket_path: &Path,
    force: bool,
) -> Result<(), Failure> {
    let mut order = dependency_order(&environment.spec.vms)?;
    order.reverse();
    for name in order {
        let runtime_id = vm_runtime_id(environment, &name);
        if force {
            // Stack remove / purge: hard-kill, no data-loss courtesy.
            // Bookkeeping is cleared immediately; brief settle so disk handles drop.
            let _ = rpc(
                socket_path,
                "vm.stop",
                json!({"vm_id": runtime_id, "force": true}),
            );
            std::thread::sleep(Duration::from_millis(250));
        } else {
            stop_one(&runtime_id, socket_path)?;
            wait_stopped(&runtime_id, socket_path)?;
        }
    }
    Ok(())
}

pub(super) fn stop_one(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
    rpc(socket_path, "vm.stop", json!({"vm_id": vm_id})).map(|_| ())
}

pub(super) fn wait_stopped(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
    wait_stopped_until(vm_id, socket_path, Duration::from_secs(30))
}

pub(super) fn wait_stopped_until(
    vm_id: &str,
    socket_path: &Path,
    timeout: Duration,
) -> Result<(), Failure> {
    let deadline = Instant::now() + timeout;
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
                EXIT_STEP,
                format!("VM {vm_id} did not stop before timeout"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn options_apply_deletes(plan: &Plan) -> bool {
    plan.actions
        .iter()
        .any(|action| action.action == "delete" && action.kind == "vm")
}

pub(super) fn purge_managed(
    environment: &Environment,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    for name in environment.spec.vms.keys() {
        let runtime_id = vm_runtime_id(environment, name);
        // Hard delete: SIGKILL via vm.purge + wipe managed bundle (no graceful stop).
        match run_self(
            &["vm", "delete", &runtime_id, "--force", "--format", "json"],
            progress,
        ) {
            Ok(_) => {}
            Err(failure) => {
                // Already gone after a partial purge is fine.
                let bundle = crate::state_dir().join("vms").join(&runtime_id);
                if bundle.join("vm.json").is_file() {
                    return Err(failure);
                }
            }
        }
    }
    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    for network in snapshot["networks"].as_array().into_iter().flatten() {
        if network["project"] == environment.spec.project
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

pub(super) fn remove_managed_vm(vm_id: &str) -> Result<(), Failure> {
    let bundle = crate::state_dir().join("vms").join(vm_id);
    let manifest = bundle.join("vm.json");
    if !manifest.is_file() {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(&fs::read(&manifest).map_err(|error| {
        Failure::new(EXIT_STEP, format!("read {}: {error}", manifest.display()))
    })?)
    .map_err(|error| Failure::new(EXIT_STEP, format!("parse {}: {error}", manifest.display())))?;
    if value["managed-by"] != "vzctl" {
        return Err(Failure::new(
            EXIT_STEP,
            format!("refusing to purge unmanaged VM bundle {}", bundle.display()),
        ));
    }
    fs::remove_dir_all(&bundle)
        .map_err(|error| Failure::new(EXIT_STEP, format!("purge {}: {error}", bundle.display())))
}

/// True when an existing bundle already reflects the desired root capacity,
/// roles and vmnet NIC addresses (docker-backend nets are logical and not in
/// identity).
pub(super) fn bundle_matches_vm(bundle: &Path, vm: &VmConfig) -> Result<bool, Failure> {
    let manifest = bundle.join("vm.json");
    if !manifest.is_file() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(&fs::read(&manifest).map_err(|error| {
        Failure::new(EXIT_STEP, format!("read {}: {error}", manifest.display()))
    })?)
    .map_err(|error| Failure::new(EXIT_STEP, format!("parse {}: {error}", manifest.display())))?;
    if value["disks"]["root"]["size_gib"].as_u64() != Some(disk_gib(&vm.disk)?) {
        return Ok(false);
    }
    let mut actual_roles = value["roles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    actual_roles.sort();
    let mut desired_roles = vm.roles.clone();
    desired_roles.sort();
    if actual_roles != desired_roles {
        return Ok(false);
    }
    let actual_ips = value["identity"]["nics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|nic| nic["address"].as_str().map(str::to_string))
        .collect::<BTreeSet<_>>();
    let desired_ips = vm
        .networks
        .iter()
        .map(|attachment| attachment.ip.clone())
        .collect::<BTreeSet<_>>();
    // Identity only lists vmnet NICs; require them to be a subset of desired IPs.
    if !actual_ips.is_subset(&desired_ips) {
        return Ok(false);
    }
    if !vm.networks.is_empty() && actual_ips.is_empty() {
        return Ok(false);
    }
    Ok(true)
}

pub(super) fn disk_gib(value: &str) -> Result<u64, Failure> {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| Failure::new(EXIT_INVALID, format!("invalid disk: {value}")))?;
    let number = value[..split]
        .parse::<u64>()
        .map_err(|_| Failure::new(EXIT_INVALID, format!("invalid disk: {value}")))?;
    let unit = value[split..].to_ascii_lowercase();
    match unit.as_str() {
        "g" | "gb" | "gib" => Ok(number),
        "t" | "tb" | "tib" => number
            .checked_mul(1024)
            .ok_or_else(|| Failure::new(EXIT_INVALID, "disk is too large")),
        _ => Err(Failure::new(
            EXIT_INVALID,
            format!("disk must use GiB/TiB for VM creation: {value}"),
        )),
    }
}

pub(super) fn memory_mib(value: &str) -> Result<u64, Failure> {
    crate::parse_memory_mib(value).map_err(|message| Failure::new(EXIT_INVALID, message))
}

pub(super) fn vm_runtime_id(environment: &Environment, config_name: &str) -> String {
    crate::runtime_vm_id(&environment.spec.project, config_name)
}
