use super::*;

#[derive(Debug)]
pub(super) enum CloudInitEvent {
    Update {
        vm_id: String,
        stage: Option<String>,
        completed: Option<(u32, u32)>,
        datasource: Option<String>,
    },
    Done {
        vm_id: String,
    },
    Failed {
        vm_id: String,
        message: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum CloudInitState {
    Running,
    Done,
    Failed(String),
}

#[derive(Debug)]
pub(super) struct CloudInitObservation {
    pub(super) state: CloudInitState,
    pub(super) stage: Option<String>,
    pub(super) datasource: Option<String>,
}

pub(super) fn await_cloud_init(
    environment: &Environment,
    plan: &Plan,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    let targets = plan
        .actions
        .iter()
        .filter(|action| action.kind == "vm" && matches!(action.action, "create" | "update"))
        .map(|action| action.name.clone())
        .collect::<BTreeSet<_>>();
    let all = environment
        .spec
        .vms
        .iter()
        .map(|(name, vm)| (vm_runtime_id(environment, name), vm))
        .collect::<Vec<_>>();

    for (vm_id, _) in &all {
        let job_id = format!("vm:{vm_id}:cloud-init");
        if targets.contains(vm_id) {
            add_cloud_init_manifest_details(progress, &job_id, vm_id);
            progress.job_start(&job_id);
        } else {
            progress.job_skip(&job_id, "VM unverändert");
        }
    }
    if targets.is_empty() {
        progress.log("cloud-init: keine neu erstellten oder ersetzten VMs");
        return Ok(());
    }

    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for (vm_id, vm) in all.into_iter().filter(|(vm_id, _)| targets.contains(vm_id)) {
            let tx = tx.clone();
            let timeout = if vm.roles.iter().any(|role| role == "docker") {
                Duration::from_secs(600)
            } else {
                Duration::from_secs(180)
            };
            scope.spawn(move || monitor_cloud_init(socket_path, &vm_id, timeout, tx));
        }
        drop(tx);

        let mut failures = Vec::new();
        while let Ok(event) = rx.recv() {
            match event {
                CloudInitEvent::Update {
                    vm_id,
                    stage,
                    completed,
                    datasource,
                } => {
                    let job_id = format!("vm:{vm_id}:cloud-init");
                    if let Some(datasource) = datasource {
                        progress.job_detail(&job_id, "Datasource", datasource);
                    }
                    if let Some(stage) = &stage {
                        progress.job_detail(&job_id, "Stufe", stage.clone());
                    }
                    if let Some((done, total)) = completed {
                        progress.job_progress(&job_id, done, total, stage);
                    } else if let Some(stage) = stage {
                        progress.log(format!("{vm_id}: cloud-init {stage}"));
                    }
                }
                CloudInitEvent::Done { vm_id } => {
                    let job_id = format!("vm:{vm_id}:cloud-init");
                    progress.job_progress(&job_id, 100, 100, Some("done".to_string()));
                    progress.job_done(&job_id);
                }
                CloudInitEvent::Failed { vm_id, message } => {
                    progress.job_fail(&format!("vm:{vm_id}:cloud-init"), &message);
                    failures.push(format!("{vm_id}: {message}"));
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Failure::new(
                EXIT_STEP,
                format!("cloud-init provisioning failed: {}", failures.join("; ")),
            ))
        }
    })
}

pub(super) fn monitor_cloud_init(
    socket_path: &Path,
    vm_id: &str,
    timeout: Duration,
    tx: mpsc::Sender<CloudInitEvent>,
) {
    monitor_cloud_init_with(vm_id, timeout, Duration::from_secs(1), tx, || {
        query_cloud_init(socket_path, vm_id)
    });
}

pub(super) fn monitor_cloud_init_with(
    vm_id: &str,
    timeout: Duration,
    poll_interval: Duration,
    tx: mpsc::Sender<CloudInitEvent>,
    mut query: impl FnMut() -> Result<CloudInitObservation, Failure>,
) {
    let deadline = Instant::now() + timeout;
    let mut last_stage = None;
    let mut last_error = None;
    loop {
        match query() {
            Ok(observation) => match observation.state {
                CloudInitState::Done => {
                    let _ = tx.send(CloudInitEvent::Done {
                        vm_id: vm_id.to_string(),
                    });
                    return;
                }
                CloudInitState::Failed(message) => {
                    let _ = tx.send(CloudInitEvent::Failed {
                        vm_id: vm_id.to_string(),
                        message,
                    });
                    return;
                }
                CloudInitState::Running => {
                    if observation.stage != last_stage {
                        last_stage = observation.stage.clone();
                        let _ = tx.send(CloudInitEvent::Update {
                            vm_id: vm_id.to_string(),
                            completed: progress::cloud_init_stage_progress(
                                observation.stage.as_deref(),
                            ),
                            stage: observation.stage,
                            datasource: observation.datasource,
                        });
                    }
                }
            },
            Err(failure) => last_error = Some(failure.message),
        }
        if Instant::now() >= deadline {
            let diagnostic = last_error
                .map(|message| format!("; letzte Diagnose: {}", safe_status_text(&message)))
                .unwrap_or_default();
            let _ = tx.send(CloudInitEvent::Failed {
                vm_id: vm_id.to_string(),
                message: format!(
                    "Timeout nach {}s{diagnostic}; prüfen mit `vzctl vm exec {vm_id} -- cloud-init status --long`",
                    timeout.as_secs()
                ),
            });
            return;
        }
        if !poll_interval.is_zero() {
            thread::sleep(poll_interval);
        }
    }
}

pub(super) fn query_cloud_init(
    socket_path: &Path,
    vm_id: &str,
) -> Result<CloudInitObservation, Failure> {
    let result = rpc(
        socket_path,
        "vm.exec",
        json!({
            "vm_id": vm_id,
            "cmd": ["cloud-init", "status", "--format=json"],
            "timeout_ms": 5_000,
        }),
    )?;
    let exit = result["exit"].as_u64().unwrap_or(1);
    let stdout = result["stdout"].as_str().unwrap_or_default();
    let stderr = result["stderr"].as_str().unwrap_or_default();
    if cloud_init_command_missing(exit, stderr) {
        return Ok(CloudInitObservation {
            state: CloudInitState::Failed("cloud-init ist im Guest nicht installiert".to_string()),
            stage: None,
            datasource: None,
        });
    }
    if let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) {
        return Ok(parse_cloud_init_json(exit, &value));
    }

