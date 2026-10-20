mod image_cli;
use image_cli::*;
mod vm_create;
use vm_create::*;
mod cloud_init;
use cloud_init::*;
mod events;
use events::*;
mod doctor;
use doctor::*;

use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

mod builder;
mod certs;
mod config;
mod controllers;
mod debug;
mod diagnose;
mod dns;
mod docker;
mod guest_utils;
mod help;
mod hostbin;
mod image;
mod ingress;
mod iwatch_bin;
mod kubernetes;
mod mounts;
mod network;
mod observability;
mod oidc;
mod port;
mod progress;
mod reconciler;
mod resource_budget;
mod route;
mod services;
mod skill;
mod stack;
mod vm;

const DEFAULT_MIN_FREE_GIB: u64 = 20;
const DEFAULT_DNS_PORT: u16 = 15353;
const CLI_API_VERSION: &str = "vzctl.dev/v1";
const VM_CREATE_API_VERSION: &str = "vzctl.dev/v2";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID_INPUT: u8 = 3;
const EXIT_INCOMPLETE_JOURNAL: u8 = 5;
const EXIT_LEASE_HELD: u8 = 6;
const EXIT_SUPERVISOR_UNHEALTHY: u8 = 10;
const EXIT_HOST_UNSUPPORTED: u8 = 11;
const EXIT_UNAVAILABLE: u8 = 12;
const EXIT_IMAGE_CUSTOMIZE_FAILED: u8 = 13;
const EXIT_IMAGE_INVARIANT_FAILED: u8 = 14;
const EXIT_IMAGE_STATE_FAILED: u8 = 15;
const EXIT_VM_DISK_PREP_FAILED: u8 = 16;
pub(crate) const IMAGE_PRESERVATION_CHECKS: &[&str] = &[
    "test -x /usr/local/sbin/vzctl-agent && test -s /usr/local/sbin/vzctl-agent",
    "test -s /usr/lib/vzctl-agent/image-metadata.json",
    // systemd (Ubuntu/Debian/…) or OpenRC (Alpine)
    "(test -s /etc/systemd/system/vzctl-agent.service && test -s /etc/systemd/system/vzctl-agent.path && test -L /etc/systemd/system/multi-user.target.wants/vzctl-agent.service && test -L /etc/systemd/system/multi-user.target.wants/vzctl-agent.path) || (test -s /etc/init.d/vzctl-agent && test -L /etc/runlevels/default/vzctl-agent)",
];
pub(crate) const IMAGE_CLEANUP_COMMANDS: &[&str] = &[
    // Alpine's cloud-init may lack --machine-id; fall back gracefully.
    "cloud-init clean --logs --machine-id 2>/dev/null || cloud-init clean --logs 2>/dev/null || true",
    "truncate -s 0 /etc/machine-id 2>/dev/null || : > /etc/machine-id",
    // Avoid shell globs (ash/nullglob differences); find is portable.
    "rm -f /var/lib/dbus/machine-id /var/lib/systemd/random-seed; find /etc/ssh -maxdepth 1 -type f -name 'ssh_host_*' -delete 2>/dev/null || true",
];
pub(crate) const IMAGE_CLONE_SAFE_CHECKS: &[&str] = &[
    "test ! -s /etc/machine-id",
    "test ! -e /var/lib/dbus/machine-id",
    "! find /etc/ssh -maxdepth 1 -type f -name 'ssh_host_*' -print -quit | grep -q .",
    "test ! -e /var/lib/systemd/random-seed",
    "! find /var/lib/cloud/instances -mindepth 1 -print -quit 2>/dev/null | grep -q .",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Eq, PartialEq)]
struct DoctorOptions {
    format: OutputFormat,
    min_free_gib: u64,
    stack: Option<PathBuf>,
}

