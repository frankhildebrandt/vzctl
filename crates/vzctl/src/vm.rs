use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_SUPERVISOR: u8 = 10;
const EXIT_UNAVAILABLE: u8 = 12;
const EXIT_VM_DISK: u8 = 16;
const EXIT_NETWORK: u8 = 17;
const EXIT_GUEST: u8 = 18;
const EXIT_VM_OP: u8 = 24;
const TRANSFER_MAX_BYTES: usize = 256 * 1024;
const DEFAULT_LOG_TAIL: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Debug, Eq, PartialEq)]
enum Operation {
    List,
    HostPs,
    Start {
        id: String,
    },
    Stop {
        id: String,
        wait: bool,
    },
    Delete {
        id: String,
        force: bool,
    },
    Inspect {
        id: String,
    },
    Exec {
        id: String,
        cmd: Vec<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
        timeout_ms: u64,
        interactive: bool,
        tty: bool,
    },
    Transfer {
        id: String,
        src: TransferPath,
        dst: TransferPath,
    },
    Attach {
        id: String,
    },
    Services {
        id: String,
        action: ServicesAction,
    },
    GuestPs {
        id: String,
    },
    Logs {
        id: String,
        follow: bool,
        tail: usize,
    },
    Mount {
        id: String,
        source: PathBuf,
        target: String,
        name: Option<String>,
        read_only: bool,
    },
    Unmount {
        id: String,
        target: Option<String>,
        name: Option<String>,
    },
    Mounts {
        id: String,
    },
    Modify {
        id: String,
        cpus: Option<u32>,
        memory_mib: Option<u64>,
    },
    AgentUpgrade {
        all: bool,
        id: Option<String>,
    },
}

#[derive(Debug, Eq, PartialEq)]
enum TransferPath {
    Host(PathBuf),
    Guest(String),
}

#[derive(Debug, Eq, PartialEq)]
enum ServicesAction {
    List,
    Start(String),
    Stop(String),
    Restart(String),
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    operation: Operation,
    format: Format,
}

#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) fn command(args: impl Iterator<Item = String>, socket_path: &Path) -> ExitCode {
    run(args, socket_path, false)
}

pub(crate) fn ps_command(args: impl Iterator<Item = String>, socket_path: &Path) -> ExitCode {
    run(
        std::iter::once("ps".to_string()).chain(args),
        socket_path,
        true,
    )
}

fn run(args: impl Iterator<Item = String>, socket_path: &Path, ps_top_level: bool) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let options = match parse(args.into_iter(), ps_top_level) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(
                requested_format,
                if ps_top_level { "ps" } else { "vm" },
                &failure,
            );
            return ExitCode::from(failure.code);
        }
    };
    let command = options.command();
    let follow_streamed = matches!(options.operation, Operation::Logs { follow: true, .. });
    match execute(&options, socket_path) {
        Ok(envelope) => {
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human if follow_streamed => {}
                Format::Human => print_human(command, &envelope),
            }
            let exit = envelope["exit_code"].as_u64().unwrap_or(0) as u8;
            ExitCode::from(exit)
        }
        Err(failure) => {
            emit_failure(options.format, command, &failure);
            ExitCode::from(failure.code)
        }
    }
}

impl Options {
    fn command(&self) -> &'static str {
        match self.operation {
            Operation::List => "vm.list",
            Operation::HostPs => "ps",
            Operation::Start { .. } => "vm.start",
            Operation::Stop { .. } => "vm.stop",
            Operation::Delete { .. } => "vm.delete",
            Operation::Inspect { .. } => "vm.inspect",
            Operation::Exec {
                interactive: true,
                tty: true,
                ..
            } => "vm.exec",
            Operation::Exec { .. } => "vm.exec",
            Operation::Transfer { .. } => "vm.transfer",
            Operation::Attach { .. } => "vm.attach",
            Operation::Services { .. } => "vm.services",
            Operation::GuestPs { .. } => "vm.ps",
            Operation::Logs { .. } => "vm.logs",
            Operation::Mount { .. } => "vm.mount",
            Operation::Unmount { .. } => "vm.unmount",
            Operation::Mounts { .. } => "vm.mounts",
            Operation::Modify { .. } => "vm.modify",
            Operation::AgentUpgrade { .. } => "vm.agent.upgrade",
        }
    }
}

fn parse(mut args: impl Iterator<Item = String>, ps_top_level: bool) -> Result<Options, Failure> {
    let operation = args
        .next()
        .ok_or_else(|| {
            usage(if ps_top_level {
                "usage: vzctl ps [--format human|json]"
            } else {
                "usage: vzctl vm list|start|stop|delete|inspect|logs|exec|transfer|attach|services|ps|mount|unmount|mounts|modify|agent ..."
            })
        })?;
    let rest = args.collect::<Vec<_>>();
    match operation.as_str() {
        "list" if !ps_top_level => parse_list(rest, Operation::List),
        "ps" if ps_top_level => parse_list(rest, Operation::HostPs),
        "ps" if !ps_top_level => parse_guest_ps(rest),
        "start" if !ps_top_level => parse_start(rest),
        "stop" if !ps_top_level => parse_stop(rest),
        "delete" if !ps_top_level => parse_delete(rest),
        "inspect" if !ps_top_level => parse_inspect(rest),
        "logs" if !ps_top_level => parse_logs(rest),
        "exec" if !ps_top_level => parse_exec(rest),
        "transfer" if !ps_top_level => parse_transfer(rest),
        "attach" if !ps_top_level => parse_attach(rest),
        "services" if !ps_top_level => parse_services(rest),
        "mount" if !ps_top_level => parse_mount(rest),
        "unmount" if !ps_top_level => parse_unmount(rest),
        "mounts" if !ps_top_level => parse_mounts_list(rest),
        "modify" if !ps_top_level => parse_modify(rest),
        "agent" if !ps_top_level => parse_agent(rest),
        other => Err(usage(if ps_top_level {
            format!("unknown ps option: {other}")
        } else {
            format!("unknown vm command: {other}")
        })),
    }
}

fn parse_list(args: Vec<String>, operation: Operation) -> Result<Options, Failure> {
    let (format, flags) = parse_flags(&args, &[])?;
    if !flags.is_empty() {
        return Err(usage(format!(
            "{} accepts only --format human|json",
            match operation {
                Operation::HostPs => "ps",
                _ => "vm list",
            }
        )));
    }
    Ok(Options { operation, format })
}

fn parse_start(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm start requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm start accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Start { id },
        format,
    })
}

fn parse_stop(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm stop requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &["--wait"])?;
    let wait = match flags.get("--wait").map(String::as_str) {
        None => true,
        Some("true" | "1" | "yes") => true,
        Some("false" | "0" | "no") => false,
        Some(other) => return Err(usage(format!("--wait requires true|false (got {other})"))),
    };
    // bare --wait without value is treated as presence via dedicated parse below
    Ok(Options {
        operation: Operation::Stop { id, wait },
        format,
    })
}

fn parse_delete(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm delete requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut force = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--force" => {
                force = true;
                index += 1;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(usage(format!("unsupported vm format: {other}")));
                    }
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm delete option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm delete argument: {other}")));
            }
        }
    }
    Ok(Options {
        operation: Operation::Delete { id, force },
        format,
    })
}

fn parse_inspect(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm inspect requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm inspect accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Inspect { id },
        format,
    })
}

fn parse_modify(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm modify requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut cpus = None;
    let mut memory_mib = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--cpus" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--cpus requires a positive integer"))?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| Failure::new(EXIT_INVALID, format!("invalid --cpus: {value}")))?;
                if parsed == 0 {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        "--cpus must be greater than zero",
                    ));
                }
                cpus = Some(parsed);
                index += 2;
            }
            "--memory" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--memory requires a size"))?;
                memory_mib = Some(
                    crate::parse_memory_mib(value)
                        .map_err(|message| Failure::new(EXIT_INVALID, message))?,
                );
                index += 2;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(usage(format!("unsupported vm format: {other}")));
                    }
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm modify option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm modify argument: {other}")));
            }
        }
    }
    if cpus.is_none() && memory_mib.is_none() {
        return Err(usage(
            "vm modify requires at least one of --cpus or --memory",
        ));
    }
    Ok(Options {
        operation: Operation::Modify {
            id,
            cpus,
            memory_mib,
        },
        format,
    })
}

fn parse_agent(args: Vec<String>) -> Result<Options, Failure> {
    let sub = positional(&args, "vm agent requires a subcommand (upgrade)")?;
    if sub != "upgrade" {
        return Err(usage(format!("unknown vm agent subcommand: {sub}")));
    }
    let mut format = Format::Human;
    let mut all = false;
    let mut id = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                all = true;
                index += 1;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(usage(format!("unsupported vm format: {other}")));
                    }
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm agent option: {other}")));
            }
            other => {
                if id.is_some() {
                    return Err(usage(format!("unexpected vm agent argument: {other}")));
                }
                validate_vm_id(other)?;
                id = Some(other.to_string());
                index += 1;
            }
        }
    }
    if all && id.is_some() {
        return Err(usage("vm agent upgrade accepts either --all or a VM id"));
    }
    if !all && id.is_none() {
        return Err(usage("vm agent upgrade requires a VM id or --all"));
    }
    Ok(Options {
        operation: Operation::AgentUpgrade { all, id },
        format,
    })
}

