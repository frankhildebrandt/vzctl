use crate::config::{self, Environment, VmConfig};
use crate::progress::{
    self, parse_progress_flag, resolve_progress_mode, ProgressMessage, ProgressMode,
    ProgressReporter, APPLY_STEPS, DOWN_STEPS,
};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_INCOMPLETE: u8 = crate::EXIT_INCOMPLETE_JOURNAL;
const EXIT_LEASE: u8 = crate::EXIT_LEASE_HELD;
const EXIT_SUPERVISOR: u8 = 10;
const EXIT_STEP: u8 = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Plan,
    Diff,
    Up,
    Apply,
    Down,
    Adopt,
}

impl Mode {
    fn command(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Diff => "diff",
            Self::Up => "up",
            Self::Apply => "apply",
            Self::Down => "down",
            Self::Adopt => "adopt",
        }
    }
}

#[derive(Clone, Debug)]
struct Options {
    mode: Mode,
    config: PathBuf,
    format: Format,
    force: bool,
    resume: bool,
    abort: bool,
    purge: bool,
    progress: ProgressMode,
}

#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Resource {
    kind: String,
    name: String,
    labels: BTreeMap<String, String>,
    state: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct PlanAction {
    action: &'static str,
    kind: String,
    name: String,
    breaking: bool,
    reason: String,
}

#[derive(Debug)]
struct Plan {
    actions: Vec<PlanAction>,
    desired: Vec<Resource>,
}