#[derive(Debug, Eq, PartialEq)]
struct EventsOptions {
    filter: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct ImageSealOptions {
    input: String,
    tag: Option<String>,
    format: OutputFormat,
    force: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct ImagePullOptions {
    alias: String,
    format: OutputFormat,
}

#[derive(Debug)]
struct ImageSealResult {
    name: String,
    source_path: PathBuf,
    image_format: String,
    marker_path: PathBuf,
    already_sealed: bool,
}

const DEFAULT_VM_CPUS: u32 = 2;
const DEFAULT_VM_MEMORY_MIB: u64 = 1024;

#[derive(Debug, Eq, PartialEq)]
struct VmCreateOptions {
    id: String,
    from: String,
    disk_gib: u64,
    cpus: u32,
    memory_mib: u64,
    roles: Vec<String>,
    requested_network: Option<String>,
    network: Option<network::VmNetworkSelection>,
    /// All NIC attachments (multi-homed router). When empty, falls back to `network`.
    networks: Vec<network::VmNetworkSelection>,
    root_password: Option<String>,
    cloud_init: Option<PathBuf>,
    project: Option<String>,
    mounts: Vec<mounts::ResolvedMount>,
    format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloneMode {
    Linked,
}

impl CloneMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
        }
    }
}

#[derive(Debug)]
struct VmCreateResult {
    id: String,
    bundle_path: PathBuf,
    source: ImageSealResult,
    root_disk_path: PathBuf,
    cidata_path: PathBuf,
    agent_token_path: PathBuf,
    cloud_init_summary: CloudInitSummary,
    disk_gib: u64,
    cpus: u32,
    memory_mib: u64,
    roles: Vec<String>,
    mounts: Vec<mounts::ResolvedMount>,
    clone_mode: CloneMode,
    identity: VmIdentity,
    network: Option<network::VmNetworkSelection>,
    networks: Vec<network::VmNetworkSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CloudInitSummary {
    datasource: &'static str,
    custom: bool,
    roles: Vec<String>,
    packages: Vec<String>,
    write_files: Vec<String>,
    commands: usize,
    users: usize,
}

#[derive(Debug)]
struct VmIdentity {
    instance_id: String,
    hostname: String,
    fqdn: String,
    mac_addresses: Vec<String>,
}

#[derive(Debug)]
struct SealFailure {
    code: u8,
    message: String,
}

impl SealFailure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct VmCreateFailure {
    code: u8,
    message: String,
}

impl VmCreateFailure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

trait ImageSealBackend {
    fn inspect_format(&self, path: &Path) -> Result<String, SealFailure>;
    fn verify_preserved(&self, path: &Path, image_format: &str) -> Result<(), SealFailure>;
    fn customize(&self, path: &Path, image_format: &str) -> Result<(), SealFailure>;
    fn verify_clone_safe(&self, path: &Path, image_format: &str) -> Result<(), SealFailure>;

    /// One logical seal operation. Local backends run the steps sequentially;
    /// Builder-VM backends override this to use a single appliance boot.
    fn seal_pipeline(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        self.verify_preserved(path, image_format)?;
        self.customize(path, image_format)?;
        self.verify_preserved(path, image_format)?;
        self.verify_clone_safe(path, image_format)?;
        Ok(())
    }
}

trait VmDiskBackend {
    fn filesystem_type(&self, path: &Path) -> Option<String>;
    fn clone_linked(&self, source: &Path, destination: &Path) -> Result<(), io::Error>;
    fn create_cloud_init_iso(
        &self,
        seed_directory: &Path,
        destination: &Path,
    ) -> Result<(), io::Error>;
}

struct LibguestfsBackend;
struct BuilderVmBackend {
    images_dir: PathBuf,
    progress: bool,
}
struct NativeVmDiskBackend;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Check {
    pub(crate) id: &'static str,
    pub(crate) status: CheckStatus,
    pub(crate) message: String,
    pub(crate) details: Value,
}

impl Check {
    fn new(
        id: &'static str,
        status: CheckStatus,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            id,
            status,
            message: message.into(),
            details,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "status": self.status.as_str(),
            "message": self.message,
            "details": self.details,
        })
    }
}