fn parse_mount(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm mount requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut source = None;
    let mut target = None;
    let mut name = None;
    let mut read_only = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                source = Some(PathBuf::from(require_value(&args, &mut index, "--source")?));
            }
            "--target" => {
                target = Some(require_value(&args, &mut index, "--target")?);
            }
            "--tag" | "--name" => {
                name = Some(require_value(&args, &mut index, "--tag")?);
            }
            "--ro" | "--read-only" => {
                read_only = true;
                index += 1;
            }
            "--format" => {
                format = parse_format_value(&require_value(&args, &mut index, "--format")?)?;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm mount option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm mount argument: {other}")));
            }
        }
    }
    let source = source.ok_or_else(|| usage("vm mount requires --source PATH"))?;
    let target = target.ok_or_else(|| usage("vm mount requires --target PATH"))?;
    Ok(Options {
        operation: Operation::Mount {
            id,
            source,
            target,
            name,
            read_only,
        },
        format,
    })
}

fn parse_unmount(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm unmount requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut target = None;
    let mut name = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                target = Some(require_value(&args, &mut index, "--target")?);
            }
            "--tag" | "--name" => {
                name = Some(require_value(&args, &mut index, "--tag")?);
            }
            "--format" => {
                format = parse_format_value(&require_value(&args, &mut index, "--format")?)?;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm unmount option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm unmount argument: {other}")));
            }
        }
    }
    if target.is_none() && name.is_none() {
        return Err(usage("vm unmount requires --target or --tag"));
    }
    Ok(Options {
        operation: Operation::Unmount { id, target, name },
        format,
    })
}

fn parse_mounts_list(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm mounts requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm mounts accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Mounts { id },
        format,
    })
}

fn require_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, Failure> {
    if *index + 1 >= args.len() {
        return Err(usage(format!("{flag} requires a value")));
    }
    let value = args[*index + 1].clone();
    *index += 2;
    Ok(value)
}

fn parse_format_value(value: &str) -> Result<Format, Failure> {
    match value {
        "human" => Ok(Format::Human),
        "json" => Ok(Format::Json),
        other => Err(usage(format!("unsupported format: {other}"))),
    }
}

fn parse_logs(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm logs requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut follow = false;
    let mut tail = DEFAULT_LOG_TAIL;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "-f" | "--follow" => {
                follow = true;
                index += 1;
            }
            "--tail" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--tail requires a line count"))?;
                tail = value
                    .parse::<usize>()
                    .map_err(|_| usage(format!("invalid --tail value: {value}")))?;
                index += 2;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            other => {
                return Err(usage(format!("unknown vm logs option: {other}")));
            }
        }
    }
    if follow && format == Format::Json {
        return Err(Failure::new(
            EXIT_INVALID,
            "vm logs --follow supports only --format human",
        ));
    }
    Ok(Options {
        operation: Operation::Logs { id, follow, tail },
        format,
    })
}

fn parse_attach(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm attach requires a VM id")?;
    validate_vm_id(&id)?;
    if args.len() > 1 {
        return Err(usage("vm attach accepts only a VM id"));
    }
    Ok(Options {
        operation: Operation::Attach { id },
        format: Format::Human,
    })
}

fn parse_guest_ps(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm ps requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm ps accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::GuestPs { id },
        format,
    })
}

fn parse_services(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm services requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut action = ServicesAction::List;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            "start" | "stop" | "restart" => {
                let verb = args[index].clone();
                let unit = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| usage(format!("vm services {verb} requires a unit")))?
                    .clone();
                action = match verb.as_str() {
                    "start" => ServicesAction::Start(unit),
                    "stop" => ServicesAction::Stop(unit),
                    _ => ServicesAction::Restart(unit),
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm services option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm services argument: {other}")));
            }
        }
    }
    Ok(Options {
        operation: Operation::Services { id, action },
        format,
    })
}

fn parse_exec(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm exec requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut cwd = None;
    let mut env = BTreeMap::new();
    let mut timeout_ms = 30_000_u64;
    let mut interactive = false;
    let mut tty = false;
    let mut index = 1;
    let mut cmd = Vec::new();
    while index < args.len() {
        match args[index].as_str() {
            "--" => {
                cmd.extend(args[index + 1..].iter().cloned());
                break;
            }
            "-i" | "--interactive" => {
                interactive = true;
                index += 1;
            }
            "-t" | "--tty" => {
                tty = true;
                index += 1;
            }
            "-it" | "-ti" => {
                interactive = true;
                tty = true;
                index += 1;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            "--cwd" => {
                cwd = Some(
                    args.get(index + 1)
                        .ok_or_else(|| usage("--cwd requires a path"))?
                        .clone(),
                );
                index += 2;
            }
            "--env" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--env requires KEY=VALUE"))?;
                let (key, value) = value
                    .split_once('=')
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or_else(|| usage(format!("--env must be KEY=VALUE: {value}")))?;
                env.insert(key.to_string(), value.to_string());
                index += 2;
            }
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--timeout-ms requires a value"))?;
                timeout_ms = value.parse::<u64>().map_err(|_| {
                    Failure::new(EXIT_INVALID, format!("invalid --timeout-ms: {value}"))
                })?;
                if timeout_ms == 0 {
                    return Err(Failure::new(EXIT_INVALID, "--timeout-ms must be > 0"));
                }
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm exec option: {other}")));
            }
            _ => {
                cmd.extend(args[index..].iter().cloned());
                break;
            }
        }
    }
    if cmd.is_empty() {
        return Err(usage("vm exec requires a command"));
    }
    if interactive != tty {
        return Err(Failure::new(
            EXIT_INVALID,
            "vm exec interactive tty requires both -i/--interactive and -t/--tty (use -it)",
        ));
    }
    Ok(Options {
        operation: Operation::Exec {
            id,
            cmd,
            cwd,
            env,
            timeout_ms,
            interactive,
            tty,
        },
        format,
    })
}

fn parse_transfer(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm transfer requires a VM id")?;
    validate_vm_id(&id)?;
    let src_raw = args
        .get(1)
        .ok_or_else(|| usage("vm transfer requires <src> <dst>"))?
        .clone();
    let dst_raw = args
        .get(2)
        .ok_or_else(|| usage("vm transfer requires <src> <dst>"))?
        .clone();
    let (format, flags) = parse_flags(&args[3..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm transfer accepts only --format human|json"));
    }
    let src = parse_transfer_path(&id, &src_raw)?;
    let dst = parse_transfer_path(&id, &dst_raw)?;
    match (&src, &dst) {
        (TransferPath::Host(_), TransferPath::Guest(_))
        | (TransferPath::Guest(_), TransferPath::Host(_)) => {}
        _ => {
            return Err(Failure::new(
                EXIT_INVALID,
                "vm transfer requires one host path and one <id>:<guest-path>",
            ));
        }
    }
    Ok(Options {
        operation: Operation::Transfer { id, src, dst },
        format,
    })
}

fn parse_transfer_path(vm_id: &str, value: &str) -> Result<TransferPath, Failure> {
    if let Some((prefix, path)) = value.split_once(':') {
        if prefix == vm_id {
            if path.is_empty() {
                return Err(Failure::new(EXIT_INVALID, "guest path must not be empty"));
            }
            return Ok(TransferPath::Guest(path.to_string()));
        }
    }
    Ok(TransferPath::Host(PathBuf::from(value)))
}

fn parse_flags(
    args: &[String],
    valued: &[&str],
) -> Result<(Format, BTreeMap<String, String>), Failure> {
    let mut format = Format::Human;
    let mut values = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        if flag == "--wait" && valued.contains(&"--wait") {
            // Optional value: bare --wait means true.
            let next = args.get(index + 1).map(String::as_str);
            match next {
                Some(value) if !value.starts_with('-') => {
                    values.insert("--wait".to_string(), value.to_string());
                    index += 2;
                }
                _ => {
                    values.insert("--wait".to_string(), "true".to_string());
                    index += 1;
                }
            }
            continue;
        }
        if flag != "--format" && !valued.contains(&flag.as_str()) {
            return Err(usage(format!("unknown vm option: {flag}")));
        }
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with('-'))
            .ok_or_else(|| usage(format!("{flag} requires a value")))?
            .clone();
        if flag == "--format" {
            format = match value.as_str() {
                "human" => Format::Human,
                "json" => Format::Json,
                other => return Err(usage(format!("unsupported vm format: {other}"))),
            };
        } else if values.insert(flag.clone(), value).is_some() {
            return Err(usage(format!("duplicate option: {flag}")));
        }
        index += 2;
    }
    Ok((format, values))
}

