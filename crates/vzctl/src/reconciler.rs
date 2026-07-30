use crate::config::{self, Environment, VmConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
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
    let plan = build_plan(effective_mode, desired, actual);

    if matches!(options.mode, Mode::Plan | Mode::Diff) {
        return Ok(plan_output(&stack_id, &plan));
    }
    if options.mode == Mode::Adopt {
        return Ok(json!({
            "message": "no lockfile-only resources adopted",
            "stack_id": stack_id,
            "actions": [],
            "changed": false,
            "minimal": true,
        }));
    }

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
            "stop_helpers",
            "detach_nets",
            "destroy_managed",
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
            "completed",
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
            "running",
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
        "ensure_vms" => ensure_vms(environment, options.force, plan, socket_path),
        "attach_nets" => {
            ensure_attachments(environment, options.mode, socket_path)?;
            prune_networks(environment, options.mode, plan, socket_path)
        }
        "start_helpers" => start_helpers(environment, socket_path),
        "await_agents" => await_helpers(environment, socket_path),
        "apply_routes_policies" => apply_routes(environment, socket_path),
        "stop_helpers" => stop_helpers(environment, socket_path),
        "detach_nets" if options.purge => detach_networks(environment, socket_path),
        "destroy_managed" if options.purge => purge_managed(environment, socket_path),
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
            if existing["cidr"] != network.cidr || existing["mode"] != mode {
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
    for name in order {
        let vm = &environment.spec.vms[&name];
        let action = plan
            .actions
            .iter()
            .find(|action| action.kind == "vm" && action.name == name);
        if action.is_some_and(|action| action.action == "update" && action.breaking) {
            if !force {
                return Err(Failure::new(EXIT_INVALID, "VM recreate requires --force"));
            }
            stop_one(&name, socket_path)?;
            wait_stopped(&name, socket_path)?;
            remove_managed_vm(&name)?;
        }
        let bundle = crate::state_dir().join("vms").join(&name);
        if bundle.join("vm.json").is_file() {
            continue;
        }
        let image_name = &environment.spec.images[&vm.from].from;
        let size = data_disk_gib(&vm.data_disk)?;
        let mut owned = vec![
            "vm".to_string(),
            "create".to_string(),
            name.clone(),
            "--from".to_string(),
            image_name.clone(),
            "--data-disk".to_string(),
            size.to_string(),
        ];
        if let Some(network) = vm.networks.first() {
            owned.extend(["--network".to_string(), network.name.clone()]);
        }
        for role in &vm.roles {
            if role == "router" {
                owned.extend(["--role".to_string(), role.clone()]);
            }
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
        .flat_map(|(vm_id, vm)| {
            vm.networks
                .iter()
                .map(move |network| (vm_id.as_str(), network.name.as_str(), network.ip.as_str()))
        })
        .collect::<BTreeSet<_>>();
    if mode == Mode::Apply {
        for attachment in snapshot["attachments"].as_array().into_iter().flatten() {
            let current = (
                attachment["vm_id"].as_str().unwrap_or_default(),
                attachment["network"].as_str().unwrap_or_default(),
                attachment["ip"].as_str().unwrap_or_default(),
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
    for (vm_id, vm) in &environment.spec.vms {
        for attachment in &vm.networks {
            let exists = snapshot["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|item| {
                    item["vm_id"] == *vm_id
                        && item["network"] == attachment.name
                        && item["ip"] == attachment.ip
                });
            if !exists {
                rpc(
                    socket_path,
                    "net.attach",
                    json!({
                        "vm_id": vm_id,
                        "network": attachment.name,
                        "ip": attachment.ip,
                        "labels": {"managed-by": "vzctl"},
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
    for vm_id in dependency_order(&environment.spec.vms)? {
        let bundle = crate::state_dir().join("vms").join(&vm_id);
        rpc(
            socket_path,
            "vm.start",
            json!({"vm_id": vm_id, "bundle": bundle}),
        )?;
    }
    Ok(())
}

fn await_helpers(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let wanted = environment
        .spec
        .vms
        .keys()
        .cloned()
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
    for vm_id in order {
        stop_one(&vm_id, socket_path)?;
        wait_stopped(&vm_id, socket_path)?;
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
    for vm_id in environment.spec.vms.keys() {
        remove_managed_vm(vm_id)?;
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
        add("vm", name, json!(vm), "running")?;
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
}