pub(crate) fn command(
    mode: &str,
    args: impl Iterator<Item = String>,
    socket_path: &Path,
) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let options = match parse(mode, args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, mode, &failure);
            return ExitCode::from(failure.code);
        }
    };
    if options.progress == ProgressMode::Ui && progress::ui_available() {
        return run_with_ui(&options, socket_path);
    }
    let progress_mode = if options.progress == ProgressMode::Ui {
        eprintln!("progress dashboard requires an interactive terminal; using plain output");
        ProgressMode::Plain
    } else {
        options.progress
    };
    let steps = if options.mode == Mode::Down {
        DOWN_STEPS
    } else {
        APPLY_STEPS
    };
    let mut reporter = ProgressReporter::new(progress_mode, None, steps);
    let _listener = if reporter.enabled() {
        Some(progress::spawn_event_listener(
            socket_path,
            None,
            Some(reporter.percent_handle()),
        ))
    } else {
        None
    };
    match run(&options, socket_path, &mut reporter) {
        Ok(output) => {
            emit_success(&options, &output);
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_failure(options.format, options.mode.command(), &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn run_with_ui(options: &Options, socket_path: &Path) -> ExitCode {
    let (tx, rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let _listener = progress::spawn_event_listener(socket_path, Some(tx.clone()), None);
    let steps: &[&str] = if options.mode == Mode::Down {
        DOWN_STEPS
    } else {
        APPLY_STEPS
    };
    let title = format!("{} {}", options.mode.command(), options.config.display());
    let apply_options = options.clone();
    let apply_socket = socket_path.to_path_buf();
    let ui_result = progress::run_ui_session(&title, rx, move || {
        let mut reporter = ProgressReporter::new(ProgressMode::Ui, Some(tx), steps);
        let result = run(&apply_options, &apply_socket, &mut reporter);
        let _ = result_tx.send(match &result {
            Ok(output) => Ok(output.clone()),
            Err(failure) => Err(Failure::new(failure.code, failure.message.clone())),
        });
        result.map(|_| ()).map_err(|failure| failure.message)
    });
    let mode_command = options.mode.command();
    let format = options.format;
    match ui_result {
        Ok(()) => match result_rx.recv() {
            Ok(Ok(output)) => {
                emit_success(options, &output);
                ExitCode::SUCCESS
            }
            Ok(Err(failure)) => {
                emit_failure(format, mode_command, &failure);
                ExitCode::from(failure.code)
            }
            Err(_) => ExitCode::SUCCESS,
        },
        Err(message) => {
            emit_failure(format, mode_command, &Failure::new(EXIT_STEP, message));
            ExitCode::from(EXIT_STEP)
        }
    }
}

fn run(
    options: &Options,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<Value, Failure> {
    let environment = load_config(&options.config)?;
    let stack_id = stack_id(&environment);
    let desired = desired_resources(&environment)?;
    let desired_hash = desired_hash(&desired)?;

    if options.abort {
        let holder = holder();
        let journal = rpc(
            socket_path,
            "stack.abort",
            json!({"stack_id": stack_id, "holder": holder}),
        )?;
        return Ok(json!({
            "message": "incomplete apply aborted; drift remains visible",
            "stack_id": stack_id,
            "journal": journal,
            "actions": [],
            "changed": false,
        }));
    }

    let state = rpc(socket_path, "stack.inspect", json!({"stack_id": stack_id}))?;
    let actual = actual_resources(&state)?;
    let (effective_mode, effective_purge) = if options.resume {
        journal_context(&state)?
    } else {
        (options.mode, options.purge)
    };

    if matches!(options.mode, Mode::Plan | Mode::Diff) {
        let plan = build_plan(effective_mode, desired, actual);
        return Ok(plan_output(&stack_id, &plan));
    }
    if options.mode == Mode::Adopt {
        return Ok(adopt_report(
            &stack_id,
            &environment,
            &desired,
            &actual,
            socket_path,
        )?);
    }

    let plan = build_plan(effective_mode, desired, actual);
    let vm_ids = if effective_mode != Mode::Down {
        let vm_ids = dependency_order(&environment.spec.vms)?
            .into_iter()
            .map(|name| vm_runtime_id(&environment, &name))
            .collect::<Vec<_>>();
        progress.add_vm_jobs(&vm_ids);
        vm_ids
    } else {
        Vec::new()
    };

    let holder = holder();
    let mode = if options.resume {
        "resume"
    } else {
        options.mode.command()
    };
    let journal = rpc(
        socket_path,
        "stack.begin",
        json!({
            "stack_id": stack_id,
            "holder": holder,
            "desired_hash": desired_hash,
            "mode": mode,
            "purge": options.purge,
        }),
    )?;
    let journal_id = journal["id"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "invalid stack.begin response"))?
        .to_string();
    if plan.actions.iter().any(|action| action.breaking)
        && !options.force
        && !confirm_breaking(&plan.actions)?
    {
        let _ = rpc(
            socket_path,
            "stack.abort",
            json!({"stack_id": stack_id, "holder": holder}),
        );
        return Err(Failure::new(
            EXIT_INVALID,
            "breaking changes require confirmation or --force",
        ));
    }
    let resume_step = options
        .resume
        .then(|| journal["step"].as_str().unwrap_or("validate").to_string());
    let steps = if effective_mode == Mode::Down {
        DOWN_STEPS.to_vec()
    } else {
        APPLY_STEPS.to_vec()
    };

    let mut reached_resume = resume_step.is_none();
    for step in steps {
        if !reached_resume {
            reached_resume = resume_step.as_deref() == Some(step);
            if !reached_resume {
                progress.job_skip(&format!("step:{step}"), "bereits im Journal abgeschlossen");
                mark_resumed_vm_jobs(progress, &vm_ids, step);
                continue;
            }
        }
        progress.step_start(step);
        checkpoint(
            socket_path,
            &journal_id,
            &stack_id,
            &holder,
            step,
            "running",
            None,
        )?;
        let mut execution_options = options.clone();
        execution_options.mode = effective_mode;
        execution_options.purge = effective_purge;
        if let Err(failure) = execute_step(
            step,
            &execution_options,
            &environment,
            &plan,
            socket_path,
            progress,
        ) {
            let _ = checkpoint(
                socket_path,
                &journal_id,
                &stack_id,
                &holder,
                step,
                "failed",
                Some(&failure.message),
            );
            progress.job_fail(&format!("step:{step}"), &failure.message);
            progress.finished(false, &failure.message);
            return Err(failure);
        }
        checkpoint(
            socket_path,
            &journal_id,
            &stack_id,
            &holder,
            step,
            "completed",
            None,
        )?;
        progress.step_done(step);
    }

    let final_resources = if effective_mode == Mode::Down && effective_purge {
        Vec::new()
    } else if effective_mode == Mode::Down {
        plan.desired
            .iter()
            .cloned()
            .map(|mut resource| {
                if resource.kind == "vm" {
                    resource.state = "stopped".to_string();
                }
                resource
            })
            .collect()
    } else {
        plan.desired.clone()
    };
    rpc(
        socket_path,
        "stack.finish",
        json!({
            "id": journal_id,
            "stack_id": stack_id,
            "holder": holder,
            "resources": final_resources,
        }),
    )?;
    progress.finished(true, format!("{} completed", effective_mode.command()));
    Ok(json!({
        "message": format!("{} completed", effective_mode.command()),
        "stack_id": stack_id,
        "journal": {"id": journal_id, "status": "done"},
        "actions": plan.actions,
        "changed": !plan.actions.is_empty() || options.mode == Mode::Down,
    }))
}

fn mark_resumed_vm_jobs(progress: &mut ProgressReporter, vm_ids: &[String], step: &str) {
    let suffix = match step {
        "ensure_vms" => Some("create"),
        "start_helpers" => Some("start"),
        "await_agents" => Some("agent"),
        "await_cloud_init" => Some("cloud-init"),
        _ => None,
    };
    let Some(suffix) = suffix else {
        return;
    };
    for vm_id in vm_ids {
        progress.job_skip(
            &format!("vm:{vm_id}:{suffix}"),
            "bereits im Journal abgeschlossen",
        );
    }
}

fn execute_step(
    step: &str,
    options: &Options,
    environment: &Environment,
    plan: &Plan,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    match step {
        "ensure_nets" => ensure_networks(environment, options.force, socket_path),
        "ensure_dns" => ensure_dns(environment, &options.config, socket_path, progress),
        "ensure_images" => ensure_images(environment, progress),
        "ensure_vms" => ensure_vms(
            environment,
            options.force,
            plan,
            &options.config,
            socket_path,
            progress,
        ),
        "attach_nets" => {
            ensure_attachments(environment, options.mode, socket_path)?;
            prune_networks(environment, options.mode, plan, socket_path)
        }
        "start_helpers" => start_helpers(environment, socket_path, progress),
        "await_agents" => {
            // Resume often lands here after a failed await while helpers already
            // exited — re-run start_helpers before waiting again.
            if options.resume {
                start_helpers(environment, socket_path, progress)?;
            }
            await_helpers(environment, socket_path, progress)
        }
        "await_cloud_init" => await_cloud_init(environment, plan, socket_path, progress),
        "ensure_guest_utils" => ensure_guest_utils(environment, socket_path, progress),
        "ensure_docker_project_mount" => {
            ensure_docker_project_mount(environment, &options.config, socket_path, progress)
        }
        "ensure_ca" => ensure_ca(environment),
        "ensure_oidc" => ensure_oidc(environment, &options.config, socket_path),
        "ensure_ingress" => ensure_ingress(environment, socket_path),
        "ensure_ca_rollout" => ensure_ca_rollout(environment, socket_path),
        "ensure_oidc_inject" => ensure_oidc_inject(environment, socket_path),
        "ensure_docker_context" => ensure_docker_context(environment),
        "ensure_containers" => ensure_containers(environment, &options.config, socket_path),
        "ensure_ports" => ensure_ports(environment, socket_path),
        "apply_routes_policies" => apply_routes(environment, socket_path),
        "purge_ingress" => purge_ingress(environment, socket_path),
        "purge_dns_records" => purge_dns_records(environment, socket_path),
        "purge_oidc" => purge_oidc(environment, socket_path),
        "stop_helpers" => stop_helpers(environment, socket_path, options.purge),
        "detach_nets" if options.purge => detach_networks(environment, socket_path),
        "destroy_managed" if options.purge => purge_managed(environment, socket_path, progress),
        "purge_docker_context" if options.purge => purge_docker_context(environment),
        "purge_ports" if options.purge => purge_ports(environment, socket_path),
        "dns_cleanup" if options.purge && environment.spec.dns.host_resolver => {
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
            .map(|_| ())
        }
        _ => Ok(()),
    }
}

fn ensure_networks(
    environment: &Environment,
    force: bool,
    socket_path: &Path,
) -> Result<(), Failure> {
    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    let current = snapshot["networks"].as_array().cloned().unwrap_or_default();
    for (name, network) in &environment.spec.networks {
        let existing = current.iter().find(|item| item["name"] == *name);
        if let Some(existing) = existing {
            let mode = serde_json::to_value(network.mode)
                .unwrap_or(Value::Null)
                .as_str()
                .unwrap_or("shared")
                .to_string();
            let existing_nat = existing["nat_egress"].as_bool().unwrap_or(true);
            let existing_backend = existing["backend"].as_str().unwrap_or("vmnet");
            let desired_backend = match network.backend {
                config::NetworkBackend::Vmnet => "vmnet",
                config::NetworkBackend::Docker => "docker",
            };
            if existing["cidr"] != network.cidr
                || existing["mode"] != mode
                || existing_nat != network.nat_egress
                || existing_backend != desired_backend
            {
                if !force {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        format!("network {name} requires recreate; use --force"),
                    ));
                }
                for attachment in snapshot["attachments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["network"] == *name)
                {
                    rpc(
                        socket_path,
                        "net.detach",
                        json!({
                            "vm_id": attachment["vm_id"],
                            "network": name,
                        }),
                    )?;
                }
                rpc(socket_path, "net.delete", json!({"name": name}))?;
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

fn ensure_dns(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    if environment.spec.dns.host_resolver {
        let path = config::config_path(config_path)
            .to_string_lossy()
            .into_owned();
        run_self_privileged(
            &[
                "dns",
                "install-resolver",
                "--config",
                &path,
                "--format",
                "json",
            ],
            progress,
        )
        .map(|_| ())?;
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

fn ensure_images(
    environment: &Environment,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    for image in environment.spec.images.values() {
        if crate::image::tagged_seal_ready(&crate::images_dir(), &image.from, &image.tag)
            .map_err(|error| Failure::new(EXIT_STEP, error))?
        {
            continue;
        }
        let existing = crate::image::resolve_alias_pulled(&crate::images_dir(), &image.from)
            .map_err(|error| Failure::new(EXIT_STEP, error))?
            .is_some();
        if !existing {
            progress.log(format!("pull image {}", image.from));
            run_self(
                &["image", "pull", &image.from, "--format", "json"],
                progress,
            )?;
        }
        progress.log(format!("bake {}:{}", image.from, image.tag));
        run_self(
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
        run_self(
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

fn ensure_vms(
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
        let size = data_disk_gib(&vm.data_disk)?;
        let mut owned = vec![
            "vm".to_string(),
            "create".to_string(),
            runtime_id.clone(),
            "--from".to_string(),
            sealed_path.to_string_lossy().into_owned(),
            "--data-disk".to_string(),
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

fn ensure_attachments(
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

fn prune_networks(
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
            rpc(socket_path, "net.delete", json!({"name": name}))?;
        }
    }
    Ok(())
}

fn start_helpers(
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

fn await_helpers(
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

fn failed_helper_diagnostics(records: &Value, wanted: &BTreeSet<String>) -> Vec<String> {
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

#[derive(Debug)]
enum CloudInitEvent {
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
enum CloudInitState {
    Running,
    Done,
    Failed(String),
}

#[derive(Debug)]
struct CloudInitObservation {
    state: CloudInitState,
    stage: Option<String>,
    datasource: Option<String>,
}

fn await_cloud_init(
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

fn monitor_cloud_init(
    socket_path: &Path,
    vm_id: &str,
    timeout: Duration,
    tx: mpsc::Sender<CloudInitEvent>,
) {
    monitor_cloud_init_with(vm_id, timeout, Duration::from_secs(1), tx, || {
        query_cloud_init(socket_path, vm_id)
    });
}

fn monitor_cloud_init_with(
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

fn query_cloud_init(socket_path: &Path, vm_id: &str) -> Result<CloudInitObservation, Failure> {
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

fn cloud_init_command_missing(exit: u64, stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    exit == 127 || stderr.contains("not found") || stderr.contains("no such file")
}

fn parse_cloud_init_json(exit: u64, value: &Value) -> CloudInitObservation {
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

fn parse_cloud_init_long(exit: u64, stdout: &str, stderr: &str) -> CloudInitObservation {
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

fn normalize_cloud_init_stage(stage: &str) -> String {
    match stage.trim() {
        "init-local" => "init-local",
        "init" => "init",
        "modules-config" | "config" => "modules-config",
        "modules-final" | "final" => "modules-final",
        other => other,
    }
    .to_string()
}

fn safe_status_text(value: &str) -> String {
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

fn add_cloud_init_manifest_details(progress: &mut ProgressReporter, job_id: &str, vm_id: &str) {
    let manifest_path = crate::state_dir().join("vms").join(vm_id).join("vm.json");
    let Ok(bytes) = fs::read(manifest_path) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    add_cloud_init_summary_details(progress, job_id, &value);
}

fn add_cloud_init_summary_details(progress: &mut ProgressReporter, job_id: &str, value: &Value) {
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

fn ensure_guest_utils(
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
fn ensure_docker_project_mount(
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

/// Retry `run_self` while `retryable` matches, until `budget` elapses.
fn run_self_retrying(
    args: &[&str],
    budget: Duration,
    retryable: impl Fn(&Failure) -> bool,
    progress: &mut ProgressReporter,
) -> Result<Value, Failure> {
    let deadline = Instant::now() + budget;
    loop {
        match run_self(args, progress) {
            Ok(value) => return Ok(value),
            Err(failure) if retryable(&failure) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(failure) => return Err(failure),
        }
    }
}

fn stop_helpers(environment: &Environment, socket_path: &Path, force: bool) -> Result<(), Failure> {
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

fn stop_one(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
    rpc(socket_path, "vm.stop", json!({"vm_id": vm_id})).map(|_| ())
}

fn wait_stopped(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
    wait_stopped_until(vm_id, socket_path, Duration::from_secs(30))
}

fn wait_stopped_until(vm_id: &str, socket_path: &Path, timeout: Duration) -> Result<(), Failure> {
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

fn options_apply_deletes(plan: &Plan) -> bool {
    plan.actions
        .iter()
        .any(|action| action.action == "delete" && action.kind == "vm")
}

fn detach_networks(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
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
                }),
            )?;
        }
    }
    Ok(())
}

fn purge_managed(
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
            rpc(socket_path, "net.delete", json!({"name": network["name"]}))?;
        }
    }
    Ok(())
}

fn ensure_docker_context(environment: &Environment) -> Result<(), Failure> {
    let has_docker = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "docker"));
    if !has_docker {
        return Ok(());
    }
    crate::docker::ensure_context(&environment.spec.project, &crate::state_dir(), None)
        .map_err(|error| Failure::new(EXIT_STEP, format!("docker context: {error}")))?;
    Ok(())
}

fn ensure_containers(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
) -> Result<(), Failure> {
    let config_dir = config::config_path(config_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let state_dir = crate::state_dir();
    let mut dns_records = Vec::new();
    for (vm_key, vm) in &environment.spec.vms {
        if !vm.roles.iter().any(|role| role == "docker") {
            continue;
        }
        if vm.compose_files.is_empty() && vm.containers.is_empty() {
            continue;
        }
        crate::docker::ensure_vm_containers(
            &environment.spec.project,
            &state_dir,
            &config_dir,
            vm_key,
            &vm.compose_files,
            &vm.containers,
        )
        .map_err(|error| {
            Failure::new(
                EXIT_STEP,
                format!("ensure containers on VM {vm_key}: {error}"),
            )
        })?;

        let docker_attachment = vm
            .networks
            .iter()
            .find(|attachment| {
                environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend == crate::config::NetworkBackend::Docker)
            })
            .ok_or_else(|| {
                Failure::new(
                    EXIT_STEP,
                    format!("docker VM {vm_key} has no backend: docker network"),
                )
            })?;
        let primary = vm
            .networks
            .iter()
            .find(|attachment| {
                environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend != crate::config::NetworkBackend::Docker)
            })
            .ok_or_else(|| {
                Failure::new(
                    EXIT_STEP,
                    format!("docker VM {vm_key} has no primary vmnet network"),
                )
            })?;
        let docker_cidr = environment.spec.networks[&docker_attachment.name]
            .cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|error| Failure::new(EXIT_STEP, format!("docker DNS CIDR: {error}")))?;
        let discovered = crate::docker::collect_vm_container_dns(
            &environment.spec.project,
            &state_dir,
            vm_key,
            &vm.compose_files,
            docker_cidr,
        )
        .map_err(|error| Failure::new(EXIT_STEP, format!("container DNS: {error}")))?;
        dns_records.extend(discovered.into_iter().map(|record| {
            json!({
                "name": record.name,
                "network": docker_attachment.name,
                "listener_network": primary.name,
                "stack": environment.spec.project,
                "project": environment.spec.project,
                "ip": record.ip.to_string(),
            })
        }));
    }
    rpc(
        socket_path,
        "dns.records.ensure",
        json!({
            "project": environment.spec.project,
            "records": dns_records,
        }),
    )?;
    Ok(())
}

fn purge_docker_context(environment: &Environment) -> Result<(), Failure> {
    let has_docker = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "docker"));
    if !has_docker {
        return Ok(());
    }
    let _ = crate::docker::remove_context(&environment.spec.project);
    Ok(())
}

fn ensure_ports(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
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

fn ensure_ca(environment: &Environment) -> Result<(), Failure> {
    let enabled = environment.spec.certs.as_ref().is_some_and(|c| c.enabled)
        || environment.spec.ingress.as_ref().is_some_and(|i| i.enabled)
        || environment.spec.oidc.as_ref().is_some_and(|o| o.enabled);
    if !enabled {
        return Ok(());
    }
    crate::certs::ensure_ca(&crate::state_dir(), false)
        .map_err(|e| Failure::new(EXIT_STEP, format!("ensure CA: {e}")))?;
    Ok(())
}

fn ensure_oidc(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
) -> Result<(), Failure> {
    let Some(oidc) = environment.spec.oidc.as_ref().filter(|o| o.enabled) else {
        return Ok(());
    };
    let state_dir = crate::state_dir();
    let mut vm_names = Vec::new();
    for (name, vm) in &environment.spec.vms {
        if vm.requires.iter().any(|r| r == "oidc") {
            vm_names.push(name.clone());
        }
    }
    let mut route_hosts = Vec::new();
    if let Some(ingress) = &environment.spec.ingress {
        for route in &ingress.routes {
            route_hosts.push((route.host.clone(), route.requires.clone()));
        }
    }
    let clients = crate::oidc::auto_clients(
        &environment.spec.project,
        &environment.spec.domain,
        &vm_names,
        &route_hosts,
    );
    crate::oidc::write_clients(&state_dir, &environment.spec.project, &clients)
        .map_err(|e| Failure::new(EXIT_STEP, format!("oidc clients: {e}")))?;

    let storage = state_dir
        .join("runtime")
        .join("oidc")
        .join(&environment.spec.project);
    fs::create_dir_all(&storage).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;

    match oidc.mode {
        config::OidcMode::OidcSimple => {
            let config_json = crate::oidc::render_simple_config(
                &oidc.issuer,
                &oidc.listen,
                &clients,
                &oidc.users,
            )
            .map_err(|e| Failure::new(EXIT_STEP, format!("oidc-simple config: {e}")))?;
            let binary = state_dir.join("bin").join("vzctl-oidc-simple");
            let binary = if binary.exists() {
                binary
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/debug/vzctl-oidc-simple")
            };
            let binary = if binary.exists() {
                binary
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/release/vzctl-oidc-simple")
            };
            rpc(
                socket_path,
                "oidc.ensure",
                json!({
                    "project": environment.spec.project,
                    "config": config_json,
                    "configName": "config.json",
                    "binary": binary.display().to_string(),
                    "arguments": ["--config", "{config}"],
                    "processName": format!("oidc-simple-{}", environment.spec.project),
                    "pidFile": "oidc.pid",
                    "listen": oidc.listen,
                }),
            )?;
        }
        config::OidcMode::Embedded => {
            let password_file = oidc.password_file.as_ref().map(|rel| {
                config::config_path(config_path)
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(rel)
            });
            let host_uplink = crate::oidc::load_host_uplink(&state_dir)
                .map_err(|e| Failure::new(EXIT_STEP, format!("oidc host uplink: {e}")))?;
            let resolved_uplink = crate::oidc::merge_uplink(
                &state_dir,
                &environment.spec.project,
                host_uplink.as_ref(),
                oidc.uplink.as_ref(),
            )
            .map_err(|e| Failure::new(EXIT_STEP, format!("oidc uplink: {e}")))?;
            let config_yaml = crate::oidc::render_dex_config(
                &oidc.issuer,
                &oidc.listen,
                &clients,
                password_file.as_deref(),
                &storage,
                resolved_uplink.as_ref(),
            )
            .map_err(|e| Failure::new(EXIT_STEP, format!("dex config: {e}")))?;

            let binary = state_dir.join("bin").join("dex");
            let binary = if binary.exists() {
                binary
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../daemon/Vendor/dex/dex")
            };
            rpc(
                socket_path,
                "oidc.ensure",
                json!({
                    "project": environment.spec.project,
                    "config": config_yaml,
                    "configName": "config.yaml",
                    "binary": binary.display().to_string(),
                    "arguments": ["serve", "{config}"],
                    "processName": format!("dex-{}", environment.spec.project),
                    "pidFile": "oidc.pid",
                    "listen": oidc.listen,
                }),
            )?;
        }
    }
    Ok(())
}

fn ensure_ingress(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let Some(ingress) = environment.spec.ingress.as_ref().filter(|i| i.enabled) else {
        return Ok(());
    };
    let state_dir = crate::state_dir();
    // Mint leafs for each route (+ localhost aliases).
    for route in &ingress.routes {
        let mut extras = Vec::new();
        if ingress.host_aliases {
            if let Some(alias) = crate::ingress::short_localhost(&route.host) {
                extras.push(alias);
            }
        }
        crate::certs::mint_leaf(&state_dir, &route.host, &extras)
            .map_err(|e| Failure::new(EXIT_STEP, format!("mint {}: {e}", route.host)))?;
    }

    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    let networks = snapshot["networks"].as_array().cloned().unwrap_or_default();
    let attachments = snapshot["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rendered = crate::ingress::render(environment, &networks, &attachments, &state_dir)
        .map_err(|e| Failure::new(EXIT_STEP, format!("caddyfile: {e}")))?;

    let binary = state_dir.join("bin").join("caddy");
    let binary = if binary.exists() {
        binary
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../daemon/Vendor/caddy/caddy")
    };
    rpc(
        socket_path,
        "ingress.ensure",
        json!({
            "project": environment.spec.project,
            "caddyfile": rendered.caddyfile,
            "binary": binary.display().to_string(),
            "http_port": rendered.http_port,
            "https_port": rendered.https_port,
            "backend_http_port": rendered.caddy_http_port,
            "backend_https_port": rendered.caddy_https_port,
            "gateways": rendered.gateways,
            "gateway_bindings": rendered.gateway_bindings,
        }),
    )?;
    rpc(
        socket_path,
        "dns.host_services.ensure",
        json!({ "project": environment.spec.project, "hosts": rendered.hosts }),
    )?;
    Ok(())
}

fn ensure_ca_rollout(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let enabled = environment.spec.certs.as_ref().is_some_and(|c| c.enabled)
        || environment.spec.ingress.as_ref().is_some_and(|i| i.enabled)
        || environment.spec.oidc.as_ref().is_some_and(|o| o.enabled);
    if !enabled {
        return Ok(());
    }
    let state_dir = crate::state_dir();
    let pem = crate::certs::read_ca_pem(&state_dir).map_err(|e| Failure::new(EXIT_STEP, e))?;
    let fingerprint =
        crate::certs::read_fingerprint(&state_dir).map_err(|e| Failure::new(EXIT_STEP, e))?;
    let targets = environment
        .spec
        .vms
        .keys()
        .map(|name| vm_runtime_id(environment, name))
        .collect::<Vec<_>>();
    rollout_ca_to_vms(&targets, &pem, &fingerprint, |method, params| {
        rpc(socket_path, method, params)
    })
}

const LEGACY_CA_INSTALL_SCRIPT: &str = r#"set -eu
fingerprint=$1
pem_b64=$2
ca_dir=/usr/local/share/ca-certificates
fingerprint_file=/var/lib/vzctl/ca.fingerprint
install -d -m 0755 "$ca_dir" /var/lib/vzctl
pem_tmp=$(mktemp "$ca_dir/.vzctl-local.crt.XXXXXX")
der_tmp=$(mktemp)
fingerprint_tmp=$(mktemp /var/lib/vzctl/.ca.fingerprint.XXXXXX)
trap 'rm -f "$pem_tmp" "$der_tmp" "$fingerprint_tmp"' EXIT
printf '%s' "$pem_b64" | base64 -d >"$pem_tmp"
openssl x509 -in "$pem_tmp" -outform DER >"$der_tmp"
actual=$(openssl dgst -sha256 -r "$der_tmp" | awk '{print $1}')
[ "$actual" = "$fingerprint" ] || {
  echo "CA fingerprint mismatch: expected $fingerprint, got $actual" >&2
  exit 1
}
chmod 0644 "$pem_tmp"
mv -f "$pem_tmp" "$ca_dir/vzctl-local.crt"
/usr/sbin/update-ca-certificates
openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt "$ca_dir/vzctl-local.crt" >/dev/null
printf '%s\n' "$fingerprint" >"$fingerprint_tmp"
chmod 0644 "$fingerprint_tmp"
mv -f "$fingerprint_tmp" "$fingerprint_file"
"#;

fn rollout_ca_to_vms<F>(
    targets: &[String],
    pem: &str,
    fingerprint: &str,
    mut call: F,
) -> Result<(), Failure>
where
    F: FnMut(&str, Value) -> Result<Value, Failure>,
{
    let pem_b64 = base64::engine::general_purpose::STANDARD.encode(pem.as_bytes());
    let mut failures = Vec::new();
    for vm_id in targets {
        let primary = call(
            "vm.agent.ca_inject",
            json!({
                "vm_id": vm_id,
                "pem": pem,
                "fingerprint": fingerprint,
                "name": "vzctl-local",
            }),
        )
        .and_then(|result| validate_ca_inject_result(&result, fingerprint));
        if primary.is_ok() {
            continue;
        }
        let primary_message = primary
            .err()
            .map(|failure| failure.message)
            .unwrap_or_else(|| "unknown ca_inject failure".to_string());
        let fallback = call(
            "vm.exec",
            json!({
                "vm_id": vm_id,
                "cmd": [
                    "sudo",
                    "-n",
                    "sh",
                    "-c",
                    LEGACY_CA_INSTALL_SCRIPT,
                    "vzctl-ca-fallback",
                    fingerprint,
                    pem_b64,
                ],
                "timeout_ms": 60_000,
            }),
        )
        .and_then(validate_ca_exec_result);
        if let Err(failure) = fallback {
            failures.push(format!(
                "{vm_id}: ca_inject failed ({primary_message}); compatibility fallback failed ({})",
                failure.message
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_STEP,
            format!("CA rollout failed: {}", failures.join("; ")),
        ))
    }
}

fn validate_ca_inject_result(result: &Value, fingerprint: &str) -> Result<Value, Failure> {
    if result["installed"] == true && result["fingerprint"].as_str() == Some(fingerprint) {
        Ok(result.clone())
    } else {
        Err(Failure::new(
            EXIT_STEP,
            "ca_inject returned an invalid result",
        ))
    }
}

fn validate_ca_exec_result(result: Value) -> Result<Value, Failure> {
    if result["exit"].as_i64() == Some(0) {
        return Ok(result);
    }
    let detail = result["stderr"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| result["stdout"].as_str())
        .unwrap_or("missing vm.exec result");
    Err(Failure::new(
        EXIT_STEP,
        format!("guest command exited with {}: {detail}", result["exit"]),
    ))
}

fn ensure_oidc_inject(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let Some(oidc) = environment.spec.oidc.as_ref().filter(|o| o.enabled) else {
        return Ok(());
    };
    let path = crate::oidc::clients_path(&crate::state_dir(), &environment.spec.project);
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;
    let clients: Value =
        serde_json::from_str(&raw).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;
    let list = clients["clients"].as_array().cloned().unwrap_or_default();
    for client in list {
        let id = client["id"].as_str().unwrap_or("client");
        let secret = client["secret"].as_str().unwrap_or("");
        let redirect = client["redirectURIs"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !environment.spec.vms.contains_key(id) {
            continue;
        }
        let runtime_id = vm_runtime_id(environment, id);
        let env_block = format!(
            "OIDC_ISSUER={}\nOIDC_CLIENT_ID={id}\nOIDC_CLIENT_SECRET={secret}\n\
             OIDC_REDIRECT_URI={redirect}\nOIDC_CA_PATH=/etc/ssl/certs/ca-certificates.crt\n",
            oidc.issuer
        );
        let script = format!(
            "mkdir -p /etc/vzctl && cat > /etc/vzctl/oidc.env <<'EOF'\n{env_block}EOF\nchmod 600 /etc/vzctl/oidc.env"
        );
        let _ = rpc(
            socket_path,
            "vm.exec",
            json!({
                "vm_id": runtime_id,
                "cmd": ["bash", "-lc", script],
                "timeout_ms": 30_000,
            }),
        );
    }
    Ok(())
}

fn purge_ingress(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let _ = rpc(
        socket_path,
        "ingress.purge",
        json!({ "project": environment.spec.project }),
    );
    Ok(())
}

fn purge_dns_records(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
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

fn purge_oidc(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let _ = rpc(
        socket_path,
        "oidc.purge",
        json!({ "project": environment.spec.project }),
    );
    Ok(())
}

fn purge_ports(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
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

fn remove_managed_vm(vm_id: &str) -> Result<(), Failure> {
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

/// True when an existing bundle already reflects the desired VM roles and
/// vmnet NIC addresses (docker-backend nets are logical and not in identity).
fn bundle_matches_vm(bundle: &Path, vm: &VmConfig) -> Result<bool, Failure> {
    let manifest = bundle.join("vm.json");
    if !manifest.is_file() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(&fs::read(&manifest).map_err(|error| {
        Failure::new(EXIT_STEP, format!("read {}: {error}", manifest.display()))
    })?)
    .map_err(|error| Failure::new(EXIT_STEP, format!("parse {}: {error}", manifest.display())))?;
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

fn apply_routes(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
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
            json!({"router": null, "policies": environment.spec.policies}),
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

fn load_config(path: &Path) -> Result<Environment, Failure> {
    let path = config::config_path(path);
    config::validate_path(&path).map_err(|issues| {
        Failure::new(
            EXIT_INVALID,
            issues
                .iter()
                .map(|issue| format!("{}: {}", issue.path, issue.message))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })
}

fn desired_resources(environment: &Environment) -> Result<Vec<Resource>, Failure> {
    let mut resources = Vec::new();
    let project = environment.spec.project.clone();
    let stack = stack_id(environment);
    let mut add = |kind: &str, name: &str, spec: Value, state: &str| -> Result<(), Failure> {
        let spec = serde_json::to_string(&spec).map_err(|error| {
            Failure::new(EXIT_STEP, format!("serialize desired state: {error}"))
        })?;
        resources.push(Resource {
            kind: kind.to_string(),
            name: name.to_string(),
            labels: BTreeMap::from([
                ("managed-by".to_string(), "vzctl".to_string()),
                ("project".to_string(), project.clone()),
                ("stack_id".to_string(), stack.clone()),
                ("spec".to_string(), spec),
            ]),
            state: state.to_string(),
        });
        Ok(())
    };
    add(
        "dns",
        &environment.spec.domain,
        json!(environment.spec.dns),
        "active",
    )?;
    for (name, image) in &environment.spec.images {
        add("image", name, json!(image), "sealed")?;
    }
    for (name, network) in &environment.spec.networks {
        add("network", name, json!(network), "active")?;
    }
    for (name, vm) in &environment.spec.vms {
        add(
            "vm",
            &vm_runtime_id(environment, name),
            json!(vm),
            "running",
        )?;
    }
    for route in &environment.spec.routes {
        add("route", &route.name, json!(route), "active")?;
    }
    for policy in &environment.spec.policies {
        add("policy", &policy.name, json!(policy), "active")?;
    }
    resources.sort_by(|left, right| (&left.kind, &left.name).cmp(&(&right.kind, &right.name)));
    Ok(resources)
}

fn actual_resources(state: &Value) -> Result<Vec<Resource>, Failure> {
    serde_json::from_value(state["resources"].clone())
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("invalid stack state: {error}")))
}

fn journal_context(state: &Value) -> Result<(Mode, bool), Failure> {
    let payload = state["journal"]["payload"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_INCOMPLETE, "incomplete journal has no payload"))?;
    let payload: Value = serde_json::from_str(payload).map_err(|error| {
        Failure::new(EXIT_INCOMPLETE, format!("invalid journal payload: {error}"))
    })?;
    let mode = match payload["mode"].as_str() {
        Some("up") => Mode::Up,
        Some("apply") => Mode::Apply,
        Some("down") => Mode::Down,
        Some(other) => {
            return Err(Failure::new(
                EXIT_INCOMPLETE,
                format!("cannot resume journal mode {other}"),
            ))
        }
        None => {
            return Err(Failure::new(
                EXIT_INCOMPLETE,
                "incomplete journal has no mode",
            ))
        }
    };
    Ok((mode, payload["purge"].as_bool().unwrap_or(false)))
}

fn build_plan(mode: Mode, desired: Vec<Resource>, actual: Vec<Resource>) -> Plan {
    let desired_map = desired
        .iter()
        .map(|resource| ((resource.kind.clone(), resource.name.clone()), resource))
        .collect::<BTreeMap<_, _>>();
    let actual_map = actual
        .iter()
        .map(|resource| ((resource.kind.clone(), resource.name.clone()), resource))
        .collect::<BTreeMap<_, _>>();
    let mut actions = Vec::new();
    for (key, wanted) in &desired_map {
        match actual_map.get(key) {
            None => actions.push(PlanAction {
                action: "create",
                kind: wanted.kind.clone(),
                name: wanted.name.clone(),
                breaking: false,
                reason: "missing from actual state".to_string(),
            }),
            Some(current) => {
                if current.labels.get("spec") != wanted.labels.get("spec") && mode != Mode::Up {
                    actions.push(PlanAction {
                        action: "update",
                        kind: wanted.kind.clone(),
                        name: wanted.name.clone(),
                        breaking: matches!(wanted.kind.as_str(), "network" | "vm"),
                        reason: "desired spec differs from actual state".to_string(),
                    });
                } else if wanted.kind == "vm" && current.state == "stopped" && mode != Mode::Down {
                    actions.push(PlanAction {
                        action: "start",
                        kind: wanted.kind.clone(),
                        name: wanted.name.clone(),
                        breaking: false,
                        reason: "VM is stopped".to_string(),
                    });
                }
            }
        }
    }
    if !matches!(mode, Mode::Up | Mode::Down) {
        for (key, current) in &actual_map {
            if !desired_map.contains_key(key) {
                actions.push(PlanAction {
                    action: "delete",
                    kind: current.kind.clone(),
                    name: current.name.clone(),
                    breaking: true,
                    reason: "managed resource absent from desired state".to_string(),
                });
            }
        }
    }
    if mode == Mode::Down {
        for vm in desired.iter().filter(|resource| resource.kind == "vm") {
            actions.push(PlanAction {
                action: "stop",
                kind: "vm".to_string(),
                name: vm.name.clone(),
                breaking: false,
                reason: "down stops VMs in reverse dependency order".to_string(),
            });
        }
    }
    actions.sort_by(|left, right| {
        (&left.kind, &left.name, left.action).cmp(&(&right.kind, &right.name, right.action))
    });
    Plan { actions, desired }
}

fn dependency_order(vms: &BTreeMap<String, VmConfig>) -> Result<Vec<String>, Failure> {
    fn visit(
        name: &str,
        vms: &BTreeMap<String, VmConfig>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        result: &mut Vec<String>,
    ) -> Result<(), Failure> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            return Err(Failure::new(EXIT_INVALID, "VM dependency cycle"));
        }
        for dependency in &vms[name].depends_on {
            visit(dependency, vms, visiting, visited, result)?;
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        result.push(name.to_string());
        Ok(())
    }
    let mut result = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for name in vms.keys() {
        visit(name, vms, &mut visiting, &mut visited, &mut result)?;
    }
    Ok(result)
}

fn data_disk_gib(value: &str) -> Result<u64, Failure> {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| Failure::new(EXIT_INVALID, format!("invalid dataDisk: {value}")))?;
    let number = value[..split]
        .parse::<u64>()
        .map_err(|_| Failure::new(EXIT_INVALID, format!("invalid dataDisk: {value}")))?;
    let unit = value[split..].to_ascii_lowercase();
    match unit.as_str() {
        "g" | "gb" | "gib" => Ok(number),
        "t" | "tb" | "tib" => number
            .checked_mul(1024)
            .ok_or_else(|| Failure::new(EXIT_INVALID, "dataDisk is too large")),
        _ => Err(Failure::new(
            EXIT_INVALID,
            format!("dataDisk must use GiB/TiB for VM creation: {value}"),
        )),
    }
}

fn memory_mib(value: &str) -> Result<u64, Failure> {
    crate::parse_memory_mib(value).map_err(|message| Failure::new(EXIT_INVALID, message))
}

fn checkpoint(
    socket_path: &Path,
    id: &str,
    stack_id: &str,
    holder: &str,
    step: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), Failure> {
    rpc(
        socket_path,
        "stack.step",
        json!({
            "id": id,
            "stack_id": stack_id,
            "holder": holder,
            "step": step,
            "status": status,
            "error": error,
        }),
    )
    .map(|_| ())
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("connect {}: {error}", socket_path.display()),
        )
    })?;
    let timeout_secs = rpc_timeout_secs(method, &params);
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout_secs)))
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(timeout_secs.min(10))))
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    let request = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
    writeln!(stream, "{request}")
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("{method}: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("{method}: {error}")))?;
    let response: Value = serde_json::from_str(&line)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("{method}: {error}")))?;
    if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
        let code = match error["code"].as_i64() {
            Some(5) => EXIT_INCOMPLETE,
            Some(6) => EXIT_LEASE,
            _ => EXIT_STEP,
        };
        return Err(Failure::new(
            code,
            error["message"].as_str().unwrap_or("supervisor error"),
        ));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, format!("{method}: missing result")))
}

fn rpc_timeout_secs(method: &str, params: &Value) -> u64 {
    if method == "vm.exec" || method.starts_with("vm.agent.") {
        params
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(|ms| (ms / 1000).saturating_add(10).max(15))
            .unwrap_or(40)
    } else {
        10
    }
}

fn run_self(args: &[&str], progress: &mut ProgressReporter) -> Result<Value, Failure> {
    let executable = std::env::current_exe()
        .map_err(|error| Failure::new(EXIT_STEP, format!("resolve vzctl executable: {error}")))?;
    let pipe_stderr = progress.pipe_subprocess_stderr();
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if pipe_stderr {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .map_err(|error| {
            Failure::new(EXIT_STEP, format!("run vzctl {}: {error}", args.join(" ")))
        })?;
    let stderr_thread = if pipe_stderr {
        child.stderr.take().map(|stderr| {
            let tx = progress.log_sender();
            let job_id = progress.current_job_id();
            let job_path = progress.current_job_path();
            let plain_percent = progress.plain_percent();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(tx) = &tx {
                        let _ = tx.send(ProgressMessage::Log {
                            job_id: job_id.clone(),
                            line,
                        });
                    } else if let Some(percent) = plain_percent {
                        progress::print_plain_subprocess_line(percent, job_path.as_deref(), &line);
                    }
                }
            })
        })
    } else {
        None
    };
    let mut stdout = Vec::new();
    child
        .stdout
        .as_mut()
        .ok_or_else(|| {
            Failure::new(
                EXIT_STEP,
                format!("vzctl {}: missing stdout", args.join(" ")),
            )
        })?
        .read_to_end(&mut stdout)
        .map_err(|error| {
            Failure::new(
                EXIT_STEP,
                format!("read vzctl {} stdout: {error}", args.join(" ")),
            )
        })?;
    let status = child.wait().map_err(|error| {
        Failure::new(EXIT_STEP, format!("wait vzctl {}: {error}", args.join(" ")))
    })?;
    if let Some(handle) = stderr_thread {
        let _ = handle.join();
    }
    if !status.success() {
        let message = serde_json::from_slice::<Value>(&stdout)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/summary/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("vzctl {} failed", args.join(" ")));
        return Err(Failure::new(
            status.code().unwrap_or(EXIT_STEP as i32) as u8,
            message,
        ));
    }
    serde_json::from_slice(&stdout).map_err(|error| {
        Failure::new(
            EXIT_STEP,
            format!("vzctl {} returned invalid JSON: {error}", args.join(" ")),
        )
    })
}

/// Like `run_self`, but retries via macOS Admin dialog when `/etc/resolver` needs root.
fn run_self_privileged(args: &[&str], progress: &mut ProgressReporter) -> Result<Value, Failure> {
    match run_self(args, progress) {
        Ok(value) => Ok(value),
        Err(failure) if failure_needs_dns_elevation(&failure) => run_self_elevated(args),
        Err(failure) => Err(failure),
    }
}

fn failure_needs_dns_elevation(failure: &Failure) -> bool {
    failure.code == crate::dns::EXIT_RESOLVER
        || failure.message.contains("Permission denied")
        || failure.message.contains("run this command with sudo")
        || failure.message.contains("os error 13")
}

fn run_self_elevated(args: &[&str]) -> Result<Value, Failure> {
    let executable = std::env::current_exe()
        .map_err(|error| Failure::new(EXIT_STEP, format!("resolve vzctl executable: {error}")))?;
    let shell = std::iter::once(executable.to_string_lossy().as_ref())
        .chain(args.iter().copied())
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        escape_applescript(&shell)
    );
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| Failure::new(EXIT_STEP, format!("osascript elevate: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Failure::new(
            crate::dns::EXIT_RESOLVER,
            if stderr.is_empty() {
                format!(
                    "Admin elevation failed or cancelled for: vzctl {}",
                    args.join(" ")
                )
            } else {
                stderr
            },
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.get("status").and_then(|s| s.as_str()) == Some("fail")
            || value
                .get("exit_code")
                .and_then(|c| c.as_u64())
                .is_some_and(|c| c != 0)
        {
            let message = value
                .pointer("/summary/message")
                .and_then(Value::as_str)
                .unwrap_or("elevated dns command failed")
                .to_string();
            return Err(Failure::new(crate::dns::EXIT_RESOLVER, message));
        }
        return Ok(value);
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start..=end]) {
                    return Ok(value);
                }
            }
        }
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "status": "ok",
        "exit_code": 0,
        "summary": {"message": if trimmed.is_empty() { "elevated ok" } else { trimmed }},
    }))
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".into();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:@+=".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse(mode: &str, mut args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let mode = match mode {
        "plan" => Mode::Plan,
        "diff" => Mode::Diff,
        "up" => Mode::Up,
        "apply" => Mode::Apply,
        "down" => Mode::Down,
        "adopt" => Mode::Adopt,
        _ => return Err(Failure::new(EXIT_USAGE, "unknown reconcile command")),
    };
    let mut config = PathBuf::from(".");
    let mut format = Format::Human;
    let mut force = false;
    let mut resume = false;
    let mut abort = false;
    let mut purge = false;
    let mut progress_explicit: Option<ProgressMode> = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-C" | "--config" => {
                config = PathBuf::from(
                    args.next()
                        .ok_or_else(|| Failure::new(EXIT_USAGE, "-C requires a path"))?,
                );
            }
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    _ => return Err(Failure::new(EXIT_USAGE, "--format requires human or json")),
                }
            }
            "--progress" if matches!(mode, Mode::Apply | Mode::Up | Mode::Down) => {
                let value = args.next().ok_or_else(|| {
                    Failure::new(EXIT_USAGE, "--progress requires plain, ui, or off")
                })?;
                progress_explicit = Some(
                    parse_progress_flag(Some(&value))
                        .map_err(|message| Failure::new(EXIT_USAGE, message))?,
                );
            }
            "--force" if matches!(mode, Mode::Apply | Mode::Up) => force = true,
            "--resume" if mode == Mode::Apply => resume = true,
            "--abort" if mode == Mode::Apply => abort = true,
            "--purge" if mode == Mode::Down => purge = true,
            _ => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown {} option: {argument}", mode.command()),
                ))
            }
        }
    }
    if resume && abort {
        return Err(Failure::new(
            EXIT_INVALID,
            "apply accepts only one of --resume or --abort",
        ));
    }
    Ok(Options {
        mode,
        config,
        format,
        force,
        resume,
        abort,
        purge,
        progress: resolve_progress_mode(
            progress_explicit,
            format == Format::Human,
            matches!(mode, Mode::Up | Mode::Apply),
        ),
    })
}