fn positional(args: &[String], message: &str) -> Result<String, Failure> {
    match args.first() {
        Some(value) if !value.starts_with('-') && !value.is_empty() => Ok(value.clone()),
        _ => Err(usage(message)),
    }
}

fn validate_vm_id(id: &str) -> Result<(), Failure> {
    if crate::valid_vm_id(id) {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_INVALID,
            format!("invalid VM id: {id} (flat label or project/vm, alphanumeric/._-)"),
        ))
    }
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format")
        .and_then(|pair| (pair[1] == "json").then_some(Format::Json))
        .unwrap_or(Format::Human)
}

fn execute(options: &Options, socket_path: &Path) -> Result<Value, Failure> {
    match &options.operation {
        Operation::List | Operation::HostPs => list_vms(options.command(), socket_path),
        Operation::Start { id } => start_vm(id, socket_path),
        Operation::Stop { id, wait } => stop_vm(id, *wait, socket_path),
        Operation::Delete { id, force } => delete_vm(id, *force, socket_path),
        Operation::Inspect { id } => inspect_vm(id, socket_path),
        Operation::Exec {
            id,
            cmd,
            cwd,
            env,
            timeout_ms: _,
            interactive: true,
            tty: true,
        } => exec_tty_vm(id, cmd, cwd.as_deref(), env, socket_path),
        Operation::Exec {
            id,
            cmd,
            cwd,
            env,
            timeout_ms,
            interactive: false,
            tty: false,
        } => exec_vm(id, cmd, cwd.as_deref(), env, *timeout_ms, socket_path),
        Operation::Exec { .. } => Err(Failure::new(
            EXIT_INVALID,
            "vm exec interactive tty requires both -i/--interactive and -t/--tty (use -it)",
        )),
        Operation::Transfer { id, src, dst } => transfer_vm(id, src, dst, socket_path),
        Operation::Attach { id } => attach_vm(id),
        Operation::Services { id, action } => services_vm(id, action, socket_path),
        Operation::GuestPs { id } => guest_ps_vm(id, socket_path),
        Operation::Logs { id, follow, tail } => logs_vm(id, *follow, *tail),
        Operation::Mount {
            id,
            source,
            target,
            name,
            read_only,
        } => mount_vm(id, source, target, name.as_deref(), *read_only, socket_path),
        Operation::Unmount { id, target, name } => {
            unmount_vm(id, target.as_deref(), name.as_deref(), socket_path)
        }
        Operation::Mounts { id } => list_vm_mounts(id, socket_path),
        Operation::Modify {
            id,
            cpus,
            memory_mib,
        } => modify_vm(id, *cpus, *memory_mib, socket_path),
        Operation::AgentUpgrade { all, id } => agent_upgrade_vm(*all, id.as_deref(), socket_path),
    }
}

fn list_vms(command: &str, socket_path: &Path) -> Result<Value, Failure> {
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

fn fallback_networks_from_entry(entry: &Value) -> Vec<Value> {
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

fn start_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
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

fn stop_vm(id: &str, wait: bool, socket_path: &Path) -> Result<Value, Failure> {
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

fn delete_vm(id: &str, force: bool, socket_path: &Path) -> Result<Value, Failure> {
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
                            let network = attachment["network"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            match rpc(
                                socket_path,
                                "net.detach",
                                json!({ "vm_id": id, "network": network }),
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

fn map_network_failure(failure: Failure) -> Failure {
    if failure.code == EXIT_VM_OP {
        Failure::new(EXIT_NETWORK, failure.message)
    } else {
        failure
    }
}

fn inspect_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
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

fn fallback_networks_from_manifest(manifest: &Value) -> Vec<Value> {
    fallback_networks_from_entry(&json!({ "identity": manifest.get("identity") }))
}

fn exec_vm(
    id: &str,
    cmd: &[String],
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
    timeout_ms: u64,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let mut params = json!({
        "vm_id": id,
        "cmd": cmd,
        "timeout_ms": timeout_ms,
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    if !env.is_empty() {
        params["env"] = json!(env);
    }
    let result = rpc(socket_path, "vm.exec", params)?;
    let exit = result["exit"].as_u64().unwrap_or(1) as u8;
    let truncated = result["truncated"].as_bool().unwrap_or(false);
    let stdout = result["stdout"].as_str().unwrap_or("").to_string();
    let stderr = result["stderr"].as_str().unwrap_or("").to_string();
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.exec",
        "status": if exit == 0 { "ok" } else { "fail" },
        "exit_code": exit,
        "summary": {
            "message": format!("exit {exit}"),
            "vm_id": id,
            "exit": exit,
            "truncated": truncated,
        },
        "exec": {
            "cmd": cmd,
            "exit": exit,
            "stdout": stdout,
            "stderr": stderr,
            "truncated": truncated,
        },
    }))
}

fn exec_tty_vm(
    id: &str,
    cmd: &[String],
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let (cols, rows) = terminal_winsize();
    let mut params = json!({
        "vm_id": id,
        "cmd": cmd,
        "cols": cols,
        "rows": rows,
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    if !env.is_empty() {
        params["env"] = json!(env);
    }
    let result = rpc(socket_path, "vm.exec_tty", params)?;
    let path = result["socket"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "vm.exec_tty missing socket path"))?;
    let mut stream = UnixStream::connect(path).map_err(|error| {
        Failure::new(EXIT_SUPERVISOR, format!("exec tty socket {path}: {error}"))
    })?;
    eprintln!("attached to {id} exec tty (Ctrl-P Ctrl-Q to detach)");
    let mut resize = [0_u8; 4];
    resize[0..2].copy_from_slice(&cols.to_le_bytes());
    resize[2..4].copy_from_slice(&rows.to_le_bytes());
    write_mux_frame(&mut stream, MUX_RESIZE, &resize)?;
    let exit = mux_tty_session(&mut stream)?;
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.exec",
        "status": if exit == 0 { "ok" } else { "fail" },
        "exit_code": exit,
        "summary": {
            "message": format!("exit {exit}"),
            "vm_id": id,
            "exit": exit,
            "tty": true,
        },
        "exec": {
            "cmd": cmd,
            "exit": exit,
            "tty": true,
        },
    }))
}

fn terminal_winsize() -> (u16, u16) {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    let cols = if ok && size.ws_col > 0 {
        size.ws_col
    } else {
        80
    };
    let rows = if ok && size.ws_row > 0 {
        size.ws_row
    } else {
        24
    };
    (cols, rows)
}

const MUX_STDIN: u8 = 0x01;
const MUX_STDOUT: u8 = 0x02;
const MUX_RESIZE: u8 = 0x04;
const MUX_EXIT: u8 = 0x05;
const MUX_STDIN_EOF: u8 = 0x06;

fn write_mux_frame(stream: &mut UnixStream, frame_type: u8, payload: &[u8]) -> Result<(), Failure> {
    use std::io::Write as IoWrite;
    if payload.len() > 1_048_576 {
        return Err(Failure::new(EXIT_VM_OP, "mux frame exceeds 1 MiB"));
    }
    let mut header = [0_u8; 5];
    header[0] = frame_type;
    header[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    stream
        .write_all(&header)
        .and_then(|_| {
            if payload.is_empty() {
                Ok(())
            } else {
                stream.write_all(payload)
            }
        })
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("mux write: {error}")))
}

fn mux_tty_session(stream: &mut UnixStream) -> Result<u8, Failure> {
    use std::io::{Read, Write as IoWrite};
    use std::os::fd::AsRawFd;

    let stdin_fd = libc::STDIN_FILENO;
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    let had_tty = unsafe { libc::tcgetattr(stdin_fd, original.as_mut_ptr()) } == 0;
    let original = if had_tty {
        Some(unsafe { original.assume_init() })
    } else {
        None
    };
    if let Some(mut raw) = original {
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
            return Err(Failure::new(EXIT_VM_OP, "cannot set raw terminal mode"));
        }
    }
    let _restore = TerminalRestore(original);

    stream
        .set_nonblocking(true)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut detach = ConsoleDetachState::default();
    let mut exit_code = 0_u8;
    loop {
        match read_mux_frame_nonblocking(stream) {
            Ok(None) => {}
            Ok(Some((MUX_STDOUT, payload))) => {
                stdout
                    .write_all(&payload)
                    .and_then(|_| stdout.flush())
                    .map_err(|error| Failure::new(EXIT_VM_OP, error.to_string()))?;
            }
            Ok(Some((MUX_EXIT, payload))) => {
                if payload.len() == 4 {
                    let status = i32::from_le_bytes(payload[0..4].try_into().unwrap());
                    exit_code = status.clamp(0, 255) as u8;
                }
                break;
            }
            Ok(Some(_)) => {
                return Err(Failure::new(EXIT_SUPERVISOR, "unexpected mux frame"));
            }
            Err(error) if error.message.contains("WouldBlock") => {}
            Err(error) => return Err(error),
        }

        let mut pollfd = libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if ready > 0 && pollfd.revents & libc::POLLIN != 0 {
            let mut input = [0_u8; 1024];
            match stdin.read(&mut input) {
                Ok(0) => {
                    write_mux_frame(stream, MUX_STDIN_EOF, &[])?;
                }
                Ok(count) => {
                    let mut forward = Vec::new();
                    if consume_console_stdin(&mut detach, &input[..count], &mut forward) {
                        break;
                    }
                    if !forward.is_empty() {
                        write_mux_frame(stream, MUX_STDIN, &forward)?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(Failure::new(EXIT_VM_OP, format!("stdin read: {error}")));
                }
            }
        }

        let mut sock_poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let _ = unsafe { libc::poll(&mut sock_poll, 1, 0) };
        if sock_poll.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
    }
    Ok(exit_code)
}

fn read_mux_frame_nonblocking(stream: &mut UnixStream) -> Result<Option<(u8, Vec<u8>)>, Failure> {
    use std::io::Read;
    let mut header = [0_u8; 5];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(Failure::new(EXIT_SUPERVISOR, "mux connection closed"));
        }
        Err(error) => {
            return Err(Failure::new(EXIT_SUPERVISOR, format!("mux read: {error}")));
        }
    }
    let length = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
    if length > 1_048_576 {
        return Err(Failure::new(EXIT_SUPERVISOR, "mux frame exceeds 1 MiB"));
    }
    let mut payload = vec![0_u8; length];
    if length > 0 {
        // After header, block briefly for payload (socket may be nonblocking).
        stream
            .set_nonblocking(false)
            .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
        let result = stream.read_exact(&mut payload);
        stream
            .set_nonblocking(true)
            .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
        result.map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("mux read: {error}")))?;
    }
    Ok(Some((header[0], payload)))
}

