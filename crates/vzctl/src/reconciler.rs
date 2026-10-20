// Apply orchestration; domain operations live in the modules below.
mod adopt;
mod guest;
mod inputs;
mod io;
mod lease;
mod network;
mod plan;
mod services;
mod vm;

use adopt::*;
use guest::*;
use inputs::*;
use io::*;
use lease::*;
use network::*;
use plan::*;
use services::*;
use vm::*;

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
use std::sync::{mpsc, Arc, Mutex};
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

#[derive(Clone, Debug)]
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
    let resource_budget = crate::resource_budget::assess(&environment);
    if matches!(
        options.mode,
        Mode::Plan | Mode::Diff | Mode::Up | Mode::Apply
    ) {
        for warning in &resource_budget.warnings {
            eprintln!("WARN: {warning}");
        }
    }
    let stack_id = stack_id(&environment);

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

    let mut desired = desired_resources(&environment)?;
    append_input_resources(
        &environment,
        &options.config,
        &crate::state_dir(),
        &mut desired,
    )?;
    let desired_digest = desired_hash(&environment, &desired)?;

    let state = rpc(socket_path, "stack.inspect", json!({"stack_id": stack_id}))?;
    let actual = actual_resources(&state)?;
    let (effective_mode, effective_purge) = if options.resume {
        journal_context(&state)?
    } else {
        (options.mode, options.purge)
    };

    // Plan and apply use the same live ownership gate, before acquiring a lease
    // or creating any network. Force never authorizes touching a foreign owner.
    if effective_mode != Mode::Down && !environment.spec.networks.is_empty() {
        validate_network_ownership(&environment, &rpc(socket_path, "net.list", json!({}))?)?;
    }

    if matches!(options.mode, Mode::Plan | Mode::Diff) {
        let plan = build_plan(effective_mode, desired, actual);
        let mut output = plan_output(&stack_id, &plan);
        output["resource_budget"] = json!(resource_budget);
        return Ok(output);
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
            "desired_hash": desired_digest,
            "mode": mode,
            "purge": options.purge,
        }),
    )?;
    let journal_id = journal["id"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "invalid stack.begin response"))?
        .to_string();
    let heartbeat = LeaseHeartbeat::start(
        socket_path,
        &journal_id,
        &stack_id,
        &holder,
        Duration::from_secs(30),
    )?;
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
        heartbeat.check()?;
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
        heartbeat.check()?;
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
    // Files are read again by subprocesses during apply. Refuse to commit an old
    // snapshot if a user edited the config or a referenced input in the meantime.
    let current_environment = load_config(&options.config)?;
    let mut current_desired = desired_resources(&current_environment)?;
    append_input_resources(
        &current_environment,
        &options.config,
        &crate::state_dir(),
        &mut current_desired,
    )?;
    if desired_hash(&current_environment, &current_desired)? != desired_digest {
        return Err(Failure::new(EXIT_INCOMPLETE,
            "configuration or referenced inputs changed during apply; abort the journal and apply again"));
    }
    // Join before finish releases the lease, so a late heartbeat cannot race it.
    heartbeat.stop()?;
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
        "resource_budget": resource_budget,
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
        "ensure_ingress"
            if !environment.spec.service_claims.is_empty()
                || !environment.spec.kubernetes.clusters.is_empty() =>
        {
            Ok(())
        }
        "ensure_ingress" => ensure_ingress(environment, socket_path),
        "ensure_ca_rollout" => ensure_ca_rollout(environment, socket_path),
        "ensure_oidc_inject" => ensure_oidc_inject(environment, socket_path),
        "ensure_docker_context" => ensure_docker_context(environment),
        "ensure_containers" => ensure_containers(environment, &options.config, socket_path),
        "ensure_service_claims" => {
            ensure_service_claims(environment, &options.config, socket_path, progress)
        }
        "ensure_kubernetes" => crate::kubernetes::ensure(
            environment,
            &options.config,
            socket_path,
            &crate::state_dir(),
            progress,
        )
        .map_err(|error| Failure::new(EXIT_STEP, format!("Kubernetes reconcile: {error}"))),
        "ensure_ports" => ensure_ports(environment, socket_path),
        "apply_routes_policies" => apply_routes(environment, socket_path),
        "disable_service_controllers" => rpc(
            socket_path,
            "controller.unregister",
            json!({"project":environment.spec.project}),
        )
        .map(|_| ()),
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

fn holder() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
    format!("{host}:{}", std::process::id())
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
            "resource_budget": output["resource_budget"],
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
mod tests;

/// Reconcile a projected claim configuration through the existing host edge pipeline.
pub(crate) fn reconcile_claim_edge(
    environment: &Environment,
    config_file: &Path,
    socket: &Path,
    kind: crate::controllers::ServiceKind,
    guard: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    guard()?;
    ensure_ca(environment).map_err(|error| error.message)?;
    let guard = || guard().map_err(|error| Failure::new(EXIT_STEP, error));
    match kind {
        crate::controllers::ServiceKind::Oidc => {
            ensure_oidc_guarded(environment, config_file, socket, &guard)
        }
        crate::controllers::ServiceKind::Ingress => {
            ensure_ingress_guarded(environment, socket, &guard)
        }
        _ => return Err("claim kind does not use the host edge controller".into()),
    }
    .map_err(|error| error.message)
}