    let fallback = rpc(
        socket_path,
        "vm.exec",
        json!({
            "vm_id": vm_id,
            "cmd": ["cloud-init", "status", "--long"],
            "timeout_ms": 5_000,
        }),
    )?;
    Ok(parse_cloud_init_long(
        fallback["exit"].as_u64().unwrap_or(exit),
        fallback["stdout"].as_str().unwrap_or_default(),
        fallback["stderr"].as_str().unwrap_or_default(),
    ))
}

pub(super) fn cloud_init_command_missing(exit: u64, stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    exit == 127 || stderr.contains("not found") || stderr.contains("no such file")
}

pub(super) fn parse_cloud_init_json(exit: u64, value: &Value) -> CloudInitObservation {
    let status = value["extended_status"]
        .as_str()
        .or_else(|| value["status"].as_str())
        .unwrap_or("running")
        .to_ascii_lowercase();
    let stage = value["stage"].as_str().map(normalize_cloud_init_stage);
    let datasource = value["datasource"]
        .as_str()
        .or_else(|| value["detail"].as_str())
        .map(safe_status_text);
    let errors = value["errors"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
        || value["recoverable_errors"]
            .as_object()
            .is_some_and(|items| !items.is_empty());
    let state = if exit == 2 || status.contains("degraded") || errors {
        CloudInitState::Failed("cloud-init meldet behebbare Fehler (Exit 2/degraded)".to_string())
    } else if exit == 1 || status.contains("error") {
        CloudInitState::Failed("cloud-init ist fehlgeschlagen (Exit 1)".to_string())
    } else if status.contains("disabled") {
        CloudInitState::Failed("cloud-init ist für diese VM deaktiviert".to_string())
    } else if status.contains("done") {
        CloudInitState::Done
    } else {
        CloudInitState::Running
    };
    CloudInitObservation {
        state,
        stage,
        datasource,
    }
}

pub(super) fn parse_cloud_init_long(exit: u64, stdout: &str, stderr: &str) -> CloudInitObservation {
    let combined = format!("{stdout}\n{stderr}");
    let lower = combined.to_ascii_lowercase();
    let stage = combined.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Running in stage:")
            .map(|value| normalize_cloud_init_stage(value.trim()))
    });
    let state = if exit == 2 || lower.contains("degraded") {
        CloudInitState::Failed("cloud-init meldet behebbare Fehler (Exit 2/degraded)".to_string())
    } else if exit == 1 || lower.contains("status: error") {
        CloudInitState::Failed("cloud-init ist fehlgeschlagen (Exit 1)".to_string())
    } else if lower.contains("status: disabled") {
        CloudInitState::Failed("cloud-init ist für diese VM deaktiviert".to_string())
    } else if lower.contains("status: done") {
        CloudInitState::Done
    } else {
        CloudInitState::Running
    };
    CloudInitObservation {
        state,
        stage,
        datasource: None,
    }
}

pub(super) fn normalize_cloud_init_stage(stage: &str) -> String {
    match stage.trim() {
        "init-local" => "init-local",
        "init" => "init",
        "modules-config" | "config" => "modules-config",
        "modules-final" | "final" => "modules-final",
        other => other,
    }
    .to_string()
}

pub(super) fn safe_status_text(value: &str) -> String {
    let one_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = one_line.to_ascii_lowercase();
    if [
        "password",
        "passwd",
        "token",
        "secret",
        "authorization",
        "private key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return "[sensible Diagnose ausgeblendet]".to_string();
    }
    one_line.chars().take(160).collect()
}