fn transfer_vm(
    id: &str,
    src: &TransferPath,
    dst: &TransferPath,
    socket_path: &Path,
) -> Result<Value, Failure> {
    match (src, dst) {
        (TransferPath::Host(host), TransferPath::Guest(guest)) => {
            let bytes = fs::read(host).map_err(|error| {
                Failure::new(EXIT_INVALID, format!("read {}: {error}", host.display()))
            })?;
            if bytes.len() > TRANSFER_MAX_BYTES {
                return Err(Failure::new(
                    EXIT_UNAVAILABLE,
                    format!(
                        "transfer exceeds {} KiB agent limit; use virtiofs for larger files",
                        TRANSFER_MAX_BYTES / 1024
                    ),
                ));
            }
            let encoded =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            let result = rpc(
                socket_path,
                "vm.exec",
                json!({
                    "vm_id": id,
                    "cmd": ["tee", guest],
                    "timeout_ms": 30_000,
                    "stdin_b64": encoded,
                }),
            )?;
            if result["exit"].as_u64().unwrap_or(1) != 0
                || result["truncated"].as_bool().unwrap_or(false)
            {
                return Err(Failure::new(
                    EXIT_GUEST,
                    format!(
                        "push failed: {}",
                        result["stderr"].as_str().unwrap_or("guest exec error")
                    ),
                ));
            }
            Ok(transfer_envelope(
                id,
                "push",
                &host.display().to_string(),
                guest,
                bytes.len(),
            ))
        }
        (TransferPath::Guest(guest), TransferPath::Host(host)) => {
            let result = rpc(
                socket_path,
                "vm.exec",
                json!({
                    "vm_id": id,
                    "cmd": ["base64", "-w0", guest],
                    "timeout_ms": 30_000,
                }),
            )?;
            if result["exit"].as_u64().unwrap_or(1) != 0 {
                return Err(Failure::new(
                    EXIT_GUEST,
                    format!(
                        "pull failed: {}",
                        result["stderr"].as_str().unwrap_or("guest exec error")
                    ),
                ));
            }
            if result["truncated"].as_bool().unwrap_or(false) {
                return Err(Failure::new(
                    EXIT_UNAVAILABLE,
                    "pull truncated by agent limits; use virtiofs for larger files",
                ));
            }
            let encoded = result["stdout"].as_str().unwrap_or("").trim();
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                .map_err(|error| {
                    Failure::new(EXIT_GUEST, format!("invalid guest base64: {error}"))
                })?;
            if bytes.len() > TRANSFER_MAX_BYTES {
                return Err(Failure::new(
                    EXIT_UNAVAILABLE,
                    format!(
                        "transfer exceeds {} KiB agent limit; use virtiofs for larger files",
                        TRANSFER_MAX_BYTES / 1024
                    ),
                ));
            }
            if let Some(parent) = host.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).map_err(|error| {
                        Failure::new(
                            EXIT_INVALID,
                            format!("create {}: {error}", parent.display()),
                        )
                    })?;
                }
            }
            fs::write(host, &bytes).map_err(|error| {
                Failure::new(EXIT_INVALID, format!("write {}: {error}", host.display()))
            })?;
            Ok(transfer_envelope(
                id,
                "pull",
                guest,
                &host.display().to_string(),
                bytes.len(),
            ))
        }
        _ => Err(Failure::new(
            EXIT_INVALID,
            "vm transfer requires one host path and one <id>:<guest-path>",
        )),
    }
}

fn transfer_envelope(id: &str, direction: &str, src: &str, dst: &str, bytes: usize) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "command": "vm.transfer",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("copied {bytes} bytes ({direction})"),
            "vm_id": id,
            "change": "copied",
            "bytes": bytes,
            "direction": direction,
        },
        "transfer": {
            "src": src,
            "dst": dst,
            "bytes": bytes,
            "direction": direction,
            "truncated": false,
        },
    })
}

fn services_vm(id: &str, action: &ServicesAction, socket_path: &Path) -> Result<Value, Failure> {
    let cmd = match action {
        ServicesAction::List => vec![
            "systemctl".to_string(),
            "list-units".to_string(),
            "--type=service".to_string(),
            "--no-pager".to_string(),
            "--plain".to_string(),
        ],
        ServicesAction::Start(unit) => {
            vec!["systemctl".to_string(), "start".to_string(), unit.clone()]
        }
        ServicesAction::Stop(unit) => {
            vec!["systemctl".to_string(), "stop".to_string(), unit.clone()]
        }
        ServicesAction::Restart(unit) => {
            vec!["systemctl".to_string(), "restart".to_string(), unit.clone()]
        }
    };
    let envelope = exec_vm(id, &cmd, None, &BTreeMap::new(), 30_000, socket_path)?;
    let mut out = envelope;
    out["command"] = json!("vm.services");
    out["services"] = out["exec"].clone();
    Ok(out)
}

fn guest_ps_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    // `|| true` keeps agent one-shot exec from mapping non-zero `ps` to exec_failed.
    let cmd = [
        "sh",
        "-c",
        "ps -A -o pid= -o user= -o pcpu= -o pmem= -o args= 2>/dev/null || true",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let envelope = exec_vm(id, &cmd, None, &BTreeMap::new(), 30_000, socket_path)?;
    let stdout = envelope["exec"]["stdout"].as_str().unwrap_or("");
    let processes = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() >= 5 {
                json!({
                    "pid": parts[0].parse::<u64>().unwrap_or(0),
                    "user": parts[1],
                    "pcpu": parts[2],
                    "pmem": parts[3],
                    "args": parts[4..].join(" "),
                })
            } else {
                json!({ "raw": line })
            }
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.ps",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("{} process(es)", processes.len()),
            "vm_id": id,
            "processes": processes.len(),
        },
        "processes": processes,
        "exec": envelope["exec"].clone(),
    }))
}

fn attach_vm(id: &str) -> Result<Value, Failure> {
    validate_vm_id(id)?;
    let path = console_socket_path(id);
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!(
                "console socket {}: {error} (is the VM running?)",
                path.display()
            ),
        )
    })?;
    eprintln!("attached to {id} serial console (Ctrl-P Ctrl-Q to detach)");
    raw_console_session(&mut stream)?;
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.attach",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("detached from {id}"),
            "vm_id": id,
        },
    }))
}

fn logs_vm(id: &str, follow: bool, tail: usize) -> Result<Value, Failure> {
    validate_vm_id(id)?;
    let bundle = bundle_path(id);
    if !bundle.join("vm.json").is_file() {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("VM bundle not found: {}", bundle.display()),
        ));
    }
    let path = serial_log_path(id);
    if !path.is_file() {
        return Err(Failure::new(
            EXIT_SUPERVISOR,
            format!(
                "serial log missing: {} (is the VM started?)",
                path.display()
            ),
        ));
    }

    let (lines, redacted) = read_serial_tail(&path, tail)?;
    if follow {
        for line in &lines {
            println!("{line}");
        }
        follow_serial_log(&path)?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.logs",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": format!("followed serial log for {id}"),
                "vm_id": id,
                "source": "serial",
                "lines": lines.len(),
                "redacted": redacted,
            },
            "log": {
                "path": path.display().to_string(),
                "source": "serial",
            },
        }));
    }

    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.logs",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("serial log for {id}"),
            "vm_id": id,
            "source": "serial",
            "lines": lines.len(),
            "redacted": redacted,
        },
        "log": {
            "path": path.display().to_string(),
            "source": "serial",
        },
        "lines": lines,
    }))
}