fn confirm_breaking(actions: &[PlanAction]) -> Result<bool, Failure> {
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    eprintln!("Breaking changes:");
    for action in actions.iter().filter(|action| action.breaking) {
        eprintln!("  {} {} {}", action.action, action.kind, action.name);
    }
    eprint!("Continue? [y/N] ");
    std::io::stderr()
        .flush()
        .map_err(|error| Failure::new(EXIT_INVALID, error.to_string()))?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| Failure::new(EXIT_INVALID, error.to_string()))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| Format::Json)
        .unwrap_or(Format::Human)
}

fn adopt_report(
    stack_id: &str,
    environment: &Environment,
    desired: &[Resource],
    actual: &[Resource],
    socket_path: &Path,
) -> Result<Value, Failure> {
    let mut candidates = BTreeSet::new();
    for resource in desired.iter().chain(actual.iter()) {
        if resource.kind == "vm" {
            candidates.insert(resource.name.clone());
        }
    }
    let project = environment.spec.project.as_str();
    for (vm_id, manifest_path) in discover_vm_manifests(&crate::state_dir().join("vms")) {
        let Ok(raw) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if manifest["managed-by"] != "vzctl" {
            continue;
        }
        let labels = &manifest["labels"];
        let matches_stack = labels["stack_id"].as_str() == Some(stack_id)
            || (labels["project"].as_str() == Some(project) && candidates.contains(&vm_id));
        if matches_stack || candidates.contains(&vm_id) {
            candidates.insert(vm_id);
        }
    }

    let runtime = match rpc(socket_path, "vm.list", json!({})) {
        Ok(records) => records,
        Err(failure) if failure.code == EXIT_SUPERVISOR => json!([]),
        Err(failure) => return Err(failure),
    };
    let live: BTreeSet<String> = runtime
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| matches!(record["state"].as_str(), Some("starting") | Some("running")))
        .filter_map(|record| record["vm_id"].as_str().map(str::to_string))
        .collect();

    let mut actions = Vec::new();
    for vm_id in &candidates {
        if live.contains(vm_id) {
            continue;
        }
        let lock_path = helper_lock_path(vm_id);
        if !lock_path.is_file() {
            continue;
        }
        if !is_safe_stale_helper_lock(&lock_path) {
            continue;
        }
        actions.push(json!({
            "action": "report",
            "kind": "helper-lock",
            "name": vm_id,
            "breaking": false,
            "reason": "stale helper lock (pid dead or flock free)",
        }));
    }

    let changed = !actions.is_empty();
    let message = if changed {
        format!("{} stale helper lock(s) reported", actions.len())
    } else {
        "no lockfile-only resources adopted".to_string()
    };
    Ok(json!({
        "message": message,
        "stack_id": stack_id,
        "actions": actions,
        "changed": changed,
        "minimal": !changed,
    }))
}