pub(super) fn add_cloud_init_manifest_details(
    progress: &mut ProgressReporter,
    job_id: &str,
    vm_id: &str,
) {
    let manifest_path = crate::state_dir().join("vms").join(vm_id).join("vm.json");
    let Ok(bytes) = fs::read(manifest_path) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    add_cloud_init_summary_details(progress, job_id, &value);
}

pub(super) fn add_cloud_init_summary_details(
    progress: &mut ProgressReporter,
    job_id: &str,
    value: &Value,
) {
    let summary = value
        .pointer("/cloud_init/summary")
        .or_else(|| value.pointer("/result/cloud_init/summary"));
    let Some(summary) = summary else {
        return;
    };
    if let Some(roles) = summary["roles"].as_array() {
        let roles = roles.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if !roles.is_empty() {
            progress.job_detail(job_id, "Rollen", roles.join(", "));
        }
    }
    let packages = summary["packages"].as_array().map(Vec::len).unwrap_or(0);
    let files = summary["write_files"].as_array().map(Vec::len).unwrap_or(0);
    let commands = summary["commands"].as_u64().unwrap_or(0);
    let users = summary["users"].as_u64().unwrap_or(0);
    progress.job_detail(
        job_id,
        "Inhalt",
        format!("{files} Dateien · {packages} Pakete · {commands} Kommandos · {users} Benutzer"),
    );
    let package_names = summary["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if !package_names.is_empty() {
        progress.job_detail(job_id, "Pakete", package_names.join(", "));
    }
    let file_paths = summary["write_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if !file_paths.is_empty() {
        progress.job_detail(job_id, "Dateien", file_paths.join(", "));
    }
}

pub(super) fn ensure_guest_utils(
    environment: &Environment,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    let targets = environment
        .spec
        .vms
        .keys()
        .map(|name| vm_runtime_id(environment, name))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(());
    }
    let bundle = crate::guest_utils::ensure_cached_bundle(&crate::state_dir())
        .map_err(|error| Failure::new(EXIT_STEP, error.message))?;
    progress.log(format!(
        "guest utils bundle {} (agent {})",
        bundle.bundle_id, bundle.agent_version
    ));
    let results = crate::guest_utils::rollout_targets(&targets, &bundle, &mut |method, params| {
        rpc(socket_path, method, params).map_err(|failure| failure.message)
    })
    .map_err(|error| Failure::new(EXIT_STEP, error.message))?;
    let upgraded = results
        .iter()
        .filter(|result| result["status"].as_str() == Some("upgraded"))
        .count();
    if upgraded > 0 {
        progress.log(format!("guest utils upgraded on {upgraded} VM(s)"));
    }
    Ok(())
}

/// Share the hypernetwork project directory into docker-role VMs at the same
/// absolute path so container `-v /Users/…/project:…` binds work.
pub(super) fn ensure_docker_project_mount(
    environment: &Environment,
    config_path: &Path,
    _socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    let docker_vms = environment
        .spec
        .vms
        .iter()
        .filter(|(_, vm)| vm.roles.iter().any(|role| role == "docker"))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    if docker_vms.is_empty() {
        return Ok(());
    }
    let project_dir = crate::mounts::resolve_project_dir(config_path)
        .map_err(|message| Failure::new(EXIT_INVALID, message))?;
    let target = project_dir.to_string_lossy().into_owned();
    let source = project_dir.to_string_lossy().into_owned();
    for name in docker_vms {
        let runtime_id = vm_runtime_id(environment, &name);
        let bundle = crate::state_dir().join("vms").join(&runtime_id);
        if !bundle.join("vm.json").is_file() {
            continue;
        }
        let mounts = crate::mounts::read_manifest_mounts(&bundle).unwrap_or_default();
        if let Some(existing) = mounts.iter().find(|mount| mount.target == target) {
            if existing.source == project_dir
                && existing.name == crate::mounts::DOCKER_PROJECT_MOUNT_TAG
            {
                // Still call mount so a running VM that lost the bind remounts.
            } else if existing.source != project_dir {
                return Err(Failure::new(
                    EXIT_INVALID,
                    format!(
                        "docker VM {runtime_id}: mount target {target} already uses source {} (expected project dir {})",
                        existing.source.display(),
                        project_dir.display()
                    ),
                ));
            }
        }
        run_self_retrying(
            &[
                "vm",
                "mount",
                &runtime_id,
                "--source",
                &source,
                "--target",
                &target,
                "--tag",
                crate::mounts::DOCKER_PROJECT_MOUNT_TAG,
                "--format",
                "json",
            ],
            Duration::from_secs(90),
            |failure| {
                failure.message.contains("is not running")
                    || failure.message.contains("guest agent unavailable")
                    || failure.message.contains("Connection reset")
                    || failure.message.contains("Connection refused")
            },
            progress,
        )?;
    }
    Ok(())
}