fn read_serial_tail(path: &Path, tail: usize) -> Result<(Vec<String>, usize), Failure> {
    let content = fs::read_to_string(path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot read serial log {}: {error}", path.display()),
        )
    })?;
    let all: Vec<(String, bool)> = content.lines().map(|line| redact_log_line(line)).collect();
    let start = all.len().saturating_sub(tail);
    let mut redacted = 0usize;
    let lines = all[start..]
        .iter()
        .map(|(value, hit)| {
            if *hit {
                redacted += 1;
            }
            value.clone()
        })
        .collect();
    Ok((lines, redacted))
}

fn follow_serial_log(path: &Path) -> Result<(), Failure> {
    let mut file = File::open(path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot open serial log {}: {error}", path.display()),
        )
    })?;
    file.seek(SeekFrom::End(0)).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot seek serial log {}: {error}", path.display()),
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = String::new();
    loop {
        buffer.clear();
        match reader.read_line(&mut buffer) {
            Ok(0) => {
                thread::sleep(Duration::from_millis(200));
                // Re-open if truncated (VM restart).
                let metadata = fs::metadata(path).ok();
                let pos = reader.stream_position().unwrap_or(0);
                if let Some(meta) = metadata {
                    if pos > meta.len() {
                        let mut file = File::open(path).map_err(|error| {
                            Failure::new(
                                EXIT_SUPERVISOR,
                                format!("cannot reopen serial log {}: {error}", path.display()),
                            )
                        })?;
                        file.seek(SeekFrom::Start(0)).ok();
                        reader = BufReader::new(file);
                    }
                }
            }
            Ok(_) => {
                let line = buffer.trim_end_matches(['\r', '\n']);
                let (value, _) = redact_log_line(line);
                println!("{value}");
            }
            Err(error) => {
                return Err(Failure::new(
                    EXIT_SUPERVISOR,
                    format!("serial log follow read: {error}"),
                ));
            }
        }
    }
}

fn redact_log_line(line: &str) -> (String, bool) {
    let lower = line.to_ascii_lowercase();
    if lower.contains("password") || lower.contains("chpasswd") || lower.contains("root_password") {
        ("[redacted]".to_string(), true)
    } else {
        (line.to_string(), false)
    }
}

fn serial_log_path(id: &str) -> PathBuf {
    logs_dir().join(format!("{}.serial.log", state_file_component(id)))
}

fn logs_dir() -> PathBuf {
    if let Some(directory) = std::env::var_os("VZCTL_LOGS_DIR") {
        return PathBuf::from(directory);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Logs/vzctl")
}

fn console_socket_path(id: &str) -> PathBuf {
    crate::state_dir()
        .join("helpers")
        .join(format!("{}.console.sock", state_file_component(id)))
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

fn raw_console_session(stream: &mut UnixStream) -> Result<(), Failure> {
    use std::io::{Read, Write as IoWrite};
    use std::os::fd::AsRawFd;

    let stdin_fd = libc::STDIN_FILENO;
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    let had_tty = unsafe { libc::tcgetattr(stdin_fd, original.as_mut_ptr()) } == 0;
    let original = if had_tty {
        Some(unsafe { original.assume_init() })
    } else {
        None
    };
    if let Some(mut raw) = original {
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
            return Err(Failure::new(EXIT_VM_OP, "cannot set raw terminal mode"));
        }
    }
    let _restore = TerminalRestore(original);

    stream
        .set_nonblocking(true)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut buffer = [0_u8; 4096];
    let mut detach = ConsoleDetachState::default();
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                stdout
                    .write_all(&buffer[..count])
                    .and_then(|_| stdout.flush())
                    .map_err(|error| Failure::new(EXIT_VM_OP, error.to_string()))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                return Err(Failure::new(
                    EXIT_SUPERVISOR,
                    format!("console read: {error}"),
                ));
            }
        }

        let mut pollfd = libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if ready > 0 && pollfd.revents & libc::POLLIN != 0 {
            let mut input = [0_u8; 1024];
            match stdin.read(&mut input) {
                Ok(0) => break,
                Ok(count) => {
                    let mut forward = Vec::new();
                    if consume_console_stdin(&mut detach, &input[..count], &mut forward) {
                        if !forward.is_empty() {
                            stream.write_all(&forward).map_err(|error| {
                                Failure::new(EXIT_SUPERVISOR, format!("console write: {error}"))
                            })?;
                        }
                        break;
                    }
                    if !forward.is_empty() {
                        stream.write_all(&forward).map_err(|error| {
                            Failure::new(EXIT_SUPERVISOR, format!("console write: {error}"))
                        })?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(Failure::new(EXIT_VM_OP, format!("stdin read: {error}")));
                }
            }
        }

        let mut sock_poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let _ = unsafe { libc::poll(&mut sock_poll, 1, 0) };
        if sock_poll.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
    }
    Ok(())
}

/// Serial attach runs the host TTY in raw mode (`cfmakeraw` clears `ISIG`), so
/// Ctrl-C is a guest keystroke — not host SIGINT. Detach like Docker: Ctrl-P
/// then Ctrl-Q. Send a literal Ctrl-P to the guest with Ctrl-P Ctrl-P.
const CONSOLE_CTRL_P: u8 = 0x10;
const CONSOLE_CTRL_Q: u8 = 0x11;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ConsoleDetachState {
    saw_ctrl_p: bool,
}

/// Returns `true` when the Docker-style detach sequence was completed.
fn consume_console_stdin(
    state: &mut ConsoleDetachState,
    input: &[u8],
    forward: &mut Vec<u8>,
) -> bool {
    for &byte in input {
        if state.saw_ctrl_p {
            state.saw_ctrl_p = false;
            if byte == CONSOLE_CTRL_Q {
                return true;
            }
            forward.push(CONSOLE_CTRL_P);
            if byte != CONSOLE_CTRL_P {
                forward.push(byte);
            }
            continue;
        }
        if byte == CONSOLE_CTRL_P {
            state.saw_ctrl_p = true;
            continue;
        }
        forward.push(byte);
    }
    false
}

struct TerminalRestore(Option<libc::termios>);

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        if let Some(termios) = self.0 {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios);
            }
        }
    }
}

