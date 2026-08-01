use crate::config::{self, Environment, VmConfig};
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
use std::time::{Duration, Instant};

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_INCOMPLETE: u8 = crate::EXIT_INCOMPLETE_JOURNAL;
const EXIT_LEASE: u8 = crate::EXIT_LEASE_HELD;
const EXIT_SUPERVISOR: u8 = 10;
const EXIT_STEP: u8 = 24;

const APPLY_STEPS: &[&str] = &[
    "validate",
    "acquire_lease",
    "ensure_nets",
    "ensure_dns",
    "ensure_images",
    "ensure_vms",
    "attach_nets",
    "start_helpers",
    "await_agents",
    "ensure_ca",
    "ensure_oidc",
    "ensure_ingress",
    "ensure_ca_rollout",
    "ensure_oidc_inject",
    "ensure_docker_context",
    "ensure_ports",
    "apply_routes_policies",
    "release_lease",
];

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
    match run(&options, socket_path) {
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

fn run(options: &Options, socket_path: &Path) -> Result<Value, Failure> {
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
        vec![
            "purge_ingress",
            "purge_oidc",
            "stop_helpers",
            "detach_nets",
            "destroy_managed",
            "purge_docker_context",
            "purge_ports",
            "dns_cleanup",
            "release_lease",
        ]
    } else {
        APPLY_STEPS.to_vec()
    };

    let mut reached_resume = resume_step.is_none();
    for step in steps {
        if !reached_resume {
            reached_resume = resume_step.as_deref() == Some(step);
            if !reached_resume {
                continue;
            }
        }
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
        if let Err(failure) =
            execute_step(step, &execution_options, &environment, &plan, socket_path)
        {
            let _ = checkpoint(
                socket_path,
                &journal_id,
                &stack_id,
                &holder,
                step,
                "failed",
                Some(&failure.message),
            );
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
    Ok(json!({
        "message": format!("{} completed", effective_mode.command()),
        "stack_id": stack_id,
        "journal": {"id": journal_id, "status": "done"},
        "actions": plan.actions,
        "changed": !plan.actions.is_empty() || options.mode == Mode::Down,
    }))
}

fn execute_step(
    step: &str,
    options: &Options,
    environment: &Environment,
    plan: &Plan,
    socket_path: &Path,
) -> Result<(), Failure> {
    match step {
        "ensure_nets" => ensure_networks(environment, options.force, socket_path),
        "ensure_dns" => ensure_dns(environment, &options.config),
        "ensure_images" => ensure_images(environment),
        "ensure_vms" => ensure_vms(
            environment,
            options.force,
            plan,
            &options.config,
            socket_path,
        ),
        "attach_nets" => {
            ensure_attachments(environment, options.mode, socket_path)?;
            prune_networks(environment, options.mode, plan, socket_path)
        }
        "start_helpers" => start_helpers(environment, socket_path),
        "await_agents" => await_helpers(environment, socket_path),
        "ensure_ca" => ensure_ca(environment),
        "ensure_oidc" => ensure_oidc(environment, &options.config, socket_path),
        "ensure_ingress" => ensure_ingress(environment, socket_path),
        "ensure_ca_rollout" => ensure_ca_rollout(environment, socket_path),
        "ensure_oidc_inject" => ensure_oidc_inject(environment, socket_path),
        "ensure_docker_context" => ensure_docker_context(environment),
        "ensure_ports" => ensure_ports(environment, socket_path),
        "apply_routes_policies" => apply_routes(environment, socket_path),
        "purge_ingress" => purge_ingress(environment, socket_path),
        "purge_oidc" => purge_oidc(environment, socket_path),
        "stop_helpers" => stop_helpers(environment, socket_path),
        "detach_nets" if options.purge => detach_networks(environment, socket_path),
        "destroy_managed" if options.purge => purge_managed(environment, socket_path),
        "purge_docker_context" if options.purge => purge_docker_context(environment),
        "purge_ports" if options.purge => purge_ports(environment, socket_path),
        "dns_cleanup" if options.purge && environment.spec.dns.host_resolver => run_self(&[
            "dns",
            "uninstall-resolver",
            "--project",
            &environment.spec.project,
            "--format",
            "json",
        ])
        .map(|_| ()),
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
            if existing["cidr"] != network.cidr
                || existing["mode"] != mode
                || existing_nat != network.nat_egress
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
                "labels": {"managed-by": "vzctl"},
                "project": environment.spec.project,
                "stack": stack_id(environment),
            }),
        )?;
    }
    Ok(())
}