fn ignore_sigpipe() {
    // Rust's default stdout writer panics on EPIPE; UI streaming closes pipes on kill.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

fn main() -> ExitCode {
    // Piped stdout (UI/supervisor) must not abort the process on EPIPE.
    ignore_sigpipe();

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => help::command(std::iter::empty()),
        Some(command) if help::is_help(command) => help::command(args),
        Some("version") => help::with_help("version", args, |args| {
            let args: Vec<_> = args.collect();
            let json = json_requested(&args);
            match parse_format_options(args.into_iter(), "version") {
                Ok(OutputFormat::Human) => {
                    println!("vzctl {}", env!("CARGO_PKG_VERSION"));
                    ExitCode::SUCCESS
                }
                Ok(OutputFormat::Json) => {
                    println!("{}", version_json(env!("CARGO_PKG_VERSION")));
                    ExitCode::SUCCESS
                }
                Err(error) => emit_command_failure(json, "version", EXIT_USAGE, &error),
            }
        }),
        Some("doctor") => help::with_help("doctor", args, |args| {
            let args: Vec<_> = args.collect();
            let json = json_requested(&args);
            match parse_doctor_options(args.into_iter()) {
                Ok(options) => doctor(options),
                Err(error) => emit_command_failure(json, "doctor", EXIT_INVALID_INPUT, &error),
            }
        }),
        Some("validate") => help::with_help("validate", args, config::command),
        Some(command @ ("plan" | "diff" | "up" | "down" | "apply" | "adopt")) => {
            help::with_help(command, args, |args| {
                reconciler::command(command, args, &supervisor_socket_path())
            })
        }
        Some("controller") => help::with_help("controller", args, |args| {
            controllers::run(args, &supervisor_socket_path())
        }),
        Some("kubernetes") => help::with_help("kubernetes", args, |args| {
            kubernetes::command(args, &state_dir(), &supervisor_socket_path())
        }),
        Some("diagnose") => help::with_help("diagnose", args, |args| {
            diagnose::command(args, &state_dir(), &supervisor_socket_path())
        }),
        Some("events") => help::with_help("events", args, events_command),
        Some("net") => help::with_help("net", args, |args| {
            network::command(args, &supervisor_socket_path())
        }),
        Some("route") => help::with_help("route", args, |args| {
            route::command(args, &supervisor_socket_path())
        }),
        Some("stack") => help::with_help("stack", args, |args| {
            stack::command(args, &supervisor_socket_path())
        }),
        Some("dns") => help::with_help("dns", args, |args| {
            dns::command(args, &supervisor_socket_path())
        }),
        Some("debug") => help::with_help("debug", args, |args| {
            debug::command(args, &supervisor_socket_path())
        }),
        Some("docker") => help::with_help("docker", args, |args| {
            docker::command(args, &state_dir(), &supervisor_socket_path())
        }),
        Some("port") => help::with_help("port", args, |args| {
            port::command(args, &supervisor_socket_path())
        }),
        Some("services") => help::with_help("services", args, services::command),
        Some("skill") => help::with_help("skill", args, skill::command),
        Some("certs") => help::with_help("certs", args, |args| certs::command(args, &state_dir())),
        Some("oidc") => help::with_help("oidc", args, |args| {
            oidc::command(args, &state_dir(), &supervisor_socket_path())
        }),
        Some("image") => help::with_help("image", args, image_command),
        Some("vm") => help::with_help("vm", args, vm_command),
        Some("ps") => help::with_help("ps", args, |args| {
            vm::ps_command(args, &supervisor_socket_path())
        }),
        Some("status") => help::with_help("stack", args, |args| {
            observability::status_alias(args, &supervisor_socket_path())
        }),
        Some(other) => {
            let args: Vec<_> = args.collect();
            emit_command_failure(
                json_requested(&args),
                "unknown",
                EXIT_USAGE,
                &format!("unknown command: {other}"),
            )
        }
    }
}

pub(crate) fn json_requested(args: &[String]) -> bool {
    args.windows(2).any(|pair| pair == ["--format", "json"])
}

pub(crate) fn emit_command_failure(
    json_output: bool,
    command: &str,
    code: u8,
    message: &str,
) -> ExitCode {
    eprintln!("{message}");
    if json_output {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": command,
                "status": "fail",
                "exit_code": code,
                "summary": { "message": message },
                "error": { "code": code, "message": message },
            })
        );
    }
    ExitCode::from(code)
}

fn parse_format_options(
    args: impl Iterator<Item = String>,
    command: &str,
) -> Result<OutputFormat, String> {
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--format requires human or json".to_string())?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => return Err(format!("unsupported {command} format: {value}")),
                };
            }
            _ => return Err(format!("unknown {command} option: {arg}")),
        }
    }

    Ok(format)
}

fn version_json(version: &str) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "version",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("vzctl {version}"),
        },
        "version": {
            "cli": version,
        },
    })
}

fn state_dir() -> PathBuf {
    if let Some(directory) = std::env::var_os("VZCTL_STATE_DIR") {
        return PathBuf::from(directory);
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("vzctl")
}

fn supervisor_socket_path() -> PathBuf {
    state_dir().join("vz.sock")
}

fn macos_major() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        s.trim().split('.').next()?.parse().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