fn scan_bundles() -> Result<BTreeMap<String, Value>, Failure> {
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

fn insert_bundle_entry(
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

fn bundle_path(id: &str) -> PathBuf {
    crate::state_dir().join("vms").join(id)
}

fn helper_is_running(vm_id: &str, socket_path: &Path) -> Result<bool, Failure> {
    match rpc(socket_path, "vm.list", json!({})) {
        Ok(records) => Ok(records.as_array().into_iter().flatten().any(|record| {
            // Live mount/exec need helper.state == running — "starting" is too early.
            record["vm_id"] == vm_id && record["state"].as_str() == Some("running")
        })),
        Err(failure) if failure.code == EXIT_SUPERVISOR => Ok(false),
        Err(failure) => Err(failure),
    }
}

fn guest_mount_unsupported(failure: &Failure) -> bool {
    let message = failure.message.to_ascii_lowercase();
    message.contains("unsupported") && message.contains("method")
}

/// Bind a virtiofs share into PID 1's mount namespace (visible to Docker).
///
/// After a live share swap, previous guest mounts may show `//deleted` in
/// mountinfo until cleared. Always clear + remount in the init mount ns.
fn guest_virtiofs_bind_mount(
    id: &str,
    mount: &crate::mounts::ResolvedMount,
    socket_path: &Path,
) -> Result<(), Failure> {
    // Deploy the current helper (PrivateTmp hides /tmp from PID 1) then bind.
    let script = crate::guest_utils::VIRTIOFS_BIND_SCRIPT;
    let deploy_and_bind = format!(
        r#"set -eu
helper=/usr/local/lib/vzctl/virtiofs-bind
cat >"$helper.new" <<'VZCTL_VIRTIOFS_BIND_EOF'
{script}
VZCTL_VIRTIOFS_BIND_EOF
chmod 0755 "$helper.new"
mv -f "$helper.new" "$helper"
nsenter -t 1 -m -- "$helper" mount {name} {target}{mode_arg}
"#,
        script = script,
        name = sh_escape(&mount.name),
        target = sh_escape(&mount.target),
        mode_arg = if mount.read_only { " ro" } else { "" },
    );
    let args = vec![
        "sudo".into(),
        "-n".into(),
        "sh".into(),
        "-c".into(),
        deploy_and_bind,
    ];
    let result = exec_vm(id, &args, None, &BTreeMap::new(), 60_000, socket_path)?;
    let exit = result["exit_code"].as_u64().unwrap_or(1);
    if exit != 0 {
        let stderr = result
            .pointer("/exec/stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let stdout = result
            .pointer("/exec/stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "virtiofs-bind failed"
        };
        return Err(Failure::new(
            EXIT_GUEST,
            format!("guest virtiofs bind {}: {detail}", mount.target),
        ));
    }
    Ok(())
}

fn sh_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn mount_vm(
    id: &str,
    source: &Path,
    target: &str,
    name: Option<&str>,
    read_only: bool,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let _ = read_manifest(&bundle)?;
    let mut flag = format!("source={},target={}", source.display(), target);
    if let Some(name) = name {
        flag = format!("tag={name},{flag}");
    }
    if read_only {
        flag.push_str(",ro");
    }
    let mount = crate::mounts::parse_mount_flag(&flag)
        .map_err(|message| Failure::new(EXIT_INVALID, message))?;
    let mut mounts = crate::mounts::read_manifest_mounts(&bundle)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;
    if mounts
        .iter()
        .any(|existing| existing.name == mount.name && existing.target != mount.target)
    {
        return Err(Failure::new(
            EXIT_INVALID,
            format!(
                "mount name {} already maps to {}",
                mount.name,
                mounts
                    .iter()
                    .find(|existing| existing.name == mount.name)
                    .map(|existing| existing.target.as_str())
                    .unwrap_or("?")
            ),
        ));
    }
    if mounts
        .iter()
        .any(|existing| existing.target == mount.target && existing.name != mount.name)
    {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("mount target {} is already in use", mount.target),
        ));
    }
    mounts.retain(|existing| existing.name != mount.name);
    mounts.push(mount.clone());
    crate::mounts::write_manifest_mounts(&bundle, &mounts)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;

    if helper_is_running(id, socket_path)? {
        let rpc_result = rpc(
            socket_path,
            "vm.mount.add",
            json!({
                "vm_id": id,
                "name": mount.name,
                "source": mount.source,
                "target": mount.target,
                "read_only": mount.read_only,
            }),
        );
        let result = match rpc_result {
            Ok(value) => value,
            Err(failure) if guest_mount_unsupported(&failure) => {
                // Helper applies the share before agent fs.mount; old agents
                // lack fs.mount — guest_virtiofs_bind_mount below still binds.
                json!({ "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>() })
            }
            Err(failure) => return Err(failure),
        };
        // Agent may run under PrivateTmp (own mount ns). Docker uses PID 1's
        // namespace — always ensure the bind there (clears //deleted after share swap).
        guest_virtiofs_bind_mount(id, &mount, socket_path)?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.mount",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": "mount added (live)",
                "vm_id": id,
                "live": true,
            },
            "vm_id": id,
            "mount": mount.to_json(),
            "mounts": result.get("mounts").cloned().unwrap_or(json!([])),
            "live": true,
        }));
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.mount",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": "mount recorded (VM stopped; applied on next start)",
            "vm_id": id,
            "live": false,
        },
        "vm_id": id,
        "mount": mount.to_json(),
        "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "live": false,
    }))
}

fn unmount_vm(
    id: &str,
    target: Option<&str>,
    name: Option<&str>,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let _ = read_manifest(&bundle)?;
    let mut mounts = crate::mounts::read_manifest_mounts(&bundle)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;
    let before = mounts.len();
    mounts.retain(|mount| {
        if let Some(name) = name {
            if mount.name == name {
                return false;
            }
        }
        if let Some(target) = target {
            if mount.target == target {
                return false;
            }
        }
        true
    });
    if mounts.len() == before {
        return Err(Failure::new(EXIT_INVALID, "mount not found"));
    }
    crate::mounts::write_manifest_mounts(&bundle, &mounts)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;

    if helper_is_running(id, socket_path)? {
        let mut params = json!({ "vm_id": id });
        if let Some(name) = name {
            params["name"] = json!(name);
        }
        if let Some(target) = target {
            params["target"] = json!(target);
        }
        let result = rpc(socket_path, "vm.mount.remove", params)?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.unmount",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": "mount removed (live)",
                "vm_id": id,
                "live": true,
            },
            "vm_id": id,
            "mounts": result.get("mounts").cloned().unwrap_or(json!([])),
            "live": true,
        }));
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.unmount",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": "mount removed from manifest",
            "vm_id": id,
            "live": false,
        },
        "vm_id": id,
        "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "live": false,
    }))
}

fn list_vm_mounts(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let _ = read_manifest(&bundle)?;
    let mounts = crate::mounts::read_manifest_mounts(&bundle)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;
    let live = if helper_is_running(id, socket_path)? {
        rpc(socket_path, "vm.mount.list", json!({ "vm_id": id })).ok()
    } else {
        None
    };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.mounts",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("{} mount(s)", mounts.len()),
            "vm_id": id,
        },
        "vm_id": id,
        "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "runtime": live,
    }))
}

fn modify_vm(
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

fn agent_upgrade_vm(all: bool, id: Option<&str>, socket_path: &Path) -> Result<Value, Failure> {
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

fn read_manifest(bundle: &Path) -> Result<Value, Failure> {
    let manifest = bundle.join("vm.json");
    if !manifest.is_file() {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("VM bundle not found: {}", bundle.display()),
        ));
    }
    read_manifest_file(&manifest)
}

fn read_manifest_file(path: &Path) -> Result<Value, Failure> {
    let bytes = fs::read(path)
        .map_err(|error| Failure::new(EXIT_VM_DISK, format!("read {}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| Failure::new(EXIT_VM_DISK, format!("parse {}: {error}", path.display())))
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
                EXIT_VM_OP,
                format!("VM {vm_id} did not stop before timeout"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn rpc_timeout_secs(method: &str, params: &Value) -> u64 {
    if method == "vm.exec" || method.starts_with("vm.agent.") {
        params
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(|ms| (ms / 1000).saturating_add(10).max(15))
            .unwrap_or(40)
    } else {
        5
    }
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("supervisor socket {}: {error}", socket_path.display()),
        )
    })?;
    let timeout_secs = rpc_timeout_secs(method, &params);
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout_secs)))
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(timeout_secs.min(10)))))
        .map_err(|error| {
            Failure::new(
                EXIT_SUPERVISOR,
                format!("supervisor timeout setup: {error}"),
            )
        })?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    writeln!(stream, "{request}")
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor request: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor response: {error}")))?;
    let response: Value = serde_json::from_str(&line).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("invalid supervisor response: {error}"),
        )
    })?;
    if let Some(error) = response.get("error") {
        let rpc_code = error["code"].as_i64().unwrap_or(-32031);
        let code = if rpc_code == -32602 {
            EXIT_INVALID
        } else if method.starts_with("net.") {
            EXIT_NETWORK
        } else if method == "vm.exec" || method.starts_with("vm.agent.") {
            EXIT_GUEST
        } else {
            EXIT_VM_OP
        };
        return Err(Failure::new(code, {
            let message = error["message"]
                .as_str()
                .unwrap_or("VM operation failed")
                .to_string();
            if message == "Method not found"
                && (method == "vm.exec" || method.starts_with("vm.agent."))
            {
                format!(
                    "helper does not support {method}; restart the VM \
                         (`vzctl vm stop <id> && vzctl vm start <id>`) after upgrading vz-helper"
                )
            } else {
                message
            }
        }));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "supervisor response has no result"))
}