fn ensure_dns(environment: &Environment, config_path: &Path) -> Result<(), Failure> {
    if environment.spec.dns.host_resolver {
        let path = config::config_path(config_path)
            .to_string_lossy()
            .into_owned();
        run_self(&[
            "dns",
            "install-resolver",
            "--config",
            &path,
            "--format",
            "json",
        ])
        .map(|_| ())
    } else {
        run_self(&[
            "dns",
            "uninstall-resolver",
            "--project",
            &environment.spec.project,
            "--format",
            "json",
        ])
        .map(|_| ())
    }
}

fn ensure_images(environment: &Environment) -> Result<(), Failure> {
    for image in environment.spec.images.values() {
        let existing = crate::image::resolve_alias(&crate::images_dir(), &image.from)
            .map_err(|error| Failure::new(EXIT_STEP, error))?
            .is_some();
        if !existing {
            run_self(&["image", "pull", &image.from, "--format", "json"])?;
        }
        run_self(&["image", "bake", &image.from, "--format", "json"])?;
        run_self(&["image", "seal", &image.from, "--format", "json"])?;
    }
    Ok(())
}

fn ensure_vms(
    environment: &Environment,
    force: bool,
    plan: &Plan,
    config_path: &Path,
    socket_path: &Path,
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
        let vm = &environment.spec.vms[&name];
        let action = plan
            .actions
            .iter()
            .find(|action| action.kind == "vm" && action.name == runtime_id);
        if action.is_some_and(|action| action.action == "update" && action.breaking) {
            if !force {
                return Err(Failure::new(EXIT_INVALID, "VM recreate requires --force"));
            }
            stop_one(&runtime_id, socket_path)?;
            wait_stopped(&runtime_id, socket_path)?;
            remove_managed_vm(&runtime_id)?;
        }
        let bundle = crate::state_dir().join("vms").join(&runtime_id);
        if bundle.join("vm.json").is_file() {
            continue;
        }
        let image_name = &environment.spec.images[&vm.from].from;
        let size = data_disk_gib(&vm.data_disk)?;
        let mut owned = vec![
            "vm".to_string(),
            "create".to_string(),
            runtime_id.clone(),
            "--from".to_string(),
            image_name.clone(),
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
        owned.extend(["--format".to_string(), "json".to_string()]);
        let refs = owned.iter().map(String::as_str).collect::<Vec<_>>();
        run_self(&refs)?;
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
            vm.networks.iter().map(move |network| {
                (
                    runtime_id.clone(),
                    network.name.clone(),
                    network.ip.clone(),
                )
            })
        })
        .collect::<BTreeSet<_>>();
    // Up and apply both converge attachments. Detach stack-owned drift first so
    // IP moves (auto-allocated create → desired static IP) cannot collide.
    if matches!(mode, Mode::Apply | Mode::Up) {
        for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
            let current = (
                attachment["vm_id"].as_str().unwrap_or_default().to_string(),
                attachment["network"].as_str().unwrap_or_default().to_string(),
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
                if vm.roles.iter().any(|role| role == "docker") {
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

fn start_helpers(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    for name in dependency_order(&environment.spec.vms)? {
        let runtime_id = vm_runtime_id(environment, &name);
        let bundle = crate::state_dir().join("vms").join(&runtime_id);
        rpc(
            socket_path,
            "vm.start",
            json!({"vm_id": runtime_id, "bundle": bundle}),
        )?;
    }
    Ok(())
}

fn await_helpers(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let wanted = environment
        .spec
        .vms
        .keys()
        .map(|name| vm_runtime_id(environment, name))
        .collect::<BTreeSet<_>>();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let records = rpc(socket_path, "vm.list", json!({}))?;
        let running = records
            .as_array()
            .into_iter()
            .flatten()
            .filter(|record| record["state"] == "running")
            .filter_map(|record| record["vm_id"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>();
        if wanted.is_subset(&running) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let missing = wanted.difference(&running).cloned().collect::<Vec<_>>();
            return Err(Failure::new(
                EXIT_STEP,
                format!("helpers/agents not ready: {}", missing.join(", ")),
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn stop_helpers(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let mut order = dependency_order(&environment.spec.vms)?;
    order.reverse();
    for name in order {
        let runtime_id = vm_runtime_id(environment, &name);
        stop_one(&runtime_id, socket_path)?;
        wait_stopped(&runtime_id, socket_path)?;
    }
    Ok(())
}

fn stop_one(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
    rpc(socket_path, "vm.stop", json!({"vm_id": vm_id})).map(|_| ())
}

fn wait_stopped(vm_id: &str, socket_path: &Path) -> Result<(), Failure> {
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

fn purge_managed(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    for name in environment.spec.vms.keys() {
        remove_managed_vm(&vm_runtime_id(environment, name))?;
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
    let enabled = environment
        .spec
        .certs
        .as_ref()
        .is_some_and(|c| c.enabled)
        || environment
            .spec
            .ingress
            .as_ref()
            .is_some_and(|i| i.enabled)
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

    let password_file = oidc.password_file.as_ref().map(|rel| {
        config::config_path(config_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(rel)
    });
    let storage = state_dir
        .join("runtime")
        .join("oidc")
        .join(&environment.spec.project);
    fs::create_dir_all(&storage).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;
    let config_yaml = crate::oidc::render_dex_config(
        &oidc.issuer,
        &oidc.listen,
        &clients,
        password_file.as_deref(),
        &storage,
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
            "binary": binary.display().to_string(),
        }),
    )?;
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
    let attachments = snapshot["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rendered = crate::ingress::render(environment, &attachments, &state_dir)
        .map_err(|e| Failure::new(EXIT_STEP, format!("caddyfile: {e}")))?;

    rpc(
        socket_path,
        "dns.host_services.ensure",
        json!({ "hosts": rendered.hosts }),
    )?;

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
            "http_port": ingress.http_port,
            "https_port": ingress.https_port,
            "gateways": rendered.gateways,
        }),
    )?;
    Ok(())
}

fn ensure_ca_rollout(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let enabled = environment
        .spec
        .certs
        .as_ref()
        .is_some_and(|c| c.enabled)
        || environment
            .spec
            .ingress
            .as_ref()
            .is_some_and(|i| i.enabled)
        || environment.spec.oidc.as_ref().is_some_and(|o| o.enabled);
    if !enabled {
        return Ok(());
    }
    let state_dir = crate::state_dir();
    let pem = crate::certs::read_ca_pem(&state_dir)
        .map_err(|e| Failure::new(EXIT_STEP, e))?;
    let fingerprint = crate::certs::read_fingerprint(&state_dir)
        .map_err(|e| Failure::new(EXIT_STEP, e))?;
    for name in environment.spec.vms.keys() {
        let runtime_id = vm_runtime_id(environment, name);
        // Best-effort live inject via supervisor → helper → agent exec.
        let script = format!(
            "mkdir -p /usr/local/share/ca-certificates /var/lib/vzctl && \
             cat > /usr/local/share/ca-certificates/vzctl-local.crt <<'EOF'\n{pem}\nEOF\n\
             echo {fingerprint} > /var/lib/vzctl/ca.fingerprint && \
             (sudo -n /usr/sbin/update-ca-certificates || update-ca-certificates || true)"
        );
        let _ = rpc(
            socket_path,
            "vm.exec",
            json!({
                "vm_id": runtime_id,
                "cmd": ["bash", "-lc", script],
                "timeout_ms": 60_000,
            }),
        );
    }
    Ok(())
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

fn apply_routes(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    if environment.spec.routes.is_empty() && environment.spec.policies.is_empty() {
        return Ok(());
    }
    rpc(
        socket_path,
        "route.apply",
        json!({"router": null, "policies": environment.spec.policies}),
    )
    .map(|_| ())
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
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
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

fn run_self(args: &[&str]) -> Result<Value, Failure> {
    let executable = std::env::current_exe()
        .map_err(|error| Failure::new(EXIT_STEP, format!("resolve vzctl executable: {error}")))?;
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| {
            Failure::new(EXIT_STEP, format!("run vzctl {}: {error}", args.join(" ")))
        })?;
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