fn helper_lock_path(vm_id: &str) -> PathBuf {
    crate::state_dir()
        .join("helpers")
        .join(format!("{}.lock", state_file_component(vm_id)))
}

fn discover_vm_manifests(vms_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(vms_dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let manifest = path.join("vm.json");
        if manifest.is_file() {
            found.push((name, manifest));
            continue;
        }
        // Nested project/vm bundles: vms/{project}/{vm}/vm.json
        let Ok(children) = fs::read_dir(&path) else {
            continue;
        };
        for child in children.flatten() {
            let child_path = child.path();
            if !child_path.is_dir() {
                continue;
            }
            let child_name = child.file_name().to_string_lossy().to_string();
            let nested_manifest = child_path.join("vm.json");
            if nested_manifest.is_file() {
                found.push((format!("{name}/{child_name}"), nested_manifest));
            }
        }
    }
    found
}

fn vm_runtime_id(environment: &Environment, config_name: &str) -> String {
    crate::runtime_vm_id(&environment.spec.project, config_name)
}

fn state_file_component(value: &str) -> String {
    let prefix: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let hash = value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{prefix}-{hash:x}")
}

fn is_safe_stale_helper_lock(path: &Path) -> bool {
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let fd = file.as_raw_fd();
    let locked = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if locked == 0 {
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        return true;
    }
    // Lock held — only treat as stale if recorded PID is dead.
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<i32>() else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return false;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn stack_id(environment: &Environment) -> String {
    format!("{}:{}", environment.spec.project, environment.metadata.name)
}

fn holder() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
    format!("{host}:{}", std::process::id())
}