fn print_human(command: &str, envelope: &Value) {
    match command {
        "vm.list" | "ps" => {
            let vms = envelope["vms"].as_array().cloned().unwrap_or_default();
            if vms.is_empty() {
                println!("no VMs");
            } else {
                println!(
                    "{:<16} {:<10} {:<8} {:<18} {}",
                    "ID", "STATE", "PID", "IPS", "BUNDLE"
                );
                for vm in vms {
                    let id = vm["id"].as_str().unwrap_or("?");
                    let state = vm["state"].as_str().unwrap_or("?");
                    let pid = vm["pid"]
                        .as_u64()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let ips = vm["ips"]
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "-".to_string());
                    let bundle = vm["bundle"].as_str().unwrap_or("-");
                    println!("{id:<16} {state:<10} {pid:<8} {ips:<18} {bundle}");
                }
            }
            if let Some(warnings) = envelope["warnings"].as_array() {
                for warning in warnings {
                    if let Some(message) = warning.as_str() {
                        eprintln!("warning: {message}");
                    }
                }
            }
        }
        "vm.start" | "vm.stop" | "vm.delete" | "vm.transfer" | "vm.attach" | "vm.mount"
        | "vm.unmount" | "vm.modify" | "vm.agent.upgrade" => {
            println!(
                "{}",
                envelope["summary"]["message"].as_str().unwrap_or(command)
            );
        }
        "vm.mounts" => {
            let mounts = envelope["mounts"].as_array().cloned().unwrap_or_default();
            if mounts.is_empty() {
                println!("no mounts");
            } else {
                println!("{:<16} {:<8} {}", "NAME", "MODE", "TARGET");
                for mount in mounts {
                    println!(
                        "{:<16} {:<8} {} ← {}",
                        mount["name"].as_str().unwrap_or("?"),
                        if mount["read_only"].as_bool() == Some(true) {
                            "ro"
                        } else {
                            "rw"
                        },
                        mount["target"].as_str().unwrap_or("?"),
                        mount["source"].as_str().unwrap_or("?")
                    );
                }
            }
        }
        "vm.inspect" => {
            let vm = &envelope["vm"];
            println!(
                "{}  state={}  pid={}",
                vm["id"].as_str().unwrap_or("?"),
                vm["state"].as_str().unwrap_or("?"),
                vm["pid"]
                    .as_u64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!("  bundle: {}", vm["bundle"].as_str().unwrap_or("-"));
            if let Some(serial) = envelope["logs"]["serial"].as_str() {
                println!("  serial log: {serial}");
            }
            if let Some(networks) = envelope["networks"].as_array() {
                for network in networks {
                    println!(
                        "  network: {}  ip={}",
                        network["name"].as_str().unwrap_or("-"),
                        network["ip"].as_str().unwrap_or("-")
                    );
                }
            }
            println!(
                "  agent: {}",
                envelope["agent"]["state"].as_str().unwrap_or("unavailable")
            );
            if let Some(warnings) = envelope["warnings"].as_array() {
                for warning in warnings {
                    if let Some(message) = warning.as_str() {
                        eprintln!("warning: {message}");
                    }
                }
            }
        }
        "vm.logs" => {
            if let Some(lines) = envelope["lines"].as_array() {
                for line in lines {
                    if let Some(text) = line.as_str() {
                        println!("{text}");
                    }
                }
            }
            if let Some(redacted) = envelope["summary"]["redacted"].as_u64() {
                if redacted > 0 {
                    eprintln!("note: redacted {redacted} line(s) that looked like secrets");
                }
            }
        }
        "vm.exec" | "vm.services" => {
            let payload = if command == "vm.services" {
                &envelope["services"]
            } else {
                &envelope["exec"]
            };
            let stdout = payload["stdout"].as_str().unwrap_or("");
            let stderr = payload["stderr"].as_str().unwrap_or("");
            if !stdout.is_empty() {
                print!("{stdout}");
                if !stdout.ends_with('\n') {
                    println!();
                }
            }
            if !stderr.is_empty() {
                eprint!("{stderr}");
                if !stderr.ends_with('\n') {
                    eprintln!();
                }
            }
        }
        "vm.ps" => {
            if let Some(processes) = envelope["processes"].as_array() {
                println!(
                    "{:<8} {:<12} {:<6} {:<6} {}",
                    "PID", "USER", "%CPU", "%MEM", "ARGS"
                );
                for process in processes {
                    if let Some(raw) = process["raw"].as_str() {
                        println!("{raw}");
                        continue;
                    }
                    println!(
                        "{:<8} {:<12} {:<6} {:<6} {}",
                        process["pid"].as_u64().unwrap_or(0),
                        process["user"].as_str().unwrap_or("-"),
                        process["pcpu"].as_str().unwrap_or("-"),
                        process["pmem"].as_str().unwrap_or("-"),
                        process["args"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
        _ => println!("{envelope}"),
    }
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    let envelope = json!({
        "apiVersion": API_VERSION,
        "command": command,
        "status": "fail",
        "exit_code": failure.code,
        "summary": {
            "message": failure.message,
        },
    });
    match format {
        Format::Json => println!("{envelope}"),
        Format::Human => eprintln!("{}", failure.message),
    }
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::new(EXIT_USAGE, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_state() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from("/private/tmp")
            .join(format!("vzctl-vm-{}-{nonce}-{seq}", std::process::id()));
        fs::create_dir_all(path.join("vms")).unwrap();
        path
    }

    fn write_bundle(state: &Path, id: &str, roles: &[&str]) {
        let bundle = state.join("vms").join(id);
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join("vm.json"),
            serde_json::to_string_pretty(&json!({
                "apiVersion": "vzctl.dev/vm-bundle/v1",
                "managed-by": "vzctl",
                "vm_id": id,
                "roles": roles,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_bundle_with_resources(state: &Path, id: &str, cpus: u32, memory_mib: u64) {
        let bundle = state.join("vms").join(id);
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join("vm.json"),
            serde_json::to_string_pretty(&json!({
                "apiVersion": "vzctl.dev/vm-bundle/v1",
                "managed-by": "vzctl",
                "vm_id": id,
                "roles": [],
                "resources": {
                    "cpus": cpus,
                    "memory_mib": memory_mib,
                },
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn parse_lifecycle_commands() {
        let list = parse(
            ["list".into(), "--format".into(), "json".into()].into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(list.operation, Operation::List);
        assert_eq!(list.format, Format::Json);

        let start = parse(["start".into(), "web".into()].into_iter(), false).unwrap();
        assert_eq!(start.operation, Operation::Start { id: "web".into() });

        let stop = parse(
            ["stop".into(), "web".into(), "--wait".into(), "false".into()].into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            stop.operation,
            Operation::Stop {
                id: "web".into(),
                wait: false
            }
        );

        let delete = parse(
            ["delete".into(), "web".into(), "--force".into()].into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            delete.operation,
            Operation::Delete {
                id: "web".into(),
                force: true
            }
        );

        let modify = parse(
            [
                "modify".into(),
                "web".into(),
                "--cpus".into(),
                "4".into(),
                "--memory".into(),
                "2G".into(),
                "--format".into(),
                "json".into(),
            ]
            .into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            modify.operation,
            Operation::Modify {
                id: "web".into(),
                cpus: Some(4),
                memory_mib: Some(2048),
            }
        );
        assert_eq!(modify.format, Format::Json);
    }

    #[test]
    fn parse_agent_upgrade_accepts_vm_id_or_all() {
        let upgrade = parse(
            [
                "agent".into(),
                "upgrade".into(),
                "demo/web".into(),
                "--format".into(),
                "json".into(),
            ]
            .into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            upgrade.operation,
            Operation::AgentUpgrade {
                all: false,
                id: Some("demo/web".into()),
            }
        );
        let all = parse(
            ["agent".into(), "upgrade".into(), "--all".into()].into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            all.operation,
            Operation::AgentUpgrade {
                all: true,
                id: None,
            }
        );
    }

    #[test]
    fn parse_modify_requires_cpus_or_memory() {
        let err = parse(["modify".into(), "web".into()].into_iter(), false).unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
    }

    #[test]
    fn modify_patches_manifest_resources() {
        let _guard = env_lock().lock().unwrap();
        let state = temp_state();
        write_bundle_with_resources(&state, "web", 2, 1024);
        std::env::set_var("VZCTL_STATE_DIR", &state);
        let socket = state.join("vz.sock");
        let envelope = modify_vm("web", Some(4), Some(2048), &socket).unwrap();
        assert_eq!(envelope["command"], "vm.modify");
        assert_eq!(envelope["status"], "ok");
        assert_eq!(envelope["vm"]["resources"]["cpus"], 4);
        assert_eq!(envelope["vm"]["resources"]["memory_mib"], 2048);
        assert_eq!(envelope["restart_required"], false);
        assert_eq!(envelope["live"], false);

        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(state.join("vms/web/vm.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["resources"]["cpus"], 4);
        assert_eq!(manifest["resources"]["memory_mib"], 2048);

        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/vm-modify.json")).unwrap();
        assert_eq!(envelope["apiVersion"], expected["apiVersion"]);
        assert_eq!(envelope["command"], expected["command"]);
        assert_eq!(envelope["status"], expected["status"]);
        assert_eq!(envelope["exit_code"], expected["exit_code"]);
        assert_eq!(envelope["vm"]["resources"], expected["vm"]["resources"]);
        assert_eq!(envelope["live"], expected["live"]);
        assert_eq!(envelope["restart_required"], expected["restart_required"]);
        std::env::remove_var("VZCTL_STATE_DIR");
    }

    #[test]
    fn list_merges_disk_and_runtime() {
        let _guard = env_lock().lock().unwrap();
        let state = temp_state();
        write_bundle(&state, "web", &[]);
        write_bundle(&state, "db", &["router"]);
        let socket = state.join("vz.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            for expected in ["vm.list", "net.list"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                assert!(request.contains(expected), "{request} missing {expected}");
                let body = if expected == "vm.list" {
                    json!({
                        "jsonrpc": "2.0",
                        "result": [{
                            "vm_id": "web",
                            "state": "running",
                            "pid": 4242,
                            "bundle": "/state/vms/web",
                            "updated_at": "2026-01-01T00:00:00Z",
                        }],
                        "id": 1,
                    })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "networks": [],
                            "attachments": [{
                                "vm_id": "web",
                                "network": "lan",
                                "ip": "10.70.0.10",
                            }],
                        },
                        "id": 1,
                    })
                };
                writeln!(stream, "{body}").unwrap();
            }
        });

        std::env::set_var("VZCTL_STATE_DIR", &state);
        let envelope = list_vms("vm.list", &socket).unwrap();
        std::env::remove_var("VZCTL_STATE_DIR");
        server.join().unwrap();
        fs::remove_dir_all(&state).unwrap();

        assert_eq!(envelope["status"], "ok");
        assert_eq!(envelope["summary"]["vms"], 2);
        assert_eq!(envelope["summary"]["running"], 1);
        let web = envelope["vms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vm| vm["id"] == "web")
            .unwrap();
        assert_eq!(web["state"], "running");
        assert_eq!(web["pid"], 4242);
        assert_eq!(web["ips"], json!(["10.70.0.10"]));
        assert_eq!(
            web["networks"],
            json!([{ "name": "lan", "ip": "10.70.0.10" }])
        );
        let db = envelope["vms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vm| vm["id"] == "db")
            .unwrap();
        assert_eq!(db["state"], "stopped");
        assert_eq!(db["roles"], json!(["router"]));
    }

    #[test]
    fn start_stop_delete_roundtrip_with_mock_supervisor() {
        let _guard = env_lock().lock().unwrap();
        let state = temp_state();
        write_bundle(&state, "web", &[]);
        let socket = state.join("vz.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let respond = |stream: &mut std::os::unix::net::UnixStream| {
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                let method = request["method"].as_str().unwrap();
                let body = match method {
                    "vm.start" => json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "vm_id": "web",
                            "state": "starting",
                            "pid": 99,
                            "bundle": request["params"]["bundle"],
                        },
                        "id": 1,
                    }),
                    "vm.stop" => json!({
                        "jsonrpc": "2.0",
                        "result": {"vm_id": "web", "state": "stopped"},
                        "id": 1,
                    }),
                    "vm.list" => json!({
                        "jsonrpc": "2.0",
                        "result": [],
                        "id": 1,
                    }),
                    "vm.purge" => json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "vm_id": "web",
                            "purged": true,
                            "detached_networks": ["lan"],
                            "ports_removed": 0,
                        },
                        "id": 1,
                    }),
                    "net.list" => json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "networks": [],
                            "attachments": [{
                                "vm_id": "web",
                                "network": "lan",
                            }],
                        },
                        "id": 1,
                    }),
                    "net.detach" => json!({
                        "jsonrpc": "2.0",
                        "result": {"vm_id": "web", "network": "lan"},
                        "id": 1,
                    }),
                    other => panic!("unexpected method {other}"),
                };
                writeln!(stream, "{body}").unwrap();
            };

            // start
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream);
            // stop + wait list
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream);
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream);
            // delete: vm.purge
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream);
        });

        std::env::set_var("VZCTL_STATE_DIR", &state);
        let started = start_vm("web", &socket).unwrap();
        assert_eq!(started["vm"]["state"], "starting");
        assert_eq!(started["vm"]["pid"], 99);

        let stopped = stop_vm("web", true, &socket).unwrap();
        assert_eq!(stopped["vm"]["state"], "stopped");

        let deleted = delete_vm("web", false, &socket).unwrap();
        assert_eq!(deleted["vm"]["deleted"], true);
        assert_eq!(deleted["vm"]["detached_networks"], json!(["lan"]));
        assert_eq!(deleted["vm"]["purged_runtime"], true);
        assert!(!state.join("vms/web").exists());
        std::env::remove_var("VZCTL_STATE_DIR");
        server.join().unwrap();
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn force_delete_cleans_orphan_without_bundle() {
        let _guard = env_lock().lock().unwrap();
        let state = temp_state();
        let socket = state.join("vz.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let respond = |stream: &mut std::os::unix::net::UnixStream| {
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                let method = request["method"].as_str().unwrap();
                let body = match method {
                    "vm.purge" => json!({
                        "jsonrpc": "2.0",
                        "result": {
                            "vm_id": "mos",
                            "purged": true,
                            "detached_networks": ["lan"],
                            "ports_removed": 1,
                        },
                        "id": 1,
                    }),
                    other => panic!("unexpected method {other}"),
                };
                writeln!(stream, "{body}").unwrap();
            };
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream);
        });

        std::env::set_var("VZCTL_STATE_DIR", &state);
        assert!(!state.join("vms/mos").exists());
        let missing = delete_vm("mos", false, &socket).unwrap_err();
        assert_eq!(missing.code, EXIT_INVALID);

        let deleted = delete_vm("mos", true, &socket).unwrap();
        assert_eq!(deleted["vm"]["deleted"], true);
        assert_eq!(deleted["vm"]["purged_bundle"], false);
        assert_eq!(deleted["vm"]["purged_runtime"], true);
        assert_eq!(deleted["vm"]["detached_networks"], json!(["lan"]));
        assert_eq!(deleted["vm"]["ports_removed"], 1);
        std::env::remove_var("VZCTL_STATE_DIR");
        server.join().unwrap();
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn console_detach_sequence_is_ctrl_p_ctrl_q() {
        let mut state = ConsoleDetachState::default();
        let mut forward = Vec::new();
        assert!(!consume_console_stdin(&mut state, b"hi", &mut forward));
        assert_eq!(forward, b"hi");
        assert!(!state.saw_ctrl_p);

        forward.clear();
        assert!(!consume_console_stdin(
            &mut state,
            &[CONSOLE_CTRL_P],
            &mut forward
        ));
        assert!(forward.is_empty());
        assert!(state.saw_ctrl_p);

        assert!(consume_console_stdin(
            &mut state,
            &[CONSOLE_CTRL_Q],
            &mut forward
        ));
        assert!(forward.is_empty());

        // Literal Ctrl-P: press it twice.
        state = ConsoleDetachState::default();
        forward.clear();
        assert!(!consume_console_stdin(
            &mut state,
            &[CONSOLE_CTRL_P, CONSOLE_CTRL_P, b'x'],
            &mut forward
        ));
        assert_eq!(forward, &[CONSOLE_CTRL_P, b'x']);
        assert!(!state.saw_ctrl_p);

        // Ctrl-C stays a guest keystroke.
        state = ConsoleDetachState::default();
        forward.clear();
        assert!(!consume_console_stdin(&mut state, &[0x03], &mut forward));
        assert_eq!(forward, &[0x03]);
    }

    #[test]
    fn exec_parses_interactive_tty_flags() {
        let options = parse(
            [
                "exec".into(),
                "web".into(),
                "-it".into(),
                "--".into(),
                "bash".into(),
            ]
            .into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            options.operation,
            Operation::Exec {
                id: "web".into(),
                cmd: vec!["bash".into()],
                cwd: None,
                env: BTreeMap::new(),
                timeout_ms: 30_000,
                interactive: true,
                tty: true,
            }
        );
        let err = parse(
            ["exec".into(), "web".into(), "-i".into(), "bash".into()].into_iter(),
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, EXIT_INVALID);
    }

    #[test]
    fn parse_logs_and_redact() {
        let options = parse(
            [
                "logs".into(),
                "web".into(),
                "--tail".into(),
                "10".into(),
                "--format".into(),
                "json".into(),
            ]
            .into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(
            options.operation,
            Operation::Logs {
                id: "web".into(),
                follow: false,
                tail: 10,
            }
        );
        assert_eq!(options.format, Format::Json);

        let follow = parse(
            ["logs".into(), "web".into(), "-f".into()].into_iter(),
            false,
        )
        .unwrap();
        assert!(matches!(
            follow.operation,
            Operation::Logs { follow: true, .. }
        ));

        let err = parse(
            [
                "logs".into(),
                "web".into(),
                "-f".into(),
                "--format".into(),
                "json".into(),
            ]
            .into_iter(),
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, EXIT_INVALID);

        assert_eq!(redact_log_line("boot ok"), ("boot ok".into(), false));
        assert_eq!(
            redact_log_line("root_password: secret"),
            ("[redacted]".into(), true)
        );
        assert_eq!(
            redact_log_line("chpasswd: list"),
            ("[redacted]".into(), true)
        );
    }

    #[test]
    fn logs_reads_serial_tail() {
        let _guard = env_lock().lock().unwrap();
        let state = temp_state();
        let logs = state.join("logs");
        fs::create_dir_all(&logs).unwrap();
        write_bundle(&state, "web", &[]);
        let path = logs.join(format!("{}.serial.log", state_file_component("web")));
        fs::write(&path, "line1\npassword=secret\nline3\nline4\n").unwrap();

        std::env::set_var("VZCTL_STATE_DIR", &state);
        std::env::set_var("VZCTL_LOGS_DIR", &logs);
        let envelope = logs_vm("web", false, 3).unwrap();
        std::env::remove_var("VZCTL_STATE_DIR");
        std::env::remove_var("VZCTL_LOGS_DIR");
        fs::remove_dir_all(&state).unwrap();

        assert_eq!(envelope["command"], "vm.logs");
        assert_eq!(envelope["status"], "ok");
        assert_eq!(envelope["lines"], json!(["[redacted]", "line3", "line4"]));
        assert_eq!(envelope["summary"]["redacted"], 1);
        assert_eq!(envelope["log"]["path"], path.display().to_string());
    }
}
