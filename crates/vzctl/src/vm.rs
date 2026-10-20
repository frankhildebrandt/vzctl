// VM command dispatch and public guest execution API.
mod args;
mod guest;
mod inventory;
mod lifecycle;
mod logs;
mod mounts;
mod output;
mod rpc;
mod terminal;

use args::*;
use guest::*;
use inventory::*;
use lifecycle::*;
use logs::*;
use mounts::*;
use output::*;
use rpc::*;
use terminal::*;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeVia {
    Dns,
    Ip,
    Both,
}

impl ProbeVia {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Ip => "ip",
            Self::Both => "both",
        }
    }

    fn parse(value: &str) -> Result<Self, Failure> {
        match value {
            "dns" => Ok(Self::Dns),
            "ip" => Ok(Self::Ip),
            "both" => Ok(Self::Both),
            other => Err(usage(format!(
                "unsupported --via: {other} (expected dns, ip, or both)"
            ))),
        }
    }
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
    Restart {
        id: String,
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
        unit_type: String,
        all: bool,
    },
    GuestPs {
        id: String,
    },
    Logs {
        id: String,
        follow: bool,
        tail: usize,
        source: String,
        list_sources: bool,
        q: Option<String>,
        min_level: Option<String>,
        group_field: Option<String>,
        group_value: Option<String>,
        filters: Vec<(String, String)>,
        restart: bool,
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
    Probe {
        id: String,
        target: String,
        via: ProbeVia,
        timeout_ms: u64,
    },
    Health {
        id: String,
    },
    Stats {
        id: String,
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
    Status(String),
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
            Operation::Restart { .. } => "vm.restart",
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
            Operation::Probe { .. } => "vm.probe",
            Operation::Health { .. } => "vm.health",
            Operation::Stats { .. } => "vm.stats",
        }
    }
}

fn execute(options: &Options, socket_path: &Path) -> Result<Value, Failure> {
    match &options.operation {
        Operation::List | Operation::HostPs => list_vms(options.command(), socket_path),
        Operation::Start { id } => start_vm(id, socket_path),
        Operation::Stop { id, wait } => stop_vm(id, *wait, socket_path),
        Operation::Restart { id } => restart_vm(id, socket_path),
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
        } => exec_vm(id, cmd, cwd.as_deref(), env, *timeout_ms, socket_path).or_else(|failure| {
            maybe_hint_agent_timeout(id, socket_path, &failure);
            Err(failure)
        }),
        Operation::Exec { .. } => Err(Failure::new(
            EXIT_INVALID,
            "vm exec interactive tty requires both -i/--interactive and -t/--tty (use -it)",
        )),
        Operation::Transfer { id, src, dst } => transfer_vm(id, src, dst, socket_path),
        Operation::Attach { id } => attach_vm(id),
        Operation::Services {
            id,
            action,
            unit_type,
            all,
        } => services_vm(id, action, unit_type, *all, socket_path),
        Operation::GuestPs { id } => guest_ps_vm(id, socket_path),
        Operation::Logs {
            id,
            follow,
            tail,
            source,
            list_sources,
            q,
            min_level,
            group_field,
            group_value,
            filters,
            restart,
        } => logs_vm(
            id,
            *follow,
            *tail,
            source,
            *list_sources,
            q.as_deref(),
            min_level.as_deref(),
            group_field.as_deref(),
            group_value.as_deref(),
            filters,
            *restart,
            socket_path,
        ),
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
        Operation::Probe {
            id,
            target,
            via,
            timeout_ms,
        } => probe_vm(id, target, *via, *timeout_ms, socket_path),
        Operation::Health { id } => health_vm(id, socket_path),
        Operation::Stats { id } => stats_vm(id, socket_path),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExecOutcome {
    pub exit: u8,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

pub(crate) fn exec_in_vm(
    id: &str,
    cmd: &[String],
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
    timeout_ms: u64,
    socket_path: &Path,
) -> Result<ExecOutcome, String> {
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
    let result = rpc(socket_path, "vm.exec", params).map_err(|failure| failure.message)?;
    Ok(ExecOutcome {
        exit: result["exit"].as_u64().unwrap_or(1) as u8,
        truncated: result["truncated"].as_bool().unwrap_or(false),
        stdout: result["stdout"].as_str().unwrap_or("").to_string(),
        stderr: result["stderr"].as_str().unwrap_or("").to_string(),
    })
}

/// Serial attach runs the host TTY in raw mode (`cfmakeraw` clears `ISIG`), so
/// Ctrl-C is a guest keystroke — not host SIGINT. Detach like Docker: Ctrl-P
/// then Ctrl-Q. Send a literal Ctrl-P to the guest with Ctrl-P Ctrl-P.
const CONSOLE_CTRL_P: u8 = 0x10;
const CONSOLE_CTRL_Q: u8 = 0x11;

pub(crate) fn supervisor_rpc_deadline(
    socket_path: &Path,
    method: &str,
    params: Value,
    timeout_secs: u64,
) -> Result<Value, (u8, String)> {
    rpc_with_timeout(socket_path, method, params, timeout_secs)
        .map_err(|failure| (failure.code, failure.message))
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::new(EXIT_USAGE, message)
}

#[cfg(test)]
mod tests;