fn desired_hash(resources: &[Resource]) -> Result<String, Failure> {
    let bytes = serde_json::to_vec(resources)
        .map_err(|error| Failure::new(EXIT_STEP, format!("hash desired state: {error}")))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn plan_output(stack_id: &str, plan: &Plan) -> Value {
    json!({
        "message": if plan.actions.is_empty() { "no changes" } else { "changes planned" },
        "stack_id": stack_id,
        "actions": plan.actions,
        "changed": !plan.actions.is_empty(),
        "breaking": plan.actions.iter().filter(|action| action.breaking).count(),
    })
}

fn emit_success(options: &Options, output: &Value) {
    let envelope = json!({
        "apiVersion": API_VERSION,
        "command": options.mode.command(),
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": output["message"],
            "changed": output["changed"],
            "actions": output["actions"].as_array().map(Vec::len).unwrap_or(0),
        },
        "stack_id": output["stack_id"],
        "journal": output.get("journal").cloned().unwrap_or(Value::Null),
        "actions": output["actions"],
    });
    match options.format {
        Format::Json => println!("{envelope}"),
        Format::Human => {
            println!("{}", output["message"].as_str().unwrap_or("ok"));
            for action in output["actions"].as_array().into_iter().flatten() {
                println!(
                    "{} {} {}{}",
                    action["action"].as_str().unwrap_or("?"),
                    action["kind"].as_str().unwrap_or("?"),
                    action["name"].as_str().unwrap_or("?"),
                    if action["breaking"] == true {
                        " [breaking]"
                    } else {
                        ""
                    }
                );
            }
        }
    }
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    eprintln!("{}", failure.message);
    if format == Format::Json {
        println!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": command,
                "status": "fail",
                "exit_code": failure.code,
                "summary": {"message": failure.message},
                "actions": [],
            })
        );
    }
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    monitor_cloud_init_with(
                        vm_id,
                        Duration::from_secs(1),
                        Duration::ZERO,
                        tx,
                        || {
                            Ok(CloudInitObservation {
                                state: CloudInitState::Done,
                                stage: None,
                                datasource: None,
                            })
                        },
                    );
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
        assert_eq!(data_disk_gib("4G").unwrap(), 4);
        assert_eq!(data_disk_gib("2TiB").unwrap(), 2048);
        assert!(data_disk_gib("512M").is_err());
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
}
