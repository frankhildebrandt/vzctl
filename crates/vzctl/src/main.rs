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
mod dns;
mod docker;
mod image;
mod ingress;
mod mounts;
mod network;
mod oidc;
mod port;
mod reconciler;
mod route;
mod vm;

const DEFAULT_MIN_FREE_GIB: u64 = 20;
const DEFAULT_DNS_PORT: u16 = 15353;
const CLI_API_VERSION: &str = "vzctl.dev/v1";
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
    data_disk_gib: u64,
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
    Full,
}

impl CloneMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Full => "full",
        }
    }
}

#[derive(Debug)]
struct VmCreateResult {
    id: String,
    bundle_path: PathBuf,
    source: ImageSealResult,
    root_disk_path: PathBuf,
    data_disk_path: PathBuf,
    cidata_path: PathBuf,
    agent_token_path: PathBuf,
    data_disk_gib: u64,
    cpus: u32,
    memory_mib: u64,
    roles: Vec<String>,
    mounts: Vec<mounts::ResolvedMount>,
    clone_mode: CloneMode,
    filesystem: String,
    identity: VmIdentity,
    network: Option<network::VmNetworkSelection>,
    networks: Vec<network::VmNetworkSelection>,
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
    fn copy_full(&self, source: &Path, destination: &Path) -> Result<(), io::Error>;
    fn create_sparse(&self, path: &Path, size_bytes: u64) -> Result<(), io::Error>;
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
enum CheckStatus {
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
struct Check {
    id: &'static str,
    status: CheckStatus,
    message: String,
    details: Value,
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
        None | Some("help") | Some("-h") | Some("--help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("version") => match parse_format_options(args, "version") {
            Ok(OutputFormat::Human) => {
                println!("vzctl {}", env!("CARGO_PKG_VERSION"));
                ExitCode::SUCCESS
            }
            Ok(OutputFormat::Json) => {
                println!("{}", version_json(env!("CARGO_PKG_VERSION")));
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        Some("doctor") => match parse_doctor_options(args) {
            Ok(options) => doctor(options),
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(EXIT_INVALID_INPUT)
            }
        },
        Some("validate") => config::command(args),
        Some(command @ ("plan" | "diff" | "up" | "down" | "apply" | "adopt")) => {
            reconciler::command(command, args, &supervisor_socket_path())
        }
        Some("events") => events_command(args),
        Some("net") => network::command(args, &supervisor_socket_path()),
        Some("route") => route::command(args, &supervisor_socket_path()),
        Some("dns") => dns::command(args, &supervisor_socket_path()),
        Some("docker") => docker::command(args, &state_dir(), &supervisor_socket_path()),
        Some("port") => port::command(args, &supervisor_socket_path()),
        Some("certs") => certs::command(args, &state_dir()),
        Some("oidc") => oidc::command(args, &state_dir(), &supervisor_socket_path()),
        Some("image") => image_command(args),
        Some("vm") => vm_command(args),
        Some("ps") => vm::ps_command(args, &supervisor_socket_path()),
        Some(other) => {
            eprintln!("unknown command: {other}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn print_help() {
    println!(
        "\
vzctl — Environments-as-Code for macOS Virtualization (Alpha)

Commands:
  doctor [--format human|json] [--min-free-gib N]
                      Check host baseline and supervisor health
  version [--format human|json]
  validate [-C <directory|config>] [--format human|json]
  validate --schema   Export hypernetwork/v1 JSON Schema
  plan|diff [-C <directory|config>] [--format human|json]
  up [-C <directory|config>] [--force] [--format human|json]
  apply [-C <directory|config>] [--force|--resume|--abort] [--format human|json]
  down [-C <directory|config>] [--purge] [--format human|json]
  adopt [-C <directory|config>] [--format human|json]
  events subscribe [--filter 'vm.*,apply.*']
  net create <name> --cidr CIDR [--mode shared] [--label key=value] [--project P] [--stack S]
  net attach <vm> --network <name> --ip <address> [--label key=value] [--project P] [--stack S]
  net list [--format human|json]
  net detach <vm> --network <name> [--format human|json]
  net delete <name> [--format human|json]
  net default show [--format human|json]
  net default set <name> --cidr CIDR [--format human|json]
  route apply|plan [--config <path>] [--router <vm-id>] [--format human|json]
  route status [--router <vm-id>] [--format human|json]
  dns status [--format human|json]
  dns query <name> [--type A|AAAA] [--server IP:port] [--format human|json]
  dns install-resolver|uninstall-resolver [--project P] [--config <path>] [--format human|json]
  dns install-bind-helper|uninstall-bind-helper [--format human|json]
  docker [--project P] [--format human|json] <ps|inspect|start|stop|restart|run> ...
  docker [--project P] [--] <docker-args...>
  port list [--project P] [--stack S] [--format human|json]
  certs ca init|install [--force] [--format human|json]
  certs mint <san> [--san alias...] [--format human|json]
  certs fingerprint [--format human|json]
  oidc status|clients [--project P] [--format human|json]
  image list [--format human|json]
  image pull <alias> [--format human|json]
  image bake <alias> --tag <tag> [--format human|json]
  image seal <name|path> --tag <tag> [--format human|json]
  vm create <id> --from <sealed> --data-disk <GiB> [--cpus N] [--memory <SIZE>] [--network <name>] [--role router|docker] [--cloud-init PATH] [--project P] [--root-password <secret>] [--format human|json]
  vm list [--format human|json]
  vm start <id> [--format human|json]
  vm stop <id> [--wait true|false] [--format human|json]
  vm delete <id> [--force] [--format human|json]
  vm modify <id> [--cpus N] [--memory <SIZE>] [--format human|json]
  vm inspect <id> [--format human|json]
  vm logs <id> [-f|--follow] [--tail N] [--format human|json]
  vm exec <id> [-it] [--cwd PATH] [--env K=V]... [--timeout-ms N] [--] <cmd> [args...]
  vm transfer <id> <src> <dst> [--format human|json]
  vm attach <id>
  vm services <id> [start|stop|restart <unit>] [--format human|json]
  vm ps <id> [--format human|json]
  ps [--format human|json]
  help

Stable exit codes:
  0   success (warnings allowed)
  2   usage or unknown command
  3   invalid input or validation
  5   incomplete apply journal
  6   apply lease held
  10  supervisor socket or health is bad
  11  macOS 26 baseline is not met
  12  command backend unavailable or not implemented
  13  image customization failed
  14  image seal invariant failed
  15  image seal state/marker failed
  16  VM root/data disk preparation failed
  17  network operation failed
  18  route or guest-agent operation failed
  19  resolver operation failed
  20  DNS query failed or returned a non-zero rcode
  21  image download/metadata network failure
  22  image checksum mismatch or invalid checksum metadata
  23  image architecture unsupported
  24  reconciler or VM lifecycle operation failed"
    );
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

fn image_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        Some("list") => image_list_command(args.collect()),
        Some("pull") => image_pull_command(args.collect()),
        Some("bake") => image_bake_command(args.collect()),
        Some("seal") => image_seal_command(args.collect()),
        Some(command) => {
            eprintln!("unknown image command: {command}");
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            eprintln!(
                "usage: vzctl image list [--format human|json] | \
                 image pull <alias> [--format human|json] | \
                 image bake <alias> --tag <tag> [--format human|json] | \
                 image seal <name|path> --tag <tag> [--format human|json]"
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn image_list_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let format = match parse_format_options(args.into_iter(), "image list") {
        Ok(format) => format,
        Err(message) => {
            emit_image_list_failure(
                requested_format,
                &image::PullFailure {
                    code: EXIT_USAGE,
                    message,
                },
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match image::list(&images_dir()) {
        Ok(result) => {
            match format {
                OutputFormat::Human => print_image_list_human(&result),
                OutputFormat::Json => println!("{}", image_list_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_image_list_failure(format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn print_image_list_human(result: &image::ListResult) {
    println!("images_dir: {}", result.images_dir.display());
    if result.images.is_empty() {
        println!("(no local images)");
    } else {
        println!(
            "{:<24} {:<10} {:<8} {:<8} {}",
            "ALIAS", "STATE", "BAKED", "SEALED", "PATH"
        );
        for image in &result.images {
            let state = if image.sealed {
                "sealed"
            } else if image.baked {
                "baked"
            } else {
                "pulled"
            };
            println!(
                "{:<24} {:<10} {:<8} {:<8} {}",
                image.alias,
                state,
                if image.baked { "yes" } else { "no" },
                if image.sealed { "yes" } else { "no" },
                image.path.display()
            );
        }
    }
    println!();
    println!("catalog ({} aliases):", result.catalog.len());
    for entry in &result.catalog {
        let aliases = if entry.aliases.len() > 1 {
            format!(" ({})", entry.aliases.join(", "))
        } else {
            String::new()
        };
        println!(
            "  {}{} — {} {}",
            entry.alias, aliases, entry.distribution, entry.release
        );
    }
}

fn image_list_json(result: &image::ListResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.list",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.images.is_empty() {
                "no local images"
            } else {
                "image cache listed"
            },
            "count": result.images.len(),
            "images_dir": result.images_dir,
        },
        "images": result.images.iter().map(|image| json!({
            "alias": image.alias,
            "canonical_alias": image.canonical_alias,
            "aliases": image.aliases,
            "distribution": image.distribution,
            "release": image.release,
            "architecture": image.architecture,
            "path": image.path,
            "format": image.format,
            "sha256": image.sha256,
            "baked": image.baked,
            "sealed": image.sealed,
            "agent_version": image.agent_version,
            "tags": image.tags.iter().map(|tag| json!({
                "tag": tag.tag,
                "path": tag.path,
                "format": tag.format,
                "sha256": tag.sha256,
                "baked": tag.baked,
                "sealed": tag.sealed,
                "agent_version": tag.agent_version,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "catalog": result.catalog.iter().map(|entry| json!({
            "alias": entry.alias,
            "aliases": entry.aliases,
            "distribution": entry.distribution,
            "release": entry.release,
        })).collect::<Vec<_>>(),
    })
}

fn emit_image_list_failure(format: OutputFormat, failure: &image::PullFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.list",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

fn image_pull_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_pull_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_image_pull_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    match image::pull(&options.alias, &images_dir()) {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => print_image_pull_human(&result),
                OutputFormat::Json => println!("{}", image_pull_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_image_pull_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn parse_image_pull_options(
    args: impl Iterator<Item = String>,
) -> Result<ImagePullOptions, image::PullFailure> {
    let mut alias = None;
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" => {
                let value = args.next().ok_or_else(|| image::PullFailure {
                    code: EXIT_USAGE,
                    message: "--format requires human or json".to_string(),
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(image::PullFailure {
                            code: EXIT_USAGE,
                            message: format!("unsupported image pull format: {value}"),
                        })
                    }
                };
            }
            "-h" | "--help" => {
                return Err(image::PullFailure {
                    code: EXIT_USAGE,
                    message: "usage: vzctl image pull <alias> [--format human|json]".to_string(),
                })
            }
            _ if arg.starts_with('-') => {
                return Err(image::PullFailure {
                    code: EXIT_USAGE,
                    message: format!("unknown image pull option: {arg}"),
                })
            }
            _ if alias.is_none() => alias = Some(arg),
            _ => {
                return Err(image::PullFailure {
                    code: EXIT_USAGE,
                    message: format!("unexpected image pull argument: {arg}"),
                })
            }
        }
    }
    let alias = alias.ok_or_else(|| image::PullFailure {
        code: EXIT_USAGE,
        message: "usage: vzctl image pull <alias> [--format human|json]".to_string(),
    })?;
    Ok(ImagePullOptions { alias, format })
}

fn print_image_pull_human(result: &image::PullResult) {
    println!(
        "Image {} {}",
        result.requested_alias,
        if result.unchanged {
            "is unchanged"
        } else {
            "pulled"
        }
    );
    println!(
        "  release: {} {} (arm64)",
        result.distribution, result.release
    );
    println!("  sha256: {}", result.normalized_digest);
    println!("  path: {}", result.image_path.display());
    println!("  sealed: {}", if result.sealed { "yes" } else { "no" });
}

fn image_pull_json(result: &image::PullResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.pull",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.unchanged { "image unchanged" } else { "image pulled" },
            "change": if result.unchanged { "unchanged" } else { "pulled" },
        },
        "image": {
            "alias": result.requested_alias,
            "canonical_alias": result.canonical_alias,
            "aliases": result.aliases,
            "distribution": result.distribution,
            "release": result.release,
            "architecture": "arm64",
            "path": result.image_path,
            "format": "raw",
            "sha256": result.normalized_digest,
            "sealed": result.sealed,
            "manifest": result.manifest_path,
        },
        "source": {
            "url": result.source_url,
            "format": result.source_format,
            "algorithm": result.source_algorithm,
            "digest": result.source_digest,
        },
    })
}

fn emit_image_pull_failure(format: OutputFormat, failure: &image::PullFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.pull",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

fn image_bake_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_bake_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_bake_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    match bake_image(&options) {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => {
                    println!(
                        "Image {} {}",
                        result.requested_alias,
                        if result.unchanged {
                            "bake unchanged"
                        } else {
                            "baked"
                        }
                    );
                    println!("  agent: {}", result.agent_version);
                    println!("  path: {}", result.image_path.display());
                }
                OutputFormat::Json => println!("{}", image_bake_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_bake_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ImageBakeOptions {
    alias: String,
    tag: String,
    format: OutputFormat,
}

fn parse_image_bake_options(
    args: impl Iterator<Item = String>,
) -> Result<ImageBakeOptions, SealFailure> {
    let mut alias = None;
    let mut tag = None;
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => {
                let value = args.next().ok_or_else(|| SealFailure {
                    code: EXIT_USAGE,
                    message: "--tag requires a value".to_string(),
                })?;
                if !image::valid_image_tag(&value) {
                    return Err(SealFailure {
                        code: EXIT_USAGE,
                        message: format!(
                            "invalid image tag {value}; expected 1-64 [A-Za-z0-9][A-Za-z0-9._-]*"
                        ),
                    });
                }
                tag = Some(value);
            }
            "--format" => {
                let value = args.next().ok_or_else(|| SealFailure {
                    code: EXIT_USAGE,
                    message: "--format requires human or json".to_string(),
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(SealFailure {
                            code: EXIT_USAGE,
                            message: format!("unsupported image bake format: {value}"),
                        })
                    }
                };
            }
            "-h" | "--help" => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: "usage: vzctl image bake <alias> --tag <tag> [--format human|json]"
                        .to_string(),
                })
            }
            _ if arg.starts_with('-') => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: format!("unknown image bake option: {arg}"),
                })
            }
            _ if alias.is_none() => alias = Some(arg),
            _ => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: format!("unexpected image bake argument: {arg}"),
                })
            }
        }
    }
    let alias = alias.ok_or_else(|| SealFailure {
        code: EXIT_USAGE,
        message: "usage: vzctl image bake <alias> --tag <tag> [--format human|json]".to_string(),
    })?;
    let tag = tag.ok_or_else(|| SealFailure {
        code: EXIT_USAGE,
        message: "image bake requires --tag <tag>".to_string(),
    })?;
    Ok(ImageBakeOptions { alias, tag, format })
}

fn bake_image(options: &ImageBakeOptions) -> Result<image::BakeResult, SealFailure> {
    let images = images_dir();
    let agent_version = agent_version_string()?;
    if let Some(existing) =
        image::already_baked(&images, &options.alias, &options.tag, &agent_version)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
    {
        return Ok(existing);
    }

    let (target, manifest, _manifest_path) =
        image::prepare_alias_for_bake(&images, &options.alias, &options.tag)
            .map_err(|error| SealFailure::new(EXIT_INVALID_INPUT, error))?;
    let canonical = manifest["canonical_alias"]
        .as_str()
        .unwrap_or(&options.alias)
        .to_string();

    let staging = build_agent_staging(&agent_version)?;
    let progress = (io::stderr().is_terminal() && options.format == OutputFormat::Human)
        || image::progress_env_enabled();
    let backend_kind = builder::select_backend_kind()
        .map_err(|failure| SealFailure::new(failure.code, failure.message))?;

    match backend_kind {
        builder::ImageBackendKind::Local => {
            bake_with_virt_customize(&target, &staging)?;
        }
        builder::ImageBackendKind::Builder => {
            if progress {
                eprintln!("Baking via builder VM…");
            }
            let appliance = builder::resolve_builder_image(&images)
                .map_err(|failure| SealFailure::new(failure.code, failure.message))?;
            let runbook = builder::bake_runbook("staging");
            builder::run_builder_vm(builder::BuilderRunOptions {
                appliance: &appliance,
                target_raw: &target,
                runbook: &runbook,
                staging_dir: Some(&staging),
                timeout: builder::default_timeout(),
                progress,
            })
            .map_err(|failure| SealFailure::new(failure.code, failure.message))?;
        }
    }

    let _ = fs::remove_dir_all(&staging);
    image::mark_aliases_baked(&images, &target, &options.tag, &agent_version)
        .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
    Ok(image::BakeResult {
        requested_alias: options.alias.clone(),
        canonical_alias: canonical,
        tag: options.tag.clone(),
        image_path: target,
        agent_version,
        unchanged: false,
    })
}

fn agent_version_string() -> Result<String, SealFailure> {
    if let Ok(version) = std::env::var("VZCTL_AGENT_VERSION") {
        return Ok(version.trim().to_string());
    }
    let version_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../guest-agent/VERSION");
    fs::read_to_string(&version_path)
        .map(|value| value.trim().to_string())
        .or_else(|_| Ok("0.1.0".to_string()))
}

fn build_agent_staging(agent_version: &str) -> Result<PathBuf, SealFailure> {
    let staging = std::env::temp_dir().join(format!(
        "vzctl-bake-staging-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&staging).map_err(|error| {
        SealFailure::new(
            EXIT_UNAVAILABLE,
            format!("cannot create bake staging: {error}"),
        )
    })?;

    let agent_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../guest-agent");
    let binary = staging.join("vzctl-agent");
    let status = Command::new("go")
        .current_dir(&agent_root)
        .env("CGO_ENABLED", "0")
        .env("GOOS", "linux")
        .env("GOARCH", "arm64")
        .args([
            "build",
            "-trimpath",
            "-ldflags",
            &format!("-s -w -buildid= -X main.version={agent_version}"),
            "-o",
        ])
        .arg(&binary)
        .arg("./cmd/vzctl-agent")
        .status()
        .map_err(|error| {
            SealFailure::new(
                EXIT_UNAVAILABLE,
                format!("go is required to cross-build vzctl-agent: {error}"),
            )
        })?;
    if !status.success() {
        return Err(SealFailure::new(
            EXIT_IMAGE_CUSTOMIZE_FAILED,
            "go build of vzctl-agent failed",
        ));
    }
    fs::copy(
        agent_root.join("systemd/vzctl-agent.service"),
        staging.join("vzctl-agent.service"),
    )
    .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.to_string()))?;
    fs::copy(
        agent_root.join("systemd/vzctl-agent.path"),
        staging.join("vzctl-agent.path"),
    )
    .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.to_string()))?;
    fs::copy(
        agent_root.join("systemd/vzctl-agent-tmpfiles.conf"),
        staging.join("vzctl-agent-tmpfiles.conf"),
    )
    .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.to_string()))?;
    fs::copy(
        agent_root.join("openrc/vzctl-agent"),
        staging.join("vzctl-agent.openrc"),
    )
    .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.to_string()))?;
    fs::write(
        staging.join("image-metadata.json"),
        format!("{{\"agent_version\":\"{agent_version}\",\"protocol\":1,\"vsock_port\":21950}}\n"),
    )
    .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.to_string()))?;
    Ok(staging)
}

fn bake_with_virt_customize(target: &Path, staging: &Path) -> Result<(), SealFailure> {
    let output = Command::new("virt-customize")
        .args(["--format", "raw", "-a"])
        .arg(target)
        .arg("--mkdir")
        .arg("/usr/lib/vzctl-agent")
        .arg("--copy-in")
        .arg(format!("{}:/usr/local/sbin", staging.join("vzctl-agent").display()))
        .arg("--copy-in")
        .arg(format!(
            "{}:/etc/systemd/system",
            staging.join("vzctl-agent.service").display()
        ))
        .arg("--copy-in")
        .arg(format!(
            "{}:/etc/systemd/system",
            staging.join("vzctl-agent.path").display()
        ))
        .arg("--copy-in")
        .arg(format!(
            "{}:/usr/lib/tmpfiles.d",
            staging.join("vzctl-agent-tmpfiles.conf").display()
        ))
        .arg("--copy-in")
        .arg(format!(
            "{}:/usr/lib/vzctl-agent",
            staging.join("image-metadata.json").display()
        ))
        .arg("--run-command")
        .arg("id -u vzctl-agent >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin vzctl-agent")
        .arg("--run-command")
        .arg("chmod 0755 /usr/local/sbin/vzctl-agent && chmod 0644 /etc/systemd/system/vzctl-agent.service /etc/systemd/system/vzctl-agent.path /usr/lib/tmpfiles.d/vzctl-agent-tmpfiles.conf /usr/lib/vzctl-agent/image-metadata.json")
        .arg("--run-command")
        .arg("systemctl enable vzctl-agent.service vzctl-agent.path")
        .output()
        .map_err(|error| {
            SealFailure::new(
                EXIT_UNAVAILABLE,
                format!("virt-customize is required for local image bake: {error}"),
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SealFailure::new(
            EXIT_IMAGE_CUSTOMIZE_FAILED,
            format!("image bake failed: {}", command_text(&output)),
        ))
    }
}

fn image_bake_json(result: &image::BakeResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.bake",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.unchanged { "image bake unchanged" } else { "image baked" },
            "change": if result.unchanged { "unchanged" } else { "baked" },
        },
        "image": {
            "alias": result.requested_alias,
            "canonical_alias": result.canonical_alias,
            "tag": result.tag,
            "path": result.image_path,
            "format": "raw",
            "baked": true,
            "agent_version": result.agent_version,
        },
    })
}

fn emit_bake_failure(format: OutputFormat, failure: &SealFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.bake",
                "status": "fail",
                "exit_code": failure.code,
                "summary": { "message": failure.message },
            })
        );
    }
}

fn image_seal_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_seal_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_seal_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };

    let images = images_dir();
    let progress = (io::stderr().is_terminal() && options.format == OutputFormat::Human)
        || image::progress_env_enabled();
    let backend_kind = match builder::select_backend_kind() {
        Ok(kind) => kind,
        Err(failure) => {
            emit_seal_failure(
                options.format,
                &SealFailure::new(failure.code, failure.message),
            );
            return ExitCode::from(failure.code);
        }
    };
    let result = match backend_kind {
        builder::ImageBackendKind::Local => seal_image(&options, &LibguestfsBackend),
        builder::ImageBackendKind::Builder => seal_image(
            &options,
            &BuilderVmBackend {
                images_dir: images,
                progress,
            },
        ),
    };
    match result {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => print_image_seal_human(&result),
                OutputFormat::Json => println!("{}", image_seal_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_seal_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn requested_output_format(args: &[String]) -> OutputFormat {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| OutputFormat::Json)
        .unwrap_or(OutputFormat::Human)
}

fn parse_image_seal_options(
    args: impl Iterator<Item = String>,
) -> Result<ImageSealOptions, SealFailure> {
    let mut input = None;
    let mut tag = None;
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => {
                let value = args
                    .next()
                    .ok_or_else(|| SealFailure::new(EXIT_USAGE, "--tag requires a value"))?;
                if !image::valid_image_tag(&value) {
                    return Err(SealFailure::new(
                        EXIT_USAGE,
                        format!(
                            "invalid image tag {value}; expected 1-64 [A-Za-z0-9][A-Za-z0-9._-]*"
                        ),
                    ));
                }
                tag = Some(value);
            }
            "--format" => {
                let value = args.next().ok_or_else(|| {
                    SealFailure::new(EXIT_USAGE, "--format requires human or json")
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(SealFailure::new(
                            EXIT_USAGE,
                            format!("unsupported image seal format: {value}"),
                        ))
                    }
                };
            }
            "-h" | "--help" => {
                return Err(SealFailure::new(
                    EXIT_USAGE,
                    "usage: vzctl image seal <name|path> --tag <tag> [--format human|json]",
                ))
            }
            _ if arg.starts_with('-') => {
                return Err(SealFailure::new(
                    EXIT_USAGE,
                    format!("unknown image seal option: {arg}"),
                ))
            }
            _ if input.is_none() => input = Some(arg),
            _ => {
                return Err(SealFailure::new(
                    EXIT_USAGE,
                    format!("unexpected image seal argument: {arg}"),
                ))
            }
        }
    }

    let input = input.ok_or_else(|| {
        SealFailure::new(
            EXIT_USAGE,
            "usage: vzctl image seal <name|path> --tag <tag> [--format human|json]",
        )
    })?;
    let tag = tag.ok_or_else(|| SealFailure::new(EXIT_USAGE, "image seal requires --tag <tag>"))?;
    Ok(ImageSealOptions {
        input,
        tag: Some(tag),
        format,
    })
}

fn seal_image(
    options: &ImageSealOptions,
    backend: &dyn ImageSealBackend,
) -> Result<ImageSealResult, SealFailure> {
    let images_dir = images_dir();
    seal_image_in_dir(options, backend, &images_dir)
}

fn seal_image_in_dir(
    options: &ImageSealOptions,
    backend: &dyn ImageSealBackend,
    images_dir: &Path,
) -> Result<ImageSealResult, SealFailure> {
    let tag = options.tag.clone().ok_or_else(|| {
        SealFailure::new(
            EXIT_USAGE,
            "image seal requires --tag <tag> for alias inputs",
        )
    })?;

    let source_path = match image::prepare_alias_for_seal(images_dir, &options.input, &tag)
        .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
    {
        Some(path) => path,
        None => {
            // Direct path input: tag still required for marker/manifest bookkeeping
            // when the path is under the images store; otherwise treat as raw path seal.
            resolve_image_input(&options.input, images_dir)?
        }
    };
    let source_path = fs::canonicalize(&source_path).map_err(|error| {
        SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("cannot resolve image {}: {error}", source_path.display()),
        )
    })?;
    if !source_path.is_file() {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("image is not a regular file: {}", source_path.display()),
        ));
    }

    let name = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image")
        .to_string();
    let marker_path = seal_marker_path(images_dir, &source_path);
    if marker_path.exists() {
        let mut result = read_existing_seal(&marker_path, &source_path, name)?;
        result.already_sealed = true;
        if image::tagged_seal_ready(images_dir, &options.input, &tag)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
        {
            return Ok(result);
        }
        // Repair incomplete tag metadata. Prefer the recorded digest; only hash
        // when this is a known alias that still lacks a tag entry.
        let recorded = image::existing_tag_sealed_digest(images_dir, &options.input, &tag)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
        if let Some(digest) = recorded {
            image::mark_aliases_sealed(
                images_dir,
                &result.source_path,
                &result.marker_path,
                &tag,
                Some(&digest),
            )
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
        } else if image::resolve_alias_pulled(images_dir, &options.input)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
            .is_some()
        {
            image::mark_aliases_sealed(
                images_dir,
                &result.source_path,
                &result.marker_path,
                &tag,
                None,
            )
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
        }
        return Ok(result);
    }

    let image_format = backend.inspect_format(&source_path)?;
    if !matches!(image_format.as_str(), "raw" | "qcow" | "qcow2") {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("unsupported image format {image_format}; expected raw or qcow2"),
        ));
    }

    backend.seal_pipeline(&source_path, &image_format)?;

    let result = ImageSealResult {
        name,
        source_path,
        image_format,
        marker_path,
        already_sealed: false,
    };
    write_seal_marker_and_lock(&result)?;
    image::mark_aliases_sealed(
        images_dir,
        &result.source_path,
        &result.marker_path,
        &tag,
        None,
    )
    .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
    Ok(result)
}

fn images_dir() -> PathBuf {
    std::env::var_os("VZCTL_IMAGES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir().join("images"))
}

fn resolve_image_input(input: &str, images_dir: &Path) -> Result<PathBuf, SealFailure> {
    let direct = PathBuf::from(input);
    if direct.exists() {
        return Ok(direct);
    }
    if direct.is_absolute() || direct.components().count() > 1 {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("image path does not exist: {}", direct.display()),
        ));
    }

    match image::resolve_alias(images_dir, input) {
        Ok(Some(path)) => return Ok(path),
        Ok(None) => {}
        Err(error) => return Err(SealFailure::new(EXIT_IMAGE_STATE_FAILED, error)),
    }

    let candidates = ["", ".raw", ".qcow", ".qcow2", ".img"]
        .into_iter()
        .map(|suffix| images_dir.join(format!("{input}{suffix}")))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "image {input} not found in {}; run `vzctl image pull {input}` first",
                images_dir.display()
            ),
        )),
        _ => Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "image name {input} is ambiguous in {}: {}",
                images_dir.display(),
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

fn seal_marker_path(images_dir: &Path, source_path: &Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let safe_stem = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let path_text = source_path.to_string_lossy();
    images_dir.join(format!(
        "{}-{:016x}.sealed.json",
        safe_stem,
        stable_path_hash(path_text.as_bytes())
    ))
}

fn stable_path_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn read_existing_seal(
    marker_path: &Path,
    source_path: &Path,
    name: String,
) -> Result<ImageSealResult, SealFailure> {
    let marker_text = fs::read_to_string(marker_path).map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!("cannot read seal marker {}: {error}", marker_path.display()),
        )
    })?;
    let marker: Value = serde_json::from_str(&marker_text).map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!("invalid seal marker {}: {error}", marker_path.display()),
        )
    })?;
    if marker["apiVersion"] != "vzctl.dev/image-seal/v1"
        || marker["sealed"] != true
        || marker["source_path"] != source_path.to_string_lossy().as_ref()
    {
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!("seal marker does not match {}", source_path.display()),
        ));
    }
    let image_format = marker["format"]
        .as_str()
        .filter(|format| matches!(*format, "raw" | "qcow" | "qcow2"))
        .ok_or_else(|| {
            SealFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!("seal marker has invalid format: {}", marker_path.display()),
            )
        })?
        .to_string();
    let permissions = fs::metadata(source_path)
        .map_err(|error| {
            SealFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!(
                    "cannot inspect sealed image {}: {error}",
                    source_path.display()
                ),
            )
        })?
        .permissions();
    if permissions.mode() & 0o222 != 0 {
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "seal marker exists but image is writable: {}",
                source_path.display()
            ),
        ));
    }
    Ok(ImageSealResult {
        name,
        source_path: source_path.to_path_buf(),
        image_format,
        marker_path: marker_path.to_path_buf(),
        already_sealed: true,
    })
}

fn write_seal_marker_and_lock(result: &ImageSealResult) -> Result<(), SealFailure> {
    fs::create_dir_all(
        result
            .marker_path
            .parent()
            .expect("seal marker always has a parent"),
    )
    .map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot create images directory for {}: {error}",
                result.marker_path.display()
            ),
        )
    })?;
    let sealed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let marker = seal_marker_json(result, sealed_at);
    let temporary_marker = result
        .marker_path
        .with_extension(format!("tmp-{}", std::process::id()));
    fs::write(
        &temporary_marker,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&marker).expect("marker JSON serializes")
        ),
    )
    .map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot write seal marker {}: {error}",
                temporary_marker.display()
            ),
        )
    })?;

    let original_permissions = fs::metadata(&result.source_path)
        .map_err(|error| {
            SealFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!("cannot inspect {}: {error}", result.source_path.display()),
            )
        })?
        .permissions();
    let mut sealed_permissions = original_permissions.clone();
    sealed_permissions.set_mode(original_permissions.mode() & !0o222);
    if let Err(error) = fs::set_permissions(&result.source_path, sealed_permissions) {
        let _ = fs::remove_file(&temporary_marker);
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot make {} read-only: {error}",
                result.source_path.display()
            ),
        ));
    }
    if let Err(error) = fs::rename(&temporary_marker, &result.marker_path) {
        let _ = fs::set_permissions(&result.source_path, original_permissions);
        let _ = fs::remove_file(&temporary_marker);
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot publish seal marker {}: {error}",
                result.marker_path.display()
            ),
        ));
    }
    Ok(())
}

fn seal_marker_json(result: &ImageSealResult, sealed_at: u64) -> Value {
    json!({
        "apiVersion": "vzctl.dev/image-seal/v1",
        "name": result.name,
        "source_path": result.source_path,
        "format": result.image_format,
        "sealed": true,
        "read_only": true,
        "sealed_at_unix_seconds": sealed_at,
        "cleanup": {
            "machine_id": true,
            "dbus_machine_id": true,
            "ssh_host_keys": true,
            "cloud_init": true,
            "random_seed": true,
        },
        "preserved": {
            "agent": "/usr/local/sbin/vzctl-agent",
            "systemd_unit": "/etc/systemd/system/vzctl-agent.service",
            "image_metadata": "/usr/lib/vzctl-agent/image-metadata.json",
        },
    })
}

fn print_image_seal_human(result: &ImageSealResult) {
    println!(
        "{} image {}",
        if result.already_sealed {
            "already sealed"
        } else {
            "sealed"
        },
        result.name
    );
    println!("  source: {}", result.source_path.display());
    println!("  format: {}", result.image_format);
    println!("  marker: {}", result.marker_path.display());
    println!("  read-only: yes");
    println!("  preserved: agent, systemd unit, image metadata");
    println!("  clone-safe: machine-id, SSH keys, cloud-init, random seed cleaned");
}

fn image_seal_json(result: &ImageSealResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.seal",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.already_sealed { "image already sealed" } else { "image sealed" },
            "sealed": true,
            "already_sealed": result.already_sealed,
        },
        "image": {
            "name": result.name,
            "path": result.source_path,
            "format": result.image_format,
            "sealed": true,
            "read_only": true,
            "marker": result.marker_path,
        },
        "cleanup": {
            "machine_id": true,
            "dbus_machine_id": true,
            "ssh_host_keys": true,
            "cloud_init": true,
            "random_seed": true,
        },
        "preserved": {
            "agent": true,
            "systemd_unit": true,
            "image_metadata": true,
        },
    })
}

fn emit_seal_failure(format: OutputFormat, failure: &SealFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.seal",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

fn vm_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        None | Some("help") | Some("-h") | Some("--help") => {
            eprintln!(
                "usage: vzctl vm create|list|start|stop|delete|inspect|logs|exec|transfer|attach|services|ps|mount|unmount|mounts|modify ..."
            );
            ExitCode::from(EXIT_USAGE)
        }
        Some("create") => vm_create_command(args.collect()),
        Some(command) => vm::command(
            std::iter::once(command.to_string()).chain(args),
            &supervisor_socket_path(),
        ),
    }
}

fn vm_create_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let mut options = match parse_vm_create_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_vm_create_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };

    let socket_path = supervisor_socket_path();
    let selection = match network::ensure_vm_network(
        &socket_path,
        &options.id,
        options.requested_network.as_deref(),
    ) {
        Ok(selection) => selection,
        Err(failure) => {
            let failure = VmCreateFailure::new(failure.code, failure.message);
            emit_vm_create_failure(options.format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    options.network = Some(selection.clone());
    // Prefer all DB attachments (attach_nets runs before create) so routers get
    // every NIC; fall back to the ensure selection for single-homed VMs.
    options.networks = match network::list_vm_attachments(&socket_path, &options.id) {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => vec![selection.clone()],
        Err(failure) => {
            let failure = VmCreateFailure::new(failure.code, failure.message);
            emit_vm_create_failure(options.format, &failure);
            return ExitCode::from(failure.code);
        }
    };

    match create_vm_bundle(&options, &NativeVmDiskBackend) {
        Ok(result) => {
            if result.clone_mode == CloneMode::Full {
                eprintln!(
                    "WARN: {} is on {}; created a full root-disk copy instead of an APFS linked clone",
                    result.bundle_path.display(),
                    result.filesystem
                );
            }
            match options.format {
                OutputFormat::Human => print_vm_create_human(&result),
                OutputFormat::Json => println!("{}", vm_create_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            network::rollback_vm_network(&socket_path, &selection, &options.id);
            emit_vm_create_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn parse_vm_create_options(
    args: impl Iterator<Item = String>,
) -> Result<VmCreateOptions, VmCreateFailure> {
    let mut id = None;
    let mut from = None;
    let mut data_disk_gib = None;
    let mut cpus = None;
    let mut memory_mib = None;
    let mut roles = Vec::new();
    let mut requested_network = None;
    let mut root_password = None;
    let mut cloud_init = None;
    let mut project = None;
    let mut mount_list = Vec::new();
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => {
                from = Some(args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--from requires a sealed image")
                })?);
            }
            "--data-disk" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--data-disk requires a GiB size")
                })?;
                let size = value.parse::<u64>().map_err(|_| {
                    VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        format!("invalid data-disk size: {value}"),
                    )
                })?;
                if size == 0 {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "data-disk size must be greater than zero",
                    ));
                }
                data_disk_gib = Some(size);
            }
            "--cpus" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--cpus requires a positive integer")
                })?;
                let parsed = value.parse::<u32>().map_err(|_| {
                    VmCreateFailure::new(EXIT_INVALID_INPUT, format!("invalid --cpus: {value}"))
                })?;
                if parsed == 0 {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "--cpus must be greater than zero",
                    ));
                }
                cpus = Some(parsed);
            }
            "--memory" => {
                let value = args
                    .next()
                    .ok_or_else(|| VmCreateFailure::new(EXIT_USAGE, "--memory requires a size"))?;
                memory_mib = Some(
                    parse_memory_mib(&value)
                        .map_err(|message| VmCreateFailure::new(EXIT_INVALID_INPUT, message))?,
                );
            }
            "--role" => {
                let role = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--role requires a role name")
                })?;
                if role != "router" && role != "docker" {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        format!("unsupported VM role: {role}"),
                    ));
                }
                if !roles.contains(&role) {
                    roles.push(role);
                }
            }
            "--cloud-init" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--cloud-init requires a path")
                })?;
                cloud_init = Some(PathBuf::from(value));
            }
            "--project" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--project requires a project name")
                })?;
                if value.is_empty() {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "--project must not be empty",
                    ));
                }
                project = Some(value);
            }
            "--network" => {
                requested_network = Some(args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--network requires a network name")
                })?);
            }
            "--mount" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(
                        EXIT_USAGE,
                        "--mount requires tag=…,source=…,target=…[,ro]",
                    )
                })?;
                let mount = mounts::parse_mount_flag(&value)
                    .map_err(|message| VmCreateFailure::new(EXIT_INVALID_INPUT, message))?;
                if mount_list.iter().any(|existing: &mounts::ResolvedMount| {
                    existing.name == mount.name || existing.target == mount.target
                }) {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        format!(
                            "duplicate mount name or target: {} → {}",
                            mount.name, mount.target
                        ),
                    ));
                }
                mount_list.push(mount);
            }
            "--root-password" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--root-password requires a password")
                })?;
                if value.is_empty() {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "--root-password must not be empty",
                    ));
                }
                root_password = Some(value);
            }
            "--format" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--format requires human or json")
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(VmCreateFailure::new(
                            EXIT_USAGE,
                            format!("unsupported vm create format: {value}"),
                        ))
                    }
                };
            }
            "-h" | "--help" => {
                return Err(VmCreateFailure::new(
                    EXIT_USAGE,
                    "usage: vzctl vm create <id> --from <sealed> --data-disk <GiB> \
                     [--cpus N] [--memory <SIZE>] [--network <name>] [--role router|docker] \
                     [--mount tag=…,source=…,target=…[,ro]] [--cloud-init PATH] [--project P] \
                     [--root-password <secret>] [--format human|json]",
                ))
            }
            _ if arg.starts_with('-') => {
                return Err(VmCreateFailure::new(
                    EXIT_USAGE,
                    format!("unknown vm create option: {arg}"),
                ))
            }
            _ if id.is_none() => id = Some(arg),
            _ => {
                return Err(VmCreateFailure::new(
                    EXIT_USAGE,
                    format!("unexpected vm create argument: {arg}"),
                ))
            }
        }
    }

    let id = id.ok_or_else(|| {
        VmCreateFailure::new(
            EXIT_USAGE,
            "usage: vzctl vm create <id> --from <sealed> --data-disk <GiB> \
             [--cpus N] [--memory <SIZE>] [--network <name>] [--role router|docker] \
             [--mount tag=…,source=…,target=…[,ro]] [--cloud-init PATH] [--project P] \
             [--root-password <secret>] [--format human|json]",
        )
    })?;
    if !valid_vm_id(&id) {
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            "vm id must be a flat label (1-63) or project/vm (segments alphanumeric/._-, total ≤127)",
        ));
    }
    let from =
        from.ok_or_else(|| VmCreateFailure::new(EXIT_USAGE, "vm create requires --from <sealed>"))?;
    let data_disk_gib = data_disk_gib
        .ok_or_else(|| VmCreateFailure::new(EXIT_USAGE, "vm create requires --data-disk <GiB>"))?;
    if roles.iter().any(|role| role == "docker") && project.is_none() {
        project = Some("default".to_string());
    }
    let id = resolve_create_vm_id(&id, project.as_deref())
        .map_err(|message| VmCreateFailure::new(EXIT_INVALID_INPUT, message))?;

    Ok(VmCreateOptions {
        id,
        from,
        data_disk_gib,
        cpus: cpus.unwrap_or(DEFAULT_VM_CPUS),
        memory_mib: memory_mib.unwrap_or(DEFAULT_VM_MEMORY_MIB),
        roles,
        requested_network,
        network: None,
        networks: Vec::new(),
        root_password,
        cloud_init,
        project,
        mounts: mount_list,
        format,
    })
}

pub(crate) fn parse_memory_mib(value: &str) -> Result<u64, String> {
    if value.is_empty() {
        return Err("invalid --memory: empty".to_string());
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        let mib = value
            .parse::<u64>()
            .map_err(|_| format!("invalid --memory: {value}"))?;
        if mib == 0 {
            return Err("--memory must be greater than zero".to_string());
        }
        return Ok(mib);
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| format!("invalid --memory: {value}"))?;
    let number = value[..split]
        .parse::<u64>()
        .map_err(|_| format!("invalid --memory: {value}"))?;
    if number == 0 {
        return Err("--memory must be greater than zero".to_string());
    }
    let unit = value[split..].to_ascii_lowercase();
    match unit.as_str() {
        "m" | "mb" | "mi" | "mib" => Ok(number),
        "g" | "gb" | "gi" | "gib" => number
            .checked_mul(1024)
            .ok_or_else(|| "--memory is too large".to_string()),
        "t" | "tb" | "ti" | "tib" => number
            .checked_mul(1024 * 1024)
            .ok_or_else(|| "--memory is too large".to_string()),
        _ => Err(format!(
            "--memory must use MiB/GiB/TiB or a bare MiB integer: {value}"
        )),
    }
}

fn valid_vm_id_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 63
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && segment.as_bytes()[0].is_ascii_alphanumeric()
}

pub(crate) fn valid_vm_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 127 {
        return false;
    }
    match id.split_once('/') {
        None => valid_vm_id_segment(id),
        Some((project, vm)) => {
            !project.contains('/')
                && !vm.contains('/')
                && valid_vm_id_segment(project)
                && valid_vm_id_segment(vm)
        }
    }
}

pub(crate) fn runtime_vm_id(project: &str, name: &str) -> String {
    format!("{project}/{name}")
}

pub(crate) fn vm_id_basename(id: &str) -> &str {
    id.rsplit_once('/').map(|(_, name)| name).unwrap_or(id)
}

pub(crate) fn resolve_create_vm_id(id: &str, project: Option<&str>) -> Result<String, String> {
    let Some(project) = project.filter(|value| !value.is_empty()) else {
        return Ok(id.to_string());
    };
    if !valid_vm_id_segment(project) {
        return Err(format!("invalid --project name: {project}"));
    }
    match id.split_once('/') {
        None => {
            let namespaced = runtime_vm_id(project, id);
            if !valid_vm_id(&namespaced) {
                return Err(format!("namespaced vm id is invalid: {namespaced}"));
            }
            Ok(namespaced)
        }
        Some((prefix, _)) => {
            if prefix == project {
                Ok(id.to_string())
            } else {
                Err(format!(
                    "vm id {id} does not match --project {project} (expected {project}/…)"
                ))
            }
        }
    }
}

fn create_vm_bundle(
    options: &VmCreateOptions,
    backend: &dyn VmDiskBackend,
) -> Result<VmCreateResult, VmCreateFailure> {
    create_vm_bundle_in_dirs(options, backend, &images_dir(), &state_dir().join("vms"))
}

fn create_vm_bundle_in_dirs(
    options: &VmCreateOptions,
    backend: &dyn VmDiskBackend,
    images_directory: &Path,
    vms_directory: &Path,
) -> Result<VmCreateResult, VmCreateFailure> {
    let source_path =
        resolve_image_input(&options.from, images_directory).map_err(vm_failure_from_seal)?;
    let source_path = fs::canonicalize(&source_path).map_err(|error| {
        VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!("cannot resolve image {}: {error}", source_path.display()),
        )
    })?;
    let marker_path = seal_marker_path(images_directory, &source_path);
    if !marker_path.is_file() {
        return Err(VmCreateFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "sealed marker missing for {}; run `vzctl image seal` first",
                source_path.display()
            ),
        ));
    }
    let name = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image")
        .to_string();
    let source =
        read_existing_seal(&marker_path, &source_path, name).map_err(vm_failure_from_seal)?;
    if source.image_format != "raw" {
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "vm create requires a sealed raw image; {} is {}",
                source.source_path.display(),
                source.image_format
            ),
        ));
    }

    let size_bytes = options
        .data_disk_gib
        .checked_mul(1024 * 1024 * 1024)
        .ok_or_else(|| {
            VmCreateFailure::new(EXIT_INVALID_INPUT, "data-disk size exceeds u64 bytes")
        })?;
    fs::create_dir_all(vms_directory).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot create VM directory {}: {error}",
                vms_directory.display()
            ),
        )
    })?;
    let bundle_path = vms_directory.join(&options.id);
    if bundle_path.exists() {
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!("VM bundle already exists: {}", bundle_path.display()),
        ));
    }
    fs::create_dir_all(&bundle_path).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!("cannot create VM bundle {}: {error}", bundle_path.display()),
        )
    })?;

    let root_disk_path = bundle_path.join("disk.raw");
    let data_disk_path = bundle_path.join("dataDisk.raw");
    let cidata_path = bundle_path.join("cidata.iso");
    let agent_token_path = bundle_path.join("agent.token");
    let result = prepare_vm_disks(
        options,
        backend,
        source,
        bundle_path,
        root_disk_path,
        data_disk_path,
        cidata_path,
        agent_token_path,
        size_bytes,
    );
    if result.is_err() {
        let _ = fs::remove_dir_all(vms_directory.join(&options.id));
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn prepare_vm_disks(
    options: &VmCreateOptions,
    backend: &dyn VmDiskBackend,
    source: ImageSealResult,
    bundle_path: PathBuf,
    root_disk_path: PathBuf,
    data_disk_path: PathBuf,
    cidata_path: PathBuf,
    agent_token_path: PathBuf,
    size_bytes: u64,
) -> Result<VmCreateResult, VmCreateFailure> {
    let filesystem = backend
        .filesystem_type(&bundle_path)
        .unwrap_or_else(|| "unknown".to_string());
    let clone_mode = if filesystem.eq_ignore_ascii_case("apfs") {
        backend
            .clone_linked(&source.source_path, &root_disk_path)
            .map_err(|error| {
                VmCreateFailure::new(
                    EXIT_VM_DISK_PREP_FAILED,
                    format!(
                        "clonefile failed for {} -> {}: {error}",
                        source.source_path.display(),
                        root_disk_path.display()
                    ),
                )
            })?;
        CloneMode::Linked
    } else {
        backend
            .copy_full(&source.source_path, &root_disk_path)
            .map_err(|error| {
                VmCreateFailure::new(
                    EXIT_VM_DISK_PREP_FAILED,
                    format!(
                        "full-copy fallback failed for {} -> {}: {error}",
                        source.source_path.display(),
                        root_disk_path.display()
                    ),
                )
            })?;
        CloneMode::Full
    };
    if options.roles.iter().any(|role| role == "docker") {
        // Sealed Ubuntu cloud roots are ~3.5G; docker.io needs ~300MiB free on /.
        ensure_root_disk_min_bytes(&root_disk_path, 8 * 1024 * 1024 * 1024)?;
    }
    backend
        .create_sparse(&data_disk_path, size_bytes)
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!(
                    "cannot create sparse data disk {}: {error}",
                    data_disk_path.display()
                ),
            )
        })?;

    let all_networks = if options.networks.is_empty() {
        options.network.iter().cloned().collect::<Vec<_>>()
    } else {
        options.networks.clone()
    };
    let nic_networks = all_networks
        .iter()
        .filter(|network| !network.is_docker_backend())
        .cloned()
        .collect::<Vec<_>>();
    let docker_bip = all_networks
        .iter()
        .find(|network| network.is_docker_backend())
        .map(|network| format!("{}/{}", network.ip, network.prefix));
    let nic_count = nic_networks.len().max(1);
    let identity = new_vm_identity(&options.id, nic_count)?;
    prepare_cloud_init_seed(
        backend,
        &bundle_path,
        &cidata_path,
        &agent_token_path,
        &identity,
        &options.roles,
        &nic_networks,
        docker_bip.as_deref(),
        options.root_password.as_deref(),
        options.cloud_init.as_deref(),
        options.project.as_deref(),
    )?;

    // Identity / manifest NICs must align with helper virtio NICs (vmnet only).
    // Docker-backend attachments are logical (bip) and must not occupy index 0
    // when sorted ahead of the real NIC (e.g. containers before lan).
    let result = VmCreateResult {
        id: options.id.clone(),
        bundle_path,
        source,
        root_disk_path,
        data_disk_path,
        cidata_path,
        agent_token_path,
        data_disk_gib: options.data_disk_gib,
        cpus: options.cpus,
        memory_mib: options.memory_mib,
        roles: options.roles.clone(),
        mounts: options.mounts.clone(),
        clone_mode,
        filesystem,
        identity,
        network: nic_networks.first().cloned(),
        networks: nic_networks,
    };
    write_vm_manifest(&result)?;
    Ok(result)
}

fn vm_failure_from_seal(failure: SealFailure) -> VmCreateFailure {
    VmCreateFailure::new(failure.code, failure.message)
}

fn write_vm_manifest(result: &VmCreateResult) -> Result<(), VmCreateFailure> {
    let manifest_path = result.bundle_path.join("vm.json");
    let manifest = json!({
        "apiVersion": "vzctl.dev/vm-bundle/v1",
        "managed-by": "vzctl",
        "vm_id": result.id,
        "roles": result.roles,
        "mounts": result.mounts.iter().map(mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "resources": {
            "cpus": result.cpus,
            "memory_mib": result.memory_mib,
        },
        "base": {
            "path": result.source.source_path,
            "marker": result.source.marker_path,
            "read_only": true,
        },
        "disks": {
            "root": {
                "path": result.root_disk_path,
                "clone": result.clone_mode.as_str(),
            },
            "data": {
                "path": result.data_disk_path,
                "size_gib": result.data_disk_gib,
                "sparse": true,
            },
        },
        "identity": {
            "instance_id": result.identity.instance_id,
            "hostname": result.identity.hostname,
            "fqdn": result.identity.fqdn,
            "nics": result.identity.mac_addresses.iter().enumerate().map(|(index, mac)| {
                let network = result.networks.get(index).or(result.network.as_ref());
                json!({
                    "index": index,
                    "mac": mac,
                    "address": network.map(|value| value.ip.as_str()).unwrap_or("dhcp"),
                    "network": network.map(|value| value.network.as_str()),
                    "cidr": network.map(|value| value.cidr.as_str()),
                })
            }).collect::<Vec<_>>(),
        },
        "cloud_init": {
            "seed": result.cidata_path,
            "agent_token": result.agent_token_path,
        },
    });
    fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).expect("VM manifest serializes")
        ),
    )
    .map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot write VM manifest {}: {error}",
                manifest_path.display()
            ),
        )
    })
}

fn print_vm_create_human(result: &VmCreateResult) {
    println!("created VM bundle {}", result.id);
    println!("  bundle: {}", result.bundle_path.display());
    println!(
        "  resources: {} cpus, {} MiB memory",
        result.cpus, result.memory_mib
    );
    println!(
        "  root: {} (clone: {})",
        result.root_disk_path.display(),
        result.clone_mode.as_str()
    );
    println!(
        "  data: {} ({} GiB sparse)",
        result.data_disk_path.display(),
        result.data_disk_gib
    );
    println!(
        "  base: {} (read-only)",
        result.source.source_path.display()
    );
    println!("  hostname: {}", result.identity.fqdn);
    println!("  mac: {}", result.identity.mac_addresses.join(", "));
    if !result.roles.is_empty() {
        println!("  roles: {}", result.roles.join(", "));
    }
    for mount in &result.mounts {
        println!(
            "  mount: {} → {} ({})",
            mount.source.display(),
            mount.target,
            if mount.read_only { "ro" } else { "rw" }
        );
    }
    if let Some(network) = &result.network {
        println!(
            "  network: {} ({}/{}, gateway {}){}",
            network.network,
            network.ip,
            network.prefix,
            network.gateway,
            if network.automatic { " [default]" } else { "" }
        );
    }
    println!("  cloud-init: {}", result.cidata_path.display());
}

fn vm_create_json(result: &VmCreateResult) -> Value {
    let warning = (result.clone_mode == CloneMode::Full).then(|| {
        format!(
            "filesystem {} is not APFS; root disk is a full copy",
            result.filesystem
        )
    });
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "vm.create",
        "status": if warning.is_some() { "warn" } else { "ok" },
        "exit_code": 0,
        "summary": {
            "message": "VM bundle created",
            "vm_id": result.id,
            "clone": result.clone_mode.as_str(),
            "warnings": warning.iter().count(),
        },
        "vm": {
            "id": result.id,
            "bundle": result.bundle_path,
            "managed-by": "vzctl",
            "roles": result.roles,
            "mounts": result.mounts.iter().map(mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
            "resources": {
                "cpus": result.cpus,
                "memory_mib": result.memory_mib,
            },
        },
        "network": result.network.as_ref().map(|network| {
            json!({
                "name": network.network,
                "cidr": network.cidr,
                "ip": network.ip,
                "prefix": network.prefix,
                "gateway": network.gateway,
                "dns": network.dns,
                "automatic": network.automatic,
            })
        }),
        "image": {
            "name": result.source.name,
            "path": result.source.source_path,
            "marker": result.source.marker_path,
            "read_only": true,
        },
        "disks": {
            "root": {
                "path": result.root_disk_path,
                "clone": result.clone_mode.as_str(),
                "read_only": false,
            },
            "data": {
                "path": result.data_disk_path,
                "size_gib": result.data_disk_gib,
                "sparse": true,
                "read_only": false,
            },
        },
        "identity": {
            "instance_id": result.identity.instance_id,
            "hostname": result.identity.hostname,
            "fqdn": result.identity.fqdn,
            "nics": result.identity.mac_addresses.iter().enumerate().map(|(index, mac)| {
                let network = result.networks.get(index).or(result.network.as_ref());
                json!({
                    "index": index,
                    "mac": mac,
                    "address": network.map(|value| value.ip.as_str()).unwrap_or("dhcp"),
                    "network": network.map(|value| value.network.as_str()),
                })
            }).collect::<Vec<_>>(),
        },
        "cloud_init": {
            "seed": result.cidata_path,
            "agent_token": result.agent_token_path,
        },
        "warnings": warning.into_iter().collect::<Vec<_>>(),
    })
}

fn new_vm_identity(vm_id: &str, nic_count: usize) -> Result<VmIdentity, VmCreateFailure> {
    let mut random = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut random))
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot generate VM identity: {error}"),
            )
        })?;

    random[6] = (random[6] & 0x0f) | 0x40;
    random[8] = (random[8] & 0x3f) | 0x80;
    let instance_id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        random[0], random[1], random[2], random[3],
        random[4], random[5], random[6], random[7],
        random[8], random[9], random[10], random[11],
        random[12], random[13], random[14], random[15],
    );
    let count = nic_count.max(1);
    let mut mac_addresses = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 17 + (index % 3) * 5;
        // Refresh entropy for additional NICs beyond the first buffer window.
        let mut bytes = [
            random[offset % 32],
            random[(offset + 1) % 32],
            random[(offset + 2) % 32],
            random[(offset + 3) % 32],
            random[(offset + 4) % 32],
        ];
        if index > 0 {
            File::open("/dev/urandom")
                .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
                .map_err(|error| {
                    VmCreateFailure::new(
                        EXIT_VM_DISK_PREP_FAILED,
                        format!("cannot generate NIC MAC: {error}"),
                    )
                })?;
        }
        mac_addresses.push(format!(
            "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]
        ));
    }
    let fqdn = cloud_init_fqdn(vm_id);
    let hostname = fqdn.split('.').next().unwrap_or("vm").to_string();
    Ok(VmIdentity {
        instance_id,
        hostname,
        fqdn,
        mac_addresses,
    })
}

fn cloud_init_fqdn(vm_id: &str) -> String {
    let normalized = vm_id_basename(vm_id)
        .to_ascii_lowercase()
        .replace('_', "-")
        .split('.')
        .filter(|label| !label.is_empty())
        .map(|label| label.trim_matches('-'))
        .filter(|label| !label.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    if normalized.is_empty() {
        "vm".to_string()
    } else {
        normalized
    }
}

fn prepare_cloud_init_seed(
    backend: &dyn VmDiskBackend,
    bundle_path: &Path,
    cidata_path: &Path,
    agent_token_path: &Path,
    identity: &VmIdentity,
    roles: &[String],
    networks: &[network::VmNetworkSelection],
    docker_bip: Option<&str>,
    root_password: Option<&str>,
    cloud_init: Option<&Path>,
    project: Option<&str>,
) -> Result<(), VmCreateFailure> {
    let mut token_bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut token_bytes))
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot generate guest-agent token: {error}"),
            )
        })?;
    let token = base64url_unpadded(&token_bytes);
    write_private_file(agent_token_path, format!("{token}\n").as_bytes())?;

    let seed_directory = bundle_path.join(".cidata.seed");
    fs::create_dir(&seed_directory).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot create cloud-init staging directory {}: {error}",
                seed_directory.display()
            ),
        )
    })?;
    fs::set_permissions(&seed_directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot protect cloud-init staging directory {}: {error}",
                seed_directory.display()
            ),
        )
    })?;

    let meta_data = format!(
        "instance-id: {}\nlocal-hostname: {}\n",
        identity.instance_id, identity.hostname
    );
    let network_config = render_cloud_init_network_config(identity, networks);

    let mut system = serde_yaml::Mapping::new();
    system.insert(
        serde_yaml::Value::String("preserve_hostname".into()),
        serde_yaml::Value::Bool(false),
    );
    system.insert(
        serde_yaml::Value::String("hostname".into()),
        serde_yaml::Value::String(identity.hostname.clone()),
    );
    system.insert(
        serde_yaml::Value::String("fqdn".into()),
        serde_yaml::Value::String(identity.fqdn.clone()),
    );
    system.insert(
        serde_yaml::Value::String("prefer_fqdn_over_hostname".into()),
        serde_yaml::Value::Bool(true),
    );
    system.insert(
        serde_yaml::Value::String("manage_etc_hosts".into()),
        serde_yaml::Value::Bool(true),
    );
    system.insert(
        serde_yaml::Value::String("ssh_deletekeys".into()),
        serde_yaml::Value::Bool(true),
    );
    system.insert(
        serde_yaml::Value::String("ssh_genkeytypes".into()),
        serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("ed25519".into()),
            serde_yaml::Value::String("rsa".into()),
        ]),
    );
    if let Some(password) = root_password {
        system.insert(
            serde_yaml::Value::String("disable_root".into()),
            serde_yaml::Value::Bool(false),
        );
        system.insert(
            serde_yaml::Value::String("ssh_pwauth".into()),
            serde_yaml::Value::Bool(true),
        );
        let mut user = serde_yaml::Mapping::new();
        user.insert(
            serde_yaml::Value::String("name".into()),
            serde_yaml::Value::String("root".into()),
        );
        user.insert(
            serde_yaml::Value::String("password".into()),
            serde_yaml::Value::String(password.to_string()),
        );
        user.insert(
            serde_yaml::Value::String("type".into()),
            serde_yaml::Value::String("text".into()),
        );
        let mut chpasswd = serde_yaml::Mapping::new();
        chpasswd.insert(
            serde_yaml::Value::String("expire".into()),
            serde_yaml::Value::Bool(false),
        );
        chpasswd.insert(
            serde_yaml::Value::String("users".into()),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(user)]),
        );
        system.insert(
            serde_yaml::Value::String("chpasswd".into()),
            serde_yaml::Value::Mapping(chpasswd),
        );
    }

    let mut write_files = vec![serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/var/lib/vzctl/agent.token".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("vzctl-agent:vzctl-agent".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0600".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(format!("{token}\n")),
        );
        file
    })];
    append_virtiofs_bind_files(&mut write_files);
    append_router_apply_files(&mut write_files);
    let mut runcmd = Vec::new();
    append_agent_privilege_files(&mut write_files, &mut runcmd);
    if let Ok(ca_files) = crate::certs::nocloud_ca_write_files(&state_dir()) {
        for file in ca_files {
            let path = file["path"].as_str().unwrap_or_default();
            let content = file["content"].as_str().unwrap_or_default();
            let permissions = file["permissions"].as_str().unwrap_or("0644");
            write_files.push(serde_yaml::Value::Mapping({
                let mut map = serde_yaml::Mapping::new();
                map.insert(
                    serde_yaml::Value::String("path".into()),
                    serde_yaml::Value::String(path.into()),
                );
                map.insert(
                    serde_yaml::Value::String("permissions".into()),
                    serde_yaml::Value::String(permissions.into()),
                );
                map.insert(
                    serde_yaml::Value::String("content".into()),
                    serde_yaml::Value::String(content.into()),
                );
                map
            }));
        }
        write_files.push(serde_yaml::Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String("path".into()),
                serde_yaml::Value::String("/etc/sudoers.d/vzctl-ca".into()),
            );
            map.insert(
                serde_yaml::Value::String("permissions".into()),
                serde_yaml::Value::String("0440".into()),
            );
            map.insert(
                serde_yaml::Value::String("content".into()),
                serde_yaml::Value::String(
                    "vzctl-agent ALL=(root) NOPASSWD: /usr/sbin/update-ca-certificates\n".into(),
                ),
            );
            map
        }));
    }

    if roles.iter().any(|role| role == "router") {
        write_files.push(serde_yaml::Value::Mapping({
            let mut file = serde_yaml::Mapping::new();
            file.insert(
                serde_yaml::Value::String("path".into()),
                serde_yaml::Value::String("/etc/sysctl.d/90-vzctl-router.conf".into()),
            );
            file.insert(
                serde_yaml::Value::String("owner".into()),
                serde_yaml::Value::String("root:root".into()),
            );
            file.insert(
                serde_yaml::Value::String("permissions".into()),
                serde_yaml::Value::String("0644".into()),
            );
            file.insert(
                serde_yaml::Value::String("content".into()),
                serde_yaml::Value::String("net.ipv4.ip_forward=1\n".into()),
            );
            file
        }));
        runcmd.push(serde_yaml::Value::String("sysctl --system".into()));
        runcmd.push(serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("sh".into()),
            serde_yaml::Value::String("-c".into()),
            serde_yaml::Value::String(
                "command -v iptables >/dev/null && iptables -P FORWARD DROP || true".into(),
            ),
        ]));
    }

    if roles.iter().any(|role| role == "docker") {
        let project = project.unwrap_or("default");
        let (_, pubkey) = docker::ensure_ssh_keypair(&state_dir(), project)
            .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
        let include_engine = cloud_init.is_none();
        let docker_cfg = docker::docker_role_cloud_config(&pubkey, include_engine, docker_bip);
        let system_value = serde_yaml::Value::Mapping(system.clone());
        let mut merged = docker::merge_cloud_config(system_value, Some(docker_cfg));
        if let Some(path) = cloud_init {
            let user = docker::load_user_cloud_init(path)
                .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
            merged = docker::merge_cloud_config(merged, Some(user));
        }
        if let Some(bip) = docker_bip {
            // User cloud-init may omit daemon.json; ensure bip after merge.
            merged = docker::ensure_docker_daemon_bip(merged, bip);
        }
        if let serde_yaml::Value::Mapping(ref mut map) = merged {
            let existing_files = map
                .remove(serde_yaml::Value::String("write_files".into()))
                .and_then(|value| match value {
                    serde_yaml::Value::Sequence(items) => Some(items),
                    _ => None,
                })
                .unwrap_or_default();
            let mut combined = write_files.clone();
            combined.extend(existing_files);
            map.insert(
                serde_yaml::Value::String("write_files".into()),
                serde_yaml::Value::Sequence(combined),
            );
            let existing_runcmd = map
                .remove(serde_yaml::Value::String("runcmd".into()))
                .and_then(|value| match value {
                    serde_yaml::Value::Sequence(items) => Some(items),
                    _ => None,
                })
                .unwrap_or_default();
            let mut combined_cmd = runcmd.clone();
            combined_cmd.extend(existing_runcmd);
            if !combined_cmd.is_empty() {
                map.insert(
                    serde_yaml::Value::String("runcmd".into()),
                    serde_yaml::Value::Sequence(combined_cmd),
                );
            }
        }
        let user_data = docker::render_user_data(&merged)
            .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
        return write_cloud_init_iso(
            backend,
            &seed_directory,
            cidata_path,
            &meta_data,
            &network_config,
            &user_data,
        );
    }

    system.insert(
        serde_yaml::Value::String("write_files".into()),
        serde_yaml::Value::Sequence(write_files),
    );
    if !runcmd.is_empty() {
        system.insert(
            serde_yaml::Value::String("runcmd".into()),
            serde_yaml::Value::Sequence(runcmd),
        );
    }
    let mut merged = serde_yaml::Value::Mapping(system);
    if let Some(path) = cloud_init {
        let user = docker::load_user_cloud_init(path)
            .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
        merged = docker::merge_cloud_config(merged, Some(user));
    }
    let user_data = docker::render_user_data(&merged)
        .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
    write_cloud_init_iso(
        backend,
        &seed_directory,
        cidata_path,
        &meta_data,
        &network_config,
        &user_data,
    )
}

fn write_cloud_init_iso(
    backend: &dyn VmDiskBackend,
    seed_directory: &Path,
    cidata_path: &Path,
    meta_data: &str,
    network_config: &str,
    user_data: &str,
) -> Result<(), VmCreateFailure> {
    let seed_result = (|| {
        write_private_file(&seed_directory.join("meta-data"), meta_data.as_bytes())?;
        write_private_file(
            &seed_directory.join("network-config"),
            network_config.as_bytes(),
        )?;
        write_private_file(&seed_directory.join("user-data"), user_data.as_bytes())?;
        backend
            .create_cloud_init_iso(seed_directory, cidata_path)
            .map_err(|error| {
                VmCreateFailure::new(
                    EXIT_VM_DISK_PREP_FAILED,
                    format!(
                        "cannot create NoCloud seed {}: {error}",
                        cidata_path.display()
                    ),
                )
            })?;
        fs::set_permissions(cidata_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!(
                    "cannot protect NoCloud seed {}: {error}",
                    cidata_path.display()
                ),
            )
        })
    })();
    let cleanup_result = fs::remove_dir_all(seed_directory);
    seed_result?;
    cleanup_result.map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot remove cloud-init staging directory {}: {error}",
                seed_directory.display()
            ),
        )
    })?;
    Ok(())
}

fn append_virtiofs_bind_files(write_files: &mut Vec<serde_yaml::Value>) {
    let script = include_str!("../../../guest-agent/scripts/virtiofs-bind");
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/usr/local/lib/vzctl/virtiofs-bind".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0755".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(script.to_string()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-virtiofs".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(
                "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/virtiofs-bind\n".into(),
            ),
        );
        file
    }));
}

fn append_router_apply_files(write_files: &mut Vec<serde_yaml::Value>) {
    let script = include_str!("../../../guest-agent/scripts/router-apply");
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/usr/local/lib/vzctl/router-apply".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0755".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(script.to_string()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-router".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(
                "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/router-apply\n".into(),
            ),
        );
        file
    }));
}

/// Passwordless sudo for `vzctl-agent` plus a unit refresh so clones pick up
/// NoNewPrivileges=no without rebaking the sealed base.
///
/// systemd loads `vzctl-agent.service` before cloud-config `write_files` runs,
/// so the first start can still inherit the sealed unit's `no_new_privs`.
/// `daemon-reload` + `restart` in runcmd (cloud-final) restarts under the
/// rewritten unit; later boots already read the on-disk unit.
fn append_agent_privilege_files(
    write_files: &mut Vec<serde_yaml::Value>,
    runcmd: &mut Vec<serde_yaml::Value>,
) {
    let unit = include_str!("../../../guest-agent/systemd/vzctl-agent.service");
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-agent".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String("vzctl-agent ALL=(ALL) NOPASSWD:ALL\n".into()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/systemd/system/vzctl-agent.service".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0644".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(unit.to_string()),
        );
        file
    }));
    runcmd.push(serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::String("systemctl".into()),
        serde_yaml::Value::String("daemon-reload".into()),
    ]));
    runcmd.push(serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::String("systemctl".into()),
        serde_yaml::Value::String("restart".into()),
        serde_yaml::Value::String("vzctl-agent.service".into()),
    ]));
}

#[cfg(test)]
fn cloud_init_root_password_snippet(password: &str) -> String {
    let quoted = serde_yaml::to_string(password)
        .unwrap_or_else(|_| format!("\"{password}\""))
        .trim()
        .to_string();
    format!(
        "disable_root: false\nssh_pwauth: true\nchpasswd:\n  expire: false\n  users:\n    - name: root\n      password: {quoted}\n      type: text\n"
    )
}

fn render_cloud_init_network_config(
    identity: &VmIdentity,
    networks: &[network::VmNetworkSelection],
) -> String {
    if networks.is_empty() {
        return format!(
            "version: 2\nethernets:\n  nic0:\n    match:\n      macaddress: \"{}\"\n    set-name: enp0s1\n    dhcp4: true\n    dhcp6: false\n",
            identity.mac_addresses[0]
        );
    }
    let mut body = String::from("version: 2\nethernets:\n");
    for (index, network) in networks.iter().enumerate() {
        let mac = identity
            .mac_addresses
            .get(index)
            .unwrap_or(&identity.mac_addresses[0]);
        let iface = format!("enp0s{}", index + 1);
        let search = network
            .project
            .as_ref()
            .map_or_else(String::new, |project| {
                format!("      search:\n        - {project}.vz.test\n")
            });
        // Only the primary NIC gets a default route; extra NICs stay link-local.
        let routes = if index == 0 {
            format!(
                "    routes:\n      - to: default\n        via: {}\n        on-link: true\n",
                network.gateway
            )
        } else {
            String::new()
        };
        let nameservers = if index == 0 {
            format!(
                "    nameservers:\n      addresses:\n        - {}\n{}",
                network.dns, search
            )
        } else {
            String::new()
        };
        body.push_str(&format!(
            "  nic{index}:\n    match:\n      macaddress: \"{mac}\"\n    set-name: {iface}\n    dhcp4: false\n    dhcp6: false\n    addresses:\n      - {}/{}\n{routes}{nameservers}",
            network.ip, network.prefix
        ));
    }
    body
}

fn ensure_root_disk_min_bytes(path: &Path, min_bytes: u64) -> Result<(), VmCreateFailure> {
    let metadata = fs::metadata(path).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!("cannot stat root disk {}: {error}", path.display()),
        )
    })?;
    if metadata.len() >= min_bytes {
        return Ok(());
    }
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(min_bytes))
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!(
                    "cannot grow root disk {} to {min_bytes} bytes: {error}",
                    path.display()
                ),
            )
        })
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), VmCreateFailure> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot create private file {}: {error}", path.display()),
            )
        })?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot write private file {}: {error}", path.display()),
            )
        })
}

fn base64url_unpadded(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        output.push(ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(value & 0x3f) as usize] as char);
        }
    }
    output
}

fn emit_vm_create_failure(format: OutputFormat, failure: &VmCreateFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "vm.create",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

impl ImageSealBackend for LibguestfsBackend {
    fn inspect_format(&self, path: &Path) -> Result<String, SealFailure> {
        inspect_image_format(path)
    }

    fn verify_preserved(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        run_virt_customize(
            path,
            image_format,
            IMAGE_PRESERVATION_CHECKS,
            EXIT_IMAGE_INVARIANT_FAILED,
            "required guest-agent files are missing",
        )
    }

    fn customize(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        run_virt_customize(
            path,
            image_format,
            IMAGE_CLEANUP_COMMANDS,
            EXIT_IMAGE_CUSTOMIZE_FAILED,
            "image cleanup failed",
        )
    }

    fn verify_clone_safe(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        run_virt_customize(
            path,
            image_format,
            IMAGE_CLONE_SAFE_CHECKS,
            EXIT_IMAGE_INVARIANT_FAILED,
            "clone-safe cleanup verification failed",
        )
    }
}

impl ImageSealBackend for BuilderVmBackend {
    fn inspect_format(&self, path: &Path) -> Result<String, SealFailure> {
        if builder::qemu_img_available() {
            return inspect_image_format(path);
        }
        // Raw-only heuristic when qemu-img is missing: reject qcow magic.
        let mut header = [0_u8; 4];
        let mut file = File::open(path).map_err(|error| {
            SealFailure::new(
                EXIT_INVALID_INPUT,
                format!("cannot open {}: {error}", path.display()),
            )
        })?;
        use std::io::Read;
        let _ = file.read(&mut header);
        if &header == b"QFI\xfb" {
            return Err(SealFailure::new(
                EXIT_INVALID_INPUT,
                "builder backend supports raw images only; convert qcow2 on the host or use a sealed alias",
            ));
        }
        Ok("raw".to_string())
    }

    fn verify_preserved(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        Ok(())
    }

    fn customize(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        Ok(())
    }

    fn verify_clone_safe(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        Ok(())
    }

    fn seal_pipeline(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        if image_format != "raw" {
            return Err(SealFailure::new(
                EXIT_INVALID_INPUT,
                format!(
                    "builder backend supports raw images only (got {image_format}); \
                     pull aliases are normalized to raw"
                ),
            ));
        }
        if self.progress {
            eprintln!("Resolving builder appliance…");
        }
        let appliance = builder::resolve_builder_image(&self.images_dir)
            .map_err(|failure| SealFailure::new(failure.code, failure.message))?;
        let runbook = builder::seal_runbook();
        if self.progress {
            eprintln!("Sealing via builder VM (one boot)…");
        }
        builder::run_builder_vm(builder::BuilderRunOptions {
            appliance: &appliance,
            target_raw: path,
            runbook: &runbook,
            staging_dir: None,
            timeout: builder::default_timeout(),
            progress: self.progress,
        })
        .map(|_| ())
        .map_err(|failure| SealFailure::new(failure.code, failure.message))
    }
}

fn inspect_image_format(path: &Path) -> Result<String, SealFailure> {
    let output = Command::new("qemu-img")
        .args(["info", "--output=json"])
        .arg(path)
        .output()
        .map_err(|error| {
            SealFailure::new(
                EXIT_UNAVAILABLE,
                format!("qemu-img is required to inspect images: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "qemu-img cannot inspect {}: {}",
                path.display(),
                command_text(&output)
            ),
        ));
    }
    let info: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "qemu-img returned invalid JSON for {}: {error}",
                path.display()
            ),
        )
    })?;
    info["format"].as_str().map(str::to_string).ok_or_else(|| {
        SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("qemu-img did not report a format for {}", path.display()),
        )
    })
}

impl VmDiskBackend for NativeVmDiskBackend {
    fn filesystem_type(&self, path: &Path) -> Option<String> {
        filesystem_type(path)
    }

    fn clone_linked(&self, source: &Path, destination: &Path) -> Result<(), io::Error> {
        clonefile_path(source, destination)?;
        let mut permissions = fs::metadata(destination)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(destination, permissions)
    }

    fn copy_full(&self, source: &Path, destination: &Path) -> Result<(), io::Error> {
        let mut source_file = File::open(source)?;
        let mut destination_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination)?;
        io::copy(&mut source_file, &mut destination_file)?;
        destination_file.sync_all()
    }

    fn create_sparse(&self, path: &Path, size_bytes: u64) -> Result<(), io::Error> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.set_len(size_bytes)?;
        file.sync_all()
    }

    fn create_cloud_init_iso(
        &self,
        seed_directory: &Path,
        destination: &Path,
    ) -> Result<(), io::Error> {
        let output = Command::new("hdiutil")
            .args([
                "makehybrid",
                "-iso",
                "-joliet",
                "-default-volume-name",
                "cidata",
            ])
            .arg("-o")
            .arg(destination)
            .arg(seed_directory)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(command_text(&output)))
        }
    }
}

#[cfg(target_os = "macos")]
fn clonefile_path(source: &Path, destination: &Path) -> Result<(), io::Error> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn clonefile(source: *const c_char, destination: *const c_char, flags: c_int) -> c_int;
    }

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    if unsafe { clonefile(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn clonefile_path(_source: &Path, _destination: &Path) -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "clonefile is only available on macOS",
    ))
}

fn run_virt_customize(
    path: &Path,
    image_format: &str,
    guest_commands: &[&str],
    failure_code: u8,
    failure_context: &str,
) -> Result<(), SealFailure> {
    let mut command = Command::new("virt-customize");
    command
        .arg("--format")
        .arg(image_format)
        .arg("-a")
        .arg(path);
    for guest_command in guest_commands {
        command.arg("--run-command").arg(guest_command);
    }
    let output = command.output().map_err(|error| {
        SealFailure::new(
            EXIT_UNAVAILABLE,
            format!("virt-customize is required on the Linux image builder: {error}"),
        )
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SealFailure::new(
            failure_code,
            format!(
                "{failure_context} for {}: {}",
                path.display(),
                command_text(&output)
            ),
        ))
    }
}

fn events_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        Some("subscribe") => match parse_events_options(args) {
            Ok(options) => events_subscribe(options),
            Err((message, code)) => {
                eprintln!("{message}");
                ExitCode::from(code)
            }
        },
        Some(command) => {
            eprintln!("unknown events command: {command}");
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            eprintln!("usage: vzctl events subscribe [--filter 'vm.*,apply.*']");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn parse_events_options(args: impl Iterator<Item = String>) -> Result<EventsOptions, (String, u8)> {
    let mut filter = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--filter" => {
                let value = args
                    .next()
                    .ok_or_else(|| ("--filter requires a pattern".to_string(), EXIT_USAGE))?;
                if !valid_event_filter(&value) {
                    return Err((format!("invalid event filter: {value}"), EXIT_INVALID_INPUT));
                }
                filter = Some(value);
            }
            _ => {
                return Err((
                    format!("unknown events subscribe option: {arg}"),
                    EXIT_USAGE,
                ))
            }
        }
    }
    Ok(EventsOptions { filter })
}

fn valid_event_filter(expression: &str) -> bool {
    let patterns = expression.split(',').map(str::trim).collect::<Vec<_>>();
    !patterns.is_empty()
        && patterns.iter().all(|pattern| {
            !pattern.is_empty()
                && pattern.matches('*').count() <= 1
                && (!pattern.contains('*') || pattern.ends_with('*'))
        })
}

fn events_subscribe(options: EventsOptions) -> ExitCode {
    let path = supervisor_socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("supervisor socket {}: {error}", path.display());
            return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
        }
    };
    let timeout = Some(Duration::from_secs(2));
    if let Err(error) = stream.set_write_timeout(timeout) {
        eprintln!("supervisor write timeout setup: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    let request = json!({
        "jsonrpc": "2.0",
        "method": "events.subscribe",
        "params": {
            "filter": options.filter,
        },
        "id": 1
    });
    if let Err(error) = writeln!(stream, "{request}") {
        eprintln!("subscribe request: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    if let Err(error) = reader.read_line(&mut response) {
        eprintln!("subscribe response: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }
    let response: Value = match serde_json::from_str(&response) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("invalid subscribe response: {error}");
            return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
        }
    };
    if response["result"]["ok"] != true || response["result"]["v"] != 1 {
        eprintln!("subscribe rejected: {response}");
        return ExitCode::from(EXIT_INVALID_INPUT);
    }

    if let Err(error) = reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(250)))
    {
        eprintln!("supervisor read timeout setup: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    if let Err(error) = ctrlc::set_handler(move || {
        signal_flag.store(true, Ordering::SeqCst);
    }) {
        eprintln!("cannot install Ctrl-C handler: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    let stdout = io::stdout();
    let mut output = stdout.lock();
    loop {
        if interrupted.load(Ordering::SeqCst) {
            return ExitCode::SUCCESS;
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) if interrupted.load(Ordering::SeqCst) => return ExitCode::SUCCESS,
            Ok(0) => {
                eprintln!("supervisor closed the event stream");
                return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
            }
            Ok(_) => {
                let event: Value = match serde_json::from_str(&line) {
                    Ok(event) => event,
                    Err(error) => {
                        eprintln!("invalid event JSON: {error}");
                        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                    }
                };
                if event["v"] != 1
                    || !event["ts"].is_string()
                    || !event["type"].is_string()
                    || !event["data"].is_object()
                {
                    eprintln!("invalid event envelope");
                    return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                }
                if let Err(error) = output
                    .write_all(line.as_bytes())
                    .and_then(|_| output.flush())
                {
                    eprintln!("event output: {error}");
                    return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                eprintln!("event stream: {error}");
                return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
            }
        }
    }
}

fn parse_doctor_options(args: impl Iterator<Item = String>) -> Result<DoctorOptions, String> {
    let mut format = OutputFormat::Human;
    let mut min_free_gib = std::env::var("VZCTL_DOCTOR_MIN_FREE_GIB")
        .ok()
        .map(|value| parse_min_free_gib(&value))
        .transpose()?
        .unwrap_or(DEFAULT_MIN_FREE_GIB);
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
                    _ => return Err(format!("unsupported doctor format: {value}")),
                };
            }
            "--min-free-gib" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--min-free-gib requires an integer".to_string())?;
                min_free_gib = parse_min_free_gib(&value)?;
            }
            "-h" | "--help" => {
                return Err(
                    "usage: vzctl doctor [--format human|json] [--min-free-gib N]".to_string(),
                )
            }
            _ => return Err(format!("unknown doctor option: {arg}")),
        }
    }

    Ok(DoctorOptions {
        format,
        min_free_gib,
    })
}

fn parse_min_free_gib(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("invalid minimum free space: {value} GiB"))?;
    if parsed == 0 {
        return Err("minimum free space must be at least 1 GiB".to_string());
    }
    Ok(parsed)
}

fn doctor(options: DoctorOptions) -> ExitCode {
    let macos_version = macos_major();
    let state_dir = state_dir();
    let images_dir = std::env::var_os("VZCTL_IMAGES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir.join("images"));
    let dns_port = std::env::var("VZCTL_DNS_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_DNS_PORT);

    let mut checks = vec![check_macos(macos_version)];
    checks.extend(check_daemon_binaries());
    checks.push(check_apfs(&images_dir));
    checks.push(check_disk_space(&images_dir, options.min_free_gib));
    let (dns_port_check, dns_port_free) = check_dns_port(dns_port);
    checks.push(dns_port_check);
    checks.push(check_resolvers(dns_port, dns_port_free));
    checks.push(check_vmnet_hint(macos_version));
    checks.push(check_image_backend(&images_dir));
    checks.push(check_supervisor());
    let bind_helper = dns::doctor_bind_helper_check();
    checks.push(Check::new(
        bind_helper.id,
        if bind_helper.ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        bind_helper.message,
        bind_helper.details,
    ));
    let docker_check = docker::doctor_check(&state_dir);
    checks.push(Check::new(
        docker_check.id,
        if docker_check.ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        docker_check.message,
        docker_check.details,
    ));
    checks.push(check_certs_host_trust(&state_dir));

    let exit_code = if macos_version.unwrap_or(0) < 26 {
        EXIT_HOST_UNSUPPORTED
    } else if checks
        .iter()
        .any(|check| check.id == "supervisor.health" && check.status == CheckStatus::Fail)
    {
        EXIT_SUPERVISOR_UNHEALTHY
    } else {
        0
    };

    match options.format {
        OutputFormat::Human => print_human_report(&checks, exit_code),
        OutputFormat::Json => print_json_report(&checks, exit_code),
    }
    ExitCode::from(exit_code)
}

fn print_human_report(checks: &[Check], exit_code: u8) {
    println!("vzctl doctor");
    for check in checks {
        println!("  {}: {}", check.status.label(), check.message);
    }
    println!("  result: exit {exit_code}");
}

fn print_json_report(checks: &[Check], exit_code: u8) {
    println!("{}", doctor_json(checks, exit_code));
}

fn doctor_json(checks: &[Check], exit_code: u8) -> Value {
    let warning_count = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Warn)
        .count();
    let failure_count = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Fail)
        .count();
    let report = json!({
        "apiVersion": CLI_API_VERSION,
        "command": "doctor",
        "status": if failure_count > 0 { "fail" } else if warning_count > 0 { "warn" } else { "ok" },
        "exit_code": exit_code,
        "summary": {
            "ok": checks.len() - warning_count - failure_count,
            "warnings": warning_count,
            "failures": failure_count,
        },
        "checks": checks.iter().map(Check::to_json).collect::<Vec<_>>(),
    });
    report
}

fn check_macos(version: Option<u32>) -> Check {
    match version {
        Some(version) if version >= 26 => Check::new(
            "host.macos",
            CheckStatus::Ok,
            format!("macOS {version} meets the 26+ baseline"),
            json!({ "major": version, "minimum_major": 26 }),
        ),
        Some(version) => Check::new(
            "host.macos",
            CheckStatus::Fail,
            format!("macOS {version} is unsupported; macOS 26+ required (ADR 0001)"),
            json!({ "major": version, "minimum_major": 26 }),
        ),
        None => Check::new(
            "host.macos",
            CheckStatus::Fail,
            "macOS version could not be determined; macOS 26+ required (ADR 0001)",
            json!({ "major": null, "minimum_major": 26 }),
        ),
    }
}

fn check_daemon_binaries() -> Vec<Check> {
    ["vz-helper", "vz-supervisor"]
        .into_iter()
        .map(check_daemon_binary)
        .collect()
}

fn check_daemon_binary(name: &'static str) -> Check {
    let id = if name == "vz-helper" {
        "codesign.vz-helper"
    } else {
        "codesign.vz-supervisor"
    };
    let Some(path) = find_daemon_binary(name) else {
        return Check::new(
            id,
            CheckStatus::Warn,
            format!(
                "{name} not found; build it or set {}",
                daemon_path_env(name)
            ),
            json!({ "binary": name, "found": false }),
        );
    };

    let verified = Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(&path)
        .output();
    let signature = Command::new("codesign")
        .args(["-dv", "--verbose=4"])
        .arg(&path)
        .output();
    let entitlements = Command::new("codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(&path)
        .output();

    let (Ok(verified), Ok(signature), Ok(entitlements)) = (verified, signature, entitlements)
    else {
        return Check::new(
            id,
            CheckStatus::Warn,
            format!("cannot inspect {name} with codesign ({})", path.display()),
            json!({ "binary": name, "path": path, "found": true }),
        );
    };

    let signature_text = command_text(&signature);
    let entitlement_text = command_text(&entitlements);
    let signed = verified.status.success();
    let has_virtualization = has_virtualization_entitlement(&entitlement_text);
    let ad_hoc = signature_text.contains("Signature=adhoc");
    let details = json!({
        "binary": name,
        "path": path,
        "found": true,
        "signed": signed,
        "ad_hoc": ad_hoc,
        "virtualization_entitlement": has_virtualization,
    });

    if signed && has_virtualization {
        Check::new(
            id,
            CheckStatus::Ok,
            format!(
                "{name} is {}signed with com.apple.security.virtualization",
                if ad_hoc { "ad-hoc " } else { "" }
            ),
            details,
        )
    } else {
        let mut missing = Vec::new();
        if !signed {
            missing.push("valid code signature");
        }
        if !has_virtualization {
            missing.push("com.apple.security.virtualization entitlement");
        }
        Check::new(
            id,
            CheckStatus::Warn,
            format!(
                "{name} missing {} ({})",
                missing.join(" and "),
                path.display()
            ),
            details,
        )
    }
}

fn daemon_path_env(name: &str) -> &'static str {
    if name == "vz-helper" {
        "VZCTL_HELPER_PATH"
    } else {
        "VZCTL_SUPERVISOR_PATH"
    }
}

fn find_daemon_binary(name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(daemon_path_env(name)).map(PathBuf::from) {
        return path.is_file().then_some(path);
    }

    let mut candidates = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            candidates.push(parent.join(name));
        }
    }
    if let Some(directory) = std::env::var_os("VZCTL_DAEMON_DIR") {
        candidates.push(PathBuf::from(directory).join(name));
    }
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    candidates.push(repo_root.join("daemon/.build/debug").join(name));
    candidates.push(repo_root.join("daemon/.build/release").join(name));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| fs::canonicalize(&path).unwrap_or(path))
}

fn command_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn has_virtualization_entitlement(output: &str) -> bool {
    let Some(key_index) = output.find("com.apple.security.virtualization") else {
        return false;
    };
    let remainder = &output[key_index..];
    remainder.contains("<true/>")
        || remainder.contains("<true />")
        || remainder.contains("= 1")
        || remainder.contains("true")
}

fn check_apfs(path: &Path) -> Check {
    let probe_path = existing_ancestor(path);
    let Some(filesystem) = filesystem_type(&probe_path) else {
        return Check::new(
            "storage.apfs",
            CheckStatus::Warn,
            format!("cannot inspect filesystem for {}", path.display()),
            json!({ "path": path, "probe_path": probe_path }),
        );
    };
    if filesystem.eq_ignore_ascii_case("apfs") {
        Check::new(
            "storage.apfs",
            CheckStatus::Ok,
            format!("{} is on APFS (clonefile capable)", probe_path.display()),
            json!({ "path": path, "probe_path": probe_path, "filesystem": filesystem, "clonefile_capable": true }),
        )
    } else {
        Check::new(
            "storage.apfs",
            CheckStatus::Warn,
            format!(
                "{} is on {}; APFS is required for clonefile-backed images",
                probe_path.display(),
                if filesystem.is_empty() {
                    "an unknown filesystem"
                } else {
                    &filesystem
                }
            ),
            json!({ "path": path, "probe_path": probe_path, "filesystem": filesystem, "clonefile_capable": false }),
        )
    }
}

fn filesystem_type(path: &Path) -> Option<String> {
    let output = Command::new("stat")
        .args(["-f", "%T"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stat_value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stat_value.starts_with('/') {
        return (!stat_value.is_empty()).then_some(stat_value);
    }

    let mounts = Command::new("mount").output().ok()?;
    if !mounts.status.success() {
        return None;
    }
    filesystem_for_mount_point(&String::from_utf8_lossy(&mounts.stdout), &stat_value)
}

fn filesystem_for_mount_point(mounts: &str, mount_point: &str) -> Option<String> {
    let marker = format!(" on {mount_point} (");
    mounts.lines().find_map(|line| {
        let options = line.split_once(&marker)?.1;
        Some(options.split(',').next()?.trim().to_string())
    })
}

fn existing_ancestor(path: &Path) -> PathBuf {
    let mut candidate = path;
    loop {
        if candidate.exists() {
            return candidate.to_path_buf();
        }
        match candidate.parent() {
            Some(parent) => candidate = parent,
            None => return PathBuf::from("/"),
        }
    }
}

fn check_disk_space(path: &Path, minimum_gib: u64) -> Check {
    let probe_path = existing_ancestor(path);
    let output = Command::new("df").arg("-Pk").arg(&probe_path).output();
    let available_kib = output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| parse_df_available_kib(&String::from_utf8_lossy(&output.stdout)));
    let Some(available_kib) = available_kib else {
        return Check::new(
            "storage.disk_space",
            CheckStatus::Warn,
            format!("cannot determine free space for {}", probe_path.display()),
            json!({ "path": path, "probe_path": probe_path, "minimum_gib": minimum_gib }),
        );
    };

    let available_gib = available_kib as f64 / 1024.0 / 1024.0;
    let details = json!({
        "path": path,
        "probe_path": probe_path,
        "available_bytes": available_kib * 1024,
        "available_gib": (available_gib * 10.0).round() / 10.0,
        "minimum_gib": minimum_gib,
    });
    if available_kib >= minimum_gib * 1024 * 1024 {
        Check::new(
            "storage.disk_space",
            CheckStatus::Ok,
            format!("{available_gib:.1} GiB free (minimum {minimum_gib} GiB)"),
            details,
        )
    } else {
        Check::new(
            "storage.disk_space",
            CheckStatus::Warn,
            format!("only {available_gib:.1} GiB free; at least {minimum_gib} GiB recommended"),
            details,
        )
    }
}

fn parse_df_available_kib(output: &str) -> Option<u64> {
    let line = output.lines().rfind(|line| !line.trim().is_empty())?;
    line.split_whitespace().nth(3)?.parse().ok()
}

fn check_dns_port(port: u16) -> (Check, Option<bool>) {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let udp = UdpSocket::bind(address);
    let tcp = TcpListener::bind(address);
    let udp_free = udp.is_ok();
    let tcp_free = tcp.is_ok();
    let udp_error = udp.as_ref().err().map(ToString::to_string);
    let tcp_error = tcp.as_ref().err().map(ToString::to_string);
    let details = json!({
        "address": Ipv4Addr::LOCALHOST,
        "port": port,
        "udp_free": udp_free,
        "tcp_free": tcp_free,
        "udp_error": udp_error,
        "tcp_error": tcp_error,
    });
    if udp_free && tcp_free {
        (
            Check::new(
                "dns.host_port",
                CheckStatus::Ok,
                format!("127.0.0.1:{port} is free for UDP and TCP DNS"),
                details,
            ),
            Some(true),
        )
    } else if udp
        .as_ref()
        .is_err_and(|error| error.kind() != std::io::ErrorKind::AddrInUse)
        || tcp
            .as_ref()
            .is_err_and(|error| error.kind() != std::io::ErrorKind::AddrInUse)
    {
        (
            Check::new(
                "dns.host_port",
                CheckStatus::Warn,
                format!(
                    "cannot reliably probe 127.0.0.1:{port} (UDP: {}; TCP: {})",
                    udp_error.as_deref().unwrap_or("free"),
                    tcp_error.as_deref().unwrap_or("free")
                ),
                details,
            ),
            None,
        )
    } else {
        (
            Check::new(
                "dns.host_port",
                CheckStatus::Warn,
                format!(
                    "127.0.0.1:{port} is in use ({})",
                    match (udp_free, tcp_free) {
                        (false, false) => "UDP and TCP",
                        (false, true) => "UDP",
                        (true, false) => "TCP",
                        (true, true) => unreachable!(),
                    }
                ),
                details,
            ),
            Some(false),
        )
    }
}

fn check_resolvers(dns_port: u16, dns_port_free: Option<bool>) -> Check {
    let resolver_dir = std::env::var_os("VZCTL_RESOLVER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/resolver"));
    let files = resolver_files(&resolver_dir);
    if files.is_empty() {
        return Check::new(
            "dns.resolvers",
            CheckStatus::Ok,
            "no *.vz.test resolver files installed (expected before DNS setup)",
            json!({ "directory": resolver_dir, "files": [] }),
        );
    }

    let malformed = files
        .iter()
        .filter(|path| !resolver_points_to(path, dns_port))
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let file_names = files
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let details = json!({
        "directory": resolver_dir,
        "files": file_names,
        "expected_nameserver": "127.0.0.1",
        "expected_port": dns_port,
        "malformed": malformed,
        "orphaned": dns_port_free == Some(true),
    });

    if !malformed.is_empty() {
        Check::new(
            "dns.resolvers",
            CheckStatus::Warn,
            format!(
                "{} resolver file(s) do not point to 127.0.0.1:{dns_port}",
                malformed.len()
            ),
            details,
        )
    } else if dns_port_free == Some(true) {
        Check::new(
            "dns.resolvers",
            CheckStatus::Warn,
            format!(
                "{} *.vz.test resolver file(s) appear orphaned; DNS port {dns_port} is free",
                files.len()
            ),
            details,
        )
    } else if dns_port_free == Some(false) {
        Check::new(
            "dns.resolvers",
            CheckStatus::Ok,
            format!(
                "{} *.vz.test resolver file(s) point to 127.0.0.1:{dns_port}",
                files.len()
            ),
            details,
        )
    } else {
        Check::new(
            "dns.resolvers",
            CheckStatus::Warn,
            format!(
                "{} *.vz.test resolver file(s) are configured, but listener state is unknown",
                files.len()
            ),
            details,
        )
    }
}

fn resolver_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".vz.test"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn resolver_points_to(path: &Path, dns_port: u16) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let mut nameserver_ok = false;
    let mut port_ok = false;
    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        match fields.as_slice() {
            ["nameserver", "127.0.0.1"] => nameserver_ok = true,
            ["port", value] if *value == dns_port.to_string() => port_ok = true,
            _ => {}
        }
    }
    nameserver_ok && port_ok
}

fn check_vmnet_hint(macos_version: Option<u32>) -> Check {
    if macos_version.is_some_and(|version| version >= 26) {
        Check::new(
            "network.vmnet",
            CheckStatus::Ok,
            "custom vmnet API baseline is available; CRUD/rebuild is supervisor-managed",
            json!({
                "doctor_creates_network": false,
                "host_gateway_dns_suffix": ".0",
                "router_suffix": ".2",
                "guest_range": ".10+",
                "bridged_networking": "out_of_scope",
            }),
        )
    } else {
        Check::new(
            "network.vmnet",
            CheckStatus::Warn,
            "custom vmnet requires macOS 26+; live network creation is not tested",
            json!({ "live_create_tested": false }),
        )
    }
}

fn check_image_backend(images_dir: &Path) -> Check {
    let (status, message, details) = builder::doctor_builder_check(images_dir);
    let status = match status.as_str() {
        "ok" => CheckStatus::Ok,
        "fail" => CheckStatus::Fail,
        _ => CheckStatus::Warn,
    };
    Check::new("image.backend", status, message, details)
}

fn check_certs_host_trust(state_dir: &Path) -> Check {
    let details = crate::certs::host_trust_status(state_dir);
    let present = details
        .get("present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let trusted = details
        .get("trusted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !present {
        Check::new(
            "certs.host_trust",
            CheckStatus::Ok,
            "no local CA yet (optional until ingress/OIDC)",
            details,
        )
    } else if trusted {
        Check::new(
            "certs.host_trust",
            CheckStatus::Ok,
            "vzctl Local CA is trusted in the macOS Keychain",
            details,
        )
    } else {
        Check::new(
            "certs.host_trust",
            CheckStatus::Warn,
            "Local CA exists but is not trusted in the Keychain; browsers show SEC_ERROR_UNKNOWN_ISSUER — run vzctl certs ca install",
            details,
        )
    }
}

fn check_supervisor() -> Check {
    let path = supervisor_socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Check::new(
                "supervisor.health",
                CheckStatus::Warn,
                format!("supervisor not running ({})", path.display()),
                json!({ "socket": path, "running": false }),
            )
        }
        Err(error) => {
            return Check::new(
                "supervisor.health",
                CheckStatus::Fail,
                format!("supervisor socket {}: {error}", path.display()),
                json!({ "socket": path, "error": error.to_string() }),
            )
        }
    };

    let timeout = Some(Duration::from_secs(2));
    if let Err(error) = stream.set_read_timeout(timeout) {
        return supervisor_failure(&path, format!("read timeout setup: {error}"));
    }
    if let Err(error) = stream.set_write_timeout(timeout) {
        return supervisor_failure(&path, format!("write timeout setup: {error}"));
    }

    let request = json!({
        "jsonrpc": "2.0",
        "method": "daemon.health",
        "id": 1
    });
    if let Err(error) = writeln!(stream, "{request}") {
        return supervisor_failure(&path, format!("health request: {error}"));
    }

    let mut response = String::new();
    if let Err(error) = BufReader::new(stream).read_line(&mut response) {
        return supervisor_failure(&path, format!("health response: {error}"));
    }
    let value: Value = match serde_json::from_str(&response) {
        Ok(value) => value,
        Err(error) => {
            return supervisor_failure(&path, format!("invalid health JSON: {error}"));
        }
    };
    let result = &value["result"];
    if result["ok"] != true || result["db_ok"] != true {
        return supervisor_failure(&path, format!("health is not ok: {value}"));
    }
    let network_orphans = result["network_orphans"].as_u64().unwrap_or(0);
    let dns_ok = result["dns_ok"].as_bool().unwrap_or(true);
    let vz_net_ok = result["vz_net_ok"].as_bool().unwrap_or(true);
    let vz_edge_ok = result["vz_edge_ok"].as_bool().unwrap_or(true);
    let dns = &result["dns"];
    let vz_net = &result["vz_net"];
    let vz_edge = &result["vz_edge"];
    let details = json!({
        "socket": path,
        "running": true,
        "version": result["version"],
        "pid": result["pid"],
        "db_ok": true,
        "dns_ok": dns_ok,
        "dns": dns,
        "vz_net_ok": vz_net_ok,
        "vz_net": vz_net,
        "vz_edge_ok": vz_edge_ok,
        "vz_edge": vz_edge,
        "networks": result["networks"],
        "network_orphans": network_orphans,
    });
    if !vz_net_ok {
        return Check::new(
            "supervisor.health",
            CheckStatus::Warn,
            "supervisor is up, but vz-net is unavailable (net.sock); vmnet acquire will fail until com.vzctl.net is running",
            details,
        );
    }
    if !vz_edge_ok {
        return Check::new(
            "supervisor.health",
            CheckStatus::Warn,
            "supervisor is up, but vz-edge is unavailable or degraded (edge.sock); DNS, ports and ingress need com.vzctl.edge",
            details,
        );
    }
    if network_orphans > 0 {
        return Check::new(
            "supervisor.health",
            CheckStatus::Warn,
            format!(
                "supervisor is healthy, but {network_orphans} vmnet CIDR(s) could not be rebuilt; \
                 an unclean vz-net exit may have orphaned reservations until reboot"
            ),
            details,
        );
    }
    if !dns_ok {
        let last_error = dns["last_error"].as_str().unwrap_or("dns degraded");
        return Check::new(
            "supervisor.health",
            CheckStatus::Warn,
            format!(
                "supervisor is up, but DNS is degraded ({last_error}); \
                 guest :53 needs: sudo vzctl dns install-bind-helper"
            ),
            details,
        );
    }

    Check::new(
        "supervisor.health",
        CheckStatus::Ok,
        format!(
            "supervisor {} (pid {}, db ok, dns ok)",
            result["version"].as_str().unwrap_or("unknown"),
            result["pid"]
        ),
        details,
    )
}

fn supervisor_failure(path: &Path, error: String) -> Check {
    Check::new(
        "supervisor.health",
        CheckStatus::Fail,
        format!("supervisor {error}"),
        json!({ "socket": path, "error": error }),
    )
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
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn doctor_options_default_to_human_and_twenty_gib() {
        std::env::remove_var("VZCTL_DOCTOR_MIN_FREE_GIB");
        assert_eq!(
            parse_doctor_options(std::iter::empty()).unwrap(),
            DoctorOptions {
                format: OutputFormat::Human,
                min_free_gib: 20,
            }
        );
    }

    #[test]
    fn doctor_options_accept_json_and_disk_override() {
        let args = ["--format", "json", "--min-free-gib", "42"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_doctor_options(args).unwrap(),
            DoctorOptions {
                format: OutputFormat::Json,
                min_free_gib: 42,
            }
        );
    }

    #[test]
    fn version_json_matches_golden_contract() {
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/version.json")).unwrap();
        assert_eq!(version_json("0.0.1"), expected);
    }

    #[test]
    fn doctor_json_matches_golden_contract() {
        let checks = vec![
            Check::new(
                "host.macos",
                CheckStatus::Ok,
                "macOS 26 meets the baseline",
                json!({ "major": 26, "minimum_major": 26 }),
            ),
            Check::new(
                "codesign.vz-helper",
                CheckStatus::Warn,
                "vz-helper not found",
                json!({ "binary": "vz-helper", "found": false }),
            ),
        ];
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/doctor.json")).unwrap();
        assert_eq!(doctor_json(&checks, 0), expected);
    }

    #[test]
    fn stable_exit_code_mapping_matches_cli_contract() {
        assert_eq!(EXIT_USAGE, 2);
        assert_eq!(EXIT_INVALID_INPUT, 3);
        assert_eq!(EXIT_INCOMPLETE_JOURNAL, 5);
        assert_eq!(EXIT_LEASE_HELD, 6);
        assert_eq!(EXIT_SUPERVISOR_UNHEALTHY, 10);
        assert_eq!(EXIT_HOST_UNSUPPORTED, 11);
        assert_eq!(EXIT_UNAVAILABLE, 12);
        assert_eq!(EXIT_IMAGE_CUSTOMIZE_FAILED, 13);
        assert_eq!(EXIT_IMAGE_INVARIANT_FAILED, 14);
        assert_eq!(EXIT_IMAGE_STATE_FAILED, 15);
        assert_eq!(EXIT_VM_DISK_PREP_FAILED, 16);
        assert_eq!(network::EXIT_NETWORK, 17);
        assert_eq!(route::EXIT_ROUTE, 18);
        assert_eq!(dns::EXIT_RESOLVER, 19);
        assert_eq!(dns::EXIT_DNS_QUERY, 20);
        assert_eq!(image::EXIT_IMAGE_NETWORK, 21);
        assert_eq!(image::EXIT_IMAGE_CHECKSUM, 22);
        assert_eq!(image::EXIT_IMAGE_ARCH, 23);
    }

    #[test]
    fn image_pull_options_accept_alias_and_json_in_any_order() {
        let args = ["--format", "json", "ubuntu-latest"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_image_pull_options(args).unwrap(),
            ImagePullOptions {
                alias: "ubuntu-latest".to_string(),
                format: OutputFormat::Json,
            }
        );
    }

    #[test]
    fn image_pull_json_exposes_digest_alias_and_unsealed_state() {
        let result = image::PullResult {
            requested_alias: "coreos-latest".to_string(),
            canonical_alias: "fedora-coreos-latest".to_string(),
            distribution: "Fedora CoreOS".to_string(),
            release: "stable".to_string(),
            source_url: "https://example.invalid/fcos.qcow2.xz".to_string(),
            source_format: "qcow2.xz".to_string(),
            source_algorithm: "sha256".to_string(),
            source_digest: "a".repeat(64),
            normalized_digest: "b".repeat(64),
            image_path: PathBuf::from("/images/objects/b.raw"),
            manifest_path: PathBuf::from("/images/aliases/coreos-latest.json"),
            unchanged: true,
            sealed: false,
            aliases: vec![
                "fedora-coreos-latest".to_string(),
                "coreos-latest".to_string(),
            ],
        };
        let value = image_pull_json(&result);
        assert_eq!(value["command"], "image.pull");
        assert_eq!(value["summary"]["change"], "unchanged");
        assert_eq!(value["image"]["canonical_alias"], "fedora-coreos-latest");
        assert_eq!(value["image"]["sealed"], false);
        assert_eq!(value["image"]["sha256"], "b".repeat(64));
    }

    #[test]
    fn image_seal_options_accept_path_and_json_in_any_order() {
        let args = ["--format", "json", "--tag", "v1", "base.raw"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_image_seal_options(args).unwrap(),
            ImageSealOptions {
                input: "base.raw".to_string(),
                tag: Some("v1".to_string()),
                format: OutputFormat::Json,
            }
        );
    }

    #[test]
    fn resolves_one_local_image_by_name() {
        let directory = test_directory("image-resolve");
        fs::create_dir_all(&directory).unwrap();
        let image = directory.join("ubuntu-base.qcow2");
        fs::write(&image, b"fake").unwrap();

        assert_eq!(
            resolve_image_input("ubuntu-base", &directory).unwrap(),
            image
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn image_cleanup_matches_guest_agent_base_pipeline() {
        assert!(IMAGE_CLEANUP_COMMANDS
            .iter()
            .any(|command| command.contains("cloud-init clean --logs")));
        assert!(IMAGE_CLEANUP_COMMANDS
            .iter()
            .any(|command| command.contains("truncate -s 0 /etc/machine-id")));
        assert!(IMAGE_CLEANUP_COMMANDS
            .iter()
            .any(|command| command.contains("/etc/ssh") && command.contains("ssh_host_*")));
        assert!(IMAGE_CLEANUP_COMMANDS
            .iter()
            .any(|command| command.contains("/var/lib/systemd/random-seed")));
        assert!(IMAGE_PRESERVATION_CHECKS
            .iter()
            .any(|command| command.contains("/usr/local/sbin/vzctl-agent")));
        assert!(IMAGE_PRESERVATION_CHECKS
            .iter()
            .any(|command| command.contains("vzctl-agent.service")));
        assert!(IMAGE_PRESERVATION_CHECKS
            .iter()
            .any(|command| command.contains("/etc/init.d/vzctl-agent")));
        assert!(IMAGE_PRESERVATION_CHECKS
            .iter()
            .any(|command| command.contains("image-metadata.json")));
    }

    #[test]
    fn seal_writes_marker_locks_image_and_is_idempotent() {
        let directory = test_directory("image-seal");
        let images_directory = directory.join("images");
        fs::create_dir_all(&images_directory).unwrap();
        let image = directory.join("base.raw");
        fs::write(&image, b"fake image").unwrap();
        let options = ImageSealOptions {
            input: image.to_string_lossy().to_string(),
            tag: Some("v1".to_string()),
            format: OutputFormat::Json,
        };
        let backend = RecordingSealBackend::new("raw");

        let result = seal_image_in_dir(&options, &backend, &images_directory).unwrap();
        assert_eq!(
            backend.calls(),
            vec![
                "inspect",
                "preserved",
                "customize",
                "preserved",
                "clone-safe"
            ]
        );
        assert!(result.marker_path.is_file());
        assert_eq!(
            fs::metadata(&result.source_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o222,
            0
        );

        let unused_backend = RecordingSealBackend::new("raw");
        let repeated = seal_image_in_dir(&options, &unused_backend, &images_directory).unwrap();
        assert!(repeated.already_sealed);
        assert!(unused_backend.calls().is_empty());

        let mut writable = fs::metadata(&result.source_path).unwrap().permissions();
        writable.set_mode(0o600);
        fs::set_permissions(&result.source_path, writable).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn image_seal_json_matches_golden_contract() {
        let result = ImageSealResult {
            name: "ubuntu-base".to_string(),
            source_path: PathBuf::from("/images/ubuntu-base.raw"),
            image_format: "raw".to_string(),
            marker_path: PathBuf::from("/images/ubuntu-base-abc.sealed.json"),
            already_sealed: false,
        };
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/image-seal.json")).unwrap();
        assert_eq!(image_seal_json(&result), expected);
    }

    #[test]
    fn vm_create_options_accept_required_flags_in_any_order() {
        let args = [
            "--data-disk",
            "64",
            "web-1",
            "--format",
            "json",
            "--role",
            "router",
            "--network",
            "lan",
            "--from",
            "ubuntu-base",
        ]
        .into_iter()
        .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(args).unwrap(),
            VmCreateOptions {
                id: "web-1".to_string(),
                from: "ubuntu-base".to_string(),
                data_disk_gib: 64,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: vec!["router".to_string()],
                requested_network: Some("lan".to_string()),
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Json,
            }
        );
    }

    #[test]
    fn vm_create_rejects_unsafe_id_and_zero_data_disk() {
        let unsafe_id = ["../web", "--from", "base", "--data-disk", "1"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(unsafe_id).unwrap_err().code,
            EXIT_INVALID_INPUT
        );
        let zero_size = ["web", "--from", "base", "--data-disk", "0"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(zero_size).unwrap_err().code,
            EXIT_INVALID_INPUT
        );
    }

    #[test]
    fn valid_vm_id_accepts_flat_and_project_slash_forms() {
        assert!(valid_vm_id("web"));
        assert!(valid_vm_id("edge-dmz/web"));
        assert!(!valid_vm_id("../web"));
        assert!(!valid_vm_id("a/b/c"));
        assert!(!valid_vm_id("/web"));
        assert!(!valid_vm_id("edge-dmz/"));
        assert!(!valid_vm_id(""));
    }

    #[test]
    fn resolve_create_vm_id_namespaces_with_project() {
        assert_eq!(
            resolve_create_vm_id("web", None).unwrap(),
            "web".to_string()
        );
        assert_eq!(
            resolve_create_vm_id("web", Some("edge-dmz")).unwrap(),
            "edge-dmz/web".to_string()
        );
        assert_eq!(
            resolve_create_vm_id("edge-dmz/web", Some("edge-dmz")).unwrap(),
            "edge-dmz/web".to_string()
        );
        assert!(resolve_create_vm_id("other/web", Some("edge-dmz")).is_err());
    }

    #[test]
    fn vm_create_with_project_prefixes_flat_id() {
        let args = [
            "web",
            "--from",
            "ubuntu-base",
            "--data-disk",
            "4",
            "--project",
            "edge-dmz",
        ]
        .into_iter()
        .map(str::to_string);
        let options = parse_vm_create_options(args).unwrap();
        assert_eq!(options.id, "edge-dmz/web");
        assert_eq!(options.project.as_deref(), Some("edge-dmz"));
    }

    #[test]
    fn vm_create_rejects_project_prefix_mismatch() {
        let args = [
            "lab/web",
            "--from",
            "ubuntu-base",
            "--data-disk",
            "4",
            "--project",
            "edge-dmz",
        ]
        .into_iter()
        .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(args).unwrap_err().code,
            EXIT_INVALID_INPUT
        );
    }

    #[test]
    fn docker_role_defaults_project_and_namespaces_id() {
        let args = [
            "web",
            "--from",
            "ubuntu-base",
            "--data-disk",
            "4",
            "--role",
            "docker",
        ]
        .into_iter()
        .map(str::to_string);
        let options = parse_vm_create_options(args).unwrap();
        assert_eq!(options.project.as_deref(), Some("default"));
        assert_eq!(options.id, "default/web");
    }

    #[test]
    fn vm_create_accepts_root_password_flag() {
        let args = [
            "web",
            "--from",
            "ubuntu-base",
            "--data-disk",
            "4",
            "--root-password",
            "s3cret!",
        ]
        .into_iter()
        .map(str::to_string);
        let options = parse_vm_create_options(args).unwrap();
        assert_eq!(options.root_password.as_deref(), Some("s3cret!"));
        let snippet = cloud_init_root_password_snippet("s3cret!");
        assert!(snippet.contains("disable_root: false"));
        assert!(snippet.contains("ssh_pwauth: true"));
        assert!(snippet.contains("name: root"));
        assert!(snippet.contains("type: text"));
        assert!(snippet.contains("s3cret!"));
    }

    #[test]
    fn vm_create_accepts_cpus_and_memory_flags() {
        let args = [
            "web",
            "--from",
            "ubuntu-base",
            "--data-disk",
            "4",
            "--cpus",
            "4",
            "--memory",
            "2Gi",
        ]
        .into_iter()
        .map(str::to_string);
        let options = parse_vm_create_options(args).unwrap();
        assert_eq!(options.cpus, 4);
        assert_eq!(options.memory_mib, 2048);
        assert_eq!(parse_memory_mib("2048").unwrap(), 2048);
        assert_eq!(parse_memory_mib("2G").unwrap(), 2048);
        assert_eq!(parse_memory_mib("2Gi").unwrap(), 2048);
    }

    #[test]
    fn vm_create_rejects_invalid_cpus_and_memory() {
        let zero_cpus = ["web", "--from", "base", "--data-disk", "1", "--cpus", "0"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(zero_cpus).unwrap_err().code,
            EXIT_INVALID_INPUT
        );
        let bad_memory = ["web", "--from", "base", "--data-disk", "1", "--memory", "0"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(bad_memory).unwrap_err().code,
            EXIT_INVALID_INPUT
        );
        let junk_memory = [
            "web",
            "--from",
            "base",
            "--data-disk",
            "1",
            "--memory",
            "lots",
        ]
        .into_iter()
        .map(str::to_string);
        assert_eq!(
            parse_vm_create_options(junk_memory).unwrap_err().code,
            EXIT_INVALID_INPUT
        );
    }

    #[test]
    fn vm_create_writes_resources_into_manifest() {
        let directory = std::env::temp_dir().join(format!(
            "vzctl-resources-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        fs::create_dir_all(&images_directory).unwrap();
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("apfs");
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "web".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: 4,
                memory_mib: 2048,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(result.bundle_path.join("vm.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["resources"]["cpus"], json!(4));
        assert_eq!(manifest["resources"]["memory_mib"], json!(2048));
        assert_eq!(manifest["mounts"], json!([]));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn vm_create_persists_mounts_in_manifest() {
        let directory = test_directory("vm-mounts-manifest");
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        let share = directory.join("share");
        fs::create_dir_all(&share).unwrap();
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("apfs");
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "web".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: vec![mounts::ResolvedMount {
                    name: "app".to_string(),
                    source: share.clone(),
                    target: "/srv/app".to_string(),
                    read_only: false,
                }],
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(result.bundle_path.join("vm.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["mounts"][0]["name"], json!("app"));
        assert_eq!(manifest["mounts"][0]["target"], json!("/srv/app"));
        assert_eq!(
            manifest["mounts"][0]["source"].as_str().unwrap(),
            share.to_string_lossy()
        );
        let user_data = &backend.seeds()[0].2;
        assert!(user_data.contains("/usr/local/lib/vzctl/virtiofs-bind"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn vm_create_root_password_lands_in_user_data() {
        let directory = std::env::temp_dir().join(format!(
            "vzctl-rootpw-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        fs::create_dir_all(&images_directory).unwrap();
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("apfs");
        create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "web".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: Some("pass:word".to_string()),
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();
        let user_data = &backend.seeds()[0].2;
        assert!(user_data.contains("disable_root: false"));
        assert!(user_data.contains("chpasswd:"));
        assert!(user_data.contains("name: root"));
        assert!(
            user_data.contains("pass:word")
                || user_data.contains("'pass:word'")
                || user_data.contains("\"pass:word\"")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn identity_helpers_produce_cloud_init_safe_values() {
        assert_eq!(cloud_init_fqdn("Web_01.Example"), "web-01.example");
        assert_eq!(cloud_init_fqdn("edge-dmz/Web_01"), "web-01");
        assert_eq!(cloud_init_fqdn("..."), "vm");
        assert_eq!(vm_id_basename("edge-dmz/web"), "web");
        assert_eq!(vm_id_basename("web"), "web");
        assert_eq!(base64url_unpadded(&[0xfb, 0xff, 0xef]), "-__v");
        assert_eq!(base64url_unpadded(&[0xff]), "_w");
    }

    #[test]
    fn namespaced_vm_create_uses_nested_bundle_path() {
        let directory = test_directory("vm-namespaced-bundle");
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("apfs");
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "edge-dmz/web".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: Some("edge-dmz".to_string()),
                mounts: Vec::new(),
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();
        assert_eq!(result.id, "edge-dmz/web");
        assert_eq!(
            result.bundle_path,
            vms_directory.join("edge-dmz").join("web")
        );
        assert!(result.bundle_path.join("vm.json").is_file());
        assert_eq!(result.identity.hostname, "web");
        assert_eq!(result.identity.fqdn, "web");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn automatic_default_network_uses_only_hypervisor_dns_and_project_search() {
        let identity = VmIdentity {
            instance_id: "test".to_string(),
            hostname: "web".to_string(),
            fqdn: "web".to_string(),
            mac_addresses: vec!["02:12:34:56:78:9a".to_string()],
        };
        let network = network::VmNetworkSelection {
            network: "lan".to_string(),
            cidr: "10.70.0.0/24".to_string(),
            ip: "10.70.0.10".to_string(),
            gateway: "10.70.0.0".to_string(),
            dns: "10.70.0.0".to_string(),
            project: Some("edge-dmz".to_string()),
            prefix: 24,
            automatic: true,
            created: true,
            backend: "vmnet".to_string(),
        };

        assert_eq!(
            render_cloud_init_network_config(&identity, std::slice::from_ref(&network)),
            concat!(
                "version: 2\n",
                "ethernets:\n",
                "  nic0:\n",
                "    match:\n",
                "      macaddress: \"02:12:34:56:78:9a\"\n",
                "    set-name: enp0s1\n",
                "    dhcp4: false\n",
                "    dhcp6: false\n",
                "    addresses:\n",
                "      - 10.70.0.10/24\n",
                "    routes:\n",
                "      - to: default\n",
                "        via: 10.70.0.0\n",
                "        on-link: true\n",
                "    nameservers:\n",
                "      addresses:\n",
                "        - 10.70.0.0\n",
                "      search:\n",
                "        - edge-dmz.vz.test\n",
            )
        );
    }

    #[test]
    fn network_without_project_does_not_invent_a_dns_search_zone() {
        let identity = VmIdentity {
            instance_id: "test".to_string(),
            hostname: "web".to_string(),
            fqdn: "web".to_string(),
            mac_addresses: vec!["02:12:34:56:78:9a".to_string()],
        };
        let network = network::VmNetworkSelection {
            network: "lan".to_string(),
            cidr: "10.70.0.0/24".to_string(),
            ip: "10.70.0.10".to_string(),
            gateway: "10.70.0.0".to_string(),
            dns: "10.70.0.0".to_string(),
            project: None,
            prefix: 24,
            automatic: true,
            created: true,
            backend: "vmnet".to_string(),
        };

        let config = render_cloud_init_network_config(&identity, std::slice::from_ref(&network));
        assert!(config.contains("addresses:\n        - 10.70.0.0"));
        assert!(!config.contains("search:"));
        assert!(!config.contains("nameserver 127.0.0.1"));
    }

    #[test]
    fn two_linked_clones_keep_base_read_only_and_get_sparse_data_disks() {
        let directory = test_directory("vm-linked-clones");
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("apfs");
        let marker_path = fs::read_dir(&images_directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .unwrap();
        let base_before = fs::read(&image).unwrap();
        let marker_before = fs::read(&marker_path).unwrap();

        let mut instance_ids = Vec::new();
        let mut mac_addresses = Vec::new();
        let mut tokens = Vec::new();
        for id in ["web-1", "web-2"] {
            let result = create_vm_bundle_in_dirs(
                &VmCreateOptions {
                    id: id.to_string(),
                    from: image.to_string_lossy().to_string(),
                    data_disk_gib: 1,
                    cpus: DEFAULT_VM_CPUS,
                    memory_mib: DEFAULT_VM_MEMORY_MIB,
                    roles: Vec::new(),
                    requested_network: None,
                    network: None,
                    networks: Vec::new(),
                    root_password: None,
                    cloud_init: None,
                    project: None,
                    mounts: Vec::new(),
                    format: OutputFormat::Json,
                },
                &backend,
                &images_directory,
                &vms_directory,
            )
            .unwrap();
            assert_eq!(result.clone_mode, CloneMode::Linked);
            assert_eq!(
                fs::read(&result.root_disk_path).unwrap(),
                b"sealed base blocks"
            );
            assert_eq!(
                fs::metadata(&result.data_disk_path).unwrap().len(),
                1024 * 1024 * 1024
            );
            assert!(result.cidata_path.is_file());
            assert_eq!(
                fs::metadata(&result.cidata_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            let token = fs::read_to_string(&result.agent_token_path).unwrap();
            assert_eq!(token.trim().len(), 43);
            assert!(!token.contains('='));
            instance_ids.push(result.identity.instance_id);
            mac_addresses.push(result.identity.mac_addresses[0].clone());
            tokens.push(token);
        }

        assert_ne!(instance_ids[0], instance_ids[1]);
        assert_ne!(mac_addresses[0], mac_addresses[1]);
        assert!(mac_addresses.iter().all(|mac| mac.starts_with("02:")));
        assert_ne!(tokens[0], tokens[1]);
        for (meta_data, network_config, user_data) in backend.seeds() {
            assert!(meta_data.contains("instance-id: "));
            assert!(network_config.contains("macaddress: \"02:"));
            assert!(network_config.contains("dhcp4: true"));
            assert!(user_data.contains("ssh_deletekeys: true"));
            assert!(user_data.contains("ed25519"));
            assert!(user_data.contains("rsa"));
            assert!(user_data.contains("ssh_genkeytypes"));
        }
        assert_eq!(
            fs::metadata(&image).unwrap().permissions().mode() & 0o222,
            0
        );
        assert_eq!(fs::read(&image).unwrap(), base_before);
        assert_eq!(fs::read(&marker_path).unwrap(), marker_before);
        assert_eq!(
            backend.calls(),
            vec!["linked", "sparse", "seed", "linked", "sparse", "seed"]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn non_apfs_uses_documented_full_copy_fallback() {
        let directory = test_directory("vm-full-clone");
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("hfs");
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "web".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Human,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();

        assert_eq!(result.clone_mode, CloneMode::Full);
        assert_eq!(backend.calls(), vec!["full", "sparse", "seed"]);
        assert_eq!(vm_create_json(&result)["status"], "warn");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn router_role_persists_manifest_and_cloud_init_forwarding() {
        let directory = test_directory("vm-router-role");
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::new("apfs");
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "router".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: vec!["router".to_string()],
                requested_network: Some("lan".to_string()),
                network: Some(network::VmNetworkSelection {
                    network: "lan".to_string(),
                    cidr: "10.70.0.0/24".to_string(),
                    ip: "10.70.0.10".to_string(),
                    gateway: "10.70.0.0".to_string(),
                    dns: "10.70.0.0".to_string(),
                    project: Some("edge-dmz".to_string()),
                    prefix: 24,
                    automatic: false,
                    created: true,
                    backend: "vmnet".to_string(),
                }),
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();

        let manifest: Value =
            serde_json::from_slice(&fs::read(result.bundle_path.join("vm.json")).unwrap()).unwrap();
        assert_eq!(manifest["roles"], json!(["router"]));
        let user_data = &backend.seeds()[0].2;
        assert!(user_data.contains("/etc/sysctl.d/90-vzctl-router.conf"));
        assert!(user_data.contains("net.ipv4.ip_forward=1"));
        assert!(user_data.contains("iptables -P FORWARD DROP"));
        assert!(user_data.contains("/usr/local/lib/vzctl/virtiofs-bind"));
        assert!(user_data.contains("/etc/sudoers.d/vzctl-virtiofs"));
        assert!(user_data.contains("/etc/sudoers.d/vzctl-agent"));
        assert!(user_data.contains("vzctl-agent ALL=(ALL) NOPASSWD:ALL"));
        assert!(user_data.contains("NoNewPrivileges=no"));
        assert!(user_data.contains("systemctl"));
        assert!(user_data.contains("daemon-reload"));
        assert!(user_data.contains("vzctl-agent.service"));
        let network_config = &backend.seeds()[0].1;
        assert!(network_config.contains("10.70.0.10/24"));
        assert!(network_config.contains("via: 10.70.0.0"));
        assert!(network_config.contains("on-link: true"));
        assert!(network_config.contains("addresses:\n        - 10.70.0.0"));
        assert!(network_config.contains("search:\n        - edge-dmz.vz.test"));
        assert!(network_config.contains("set-name: enp0s1"));
        assert!(!network_config.contains("10.70.0.2"));
        assert!(!network_config.contains("dhcp4: true"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn clonefile_failure_is_exit_16_and_removes_partial_bundle() {
        let directory = test_directory("vm-clone-failure");
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        let image = prepare_sealed_test_image(&directory, &images_directory);
        let backend = RecordingVmDiskBackend::failing_clone("apfs");
        let failure = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: "web".to_string(),
                from: image.to_string_lossy().to_string(),
                data_disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap_err();

        assert_eq!(failure.code, EXIT_VM_DISK_PREP_FAILED);
        assert!(failure.message.contains("clonefile failed"));
        assert!(!vms_directory.join("web").exists());
        assert_eq!(
            fs::metadata(&image).unwrap().permissions().mode() & 0o222,
            0
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn two_native_apfs_clonefiles_diverge_without_changing_shared_base() {
        let directory = test_directory("vm-native-apfs");
        fs::create_dir_all(&directory).unwrap();
        if !filesystem_type(&directory).is_some_and(|value| value.eq_ignore_ascii_case("apfs")) {
            fs::remove_dir_all(directory).unwrap();
            return;
        }
        let images_directory = directory.join("images");
        let vms_directory = directory.join("vms");
        fs::create_dir_all(&images_directory).unwrap();
        let image = directory.join("base.raw");
        let original = vec![0x5a; 4 * 1024 * 1024];
        fs::write(&image, &original).unwrap();
        seal_image_in_dir(
            &ImageSealOptions {
                input: image.to_string_lossy().to_string(),
                tag: Some("v1".to_string()),
                format: OutputFormat::Json,
            },
            &RecordingSealBackend::new("raw"),
            &images_directory,
        )
        .unwrap();

        let mut results = Vec::new();
        for id in ["native-a", "native-b"] {
            let result = create_vm_bundle_in_dirs(
                &VmCreateOptions {
                    id: id.to_string(),
                    from: image.to_string_lossy().to_string(),
                    data_disk_gib: 1,
                    cpus: DEFAULT_VM_CPUS,
                    memory_mib: DEFAULT_VM_MEMORY_MIB,
                    roles: Vec::new(),
                    requested_network: None,
                    network: None,
                    networks: Vec::new(),
                    root_password: None,
                    cloud_init: None,
                    project: None,
                    mounts: Vec::new(),
                    format: OutputFormat::Human,
                },
                &NativeVmDiskBackend,
                &images_directory,
                &vms_directory,
            )
            .unwrap();
            assert_eq!(result.clone_mode, CloneMode::Linked);
            assert_eq!(fs::read(&result.root_disk_path).unwrap(), original);
            results.push(result);
        }
        let mut clone = OpenOptions::new()
            .write(true)
            .open(&results[0].root_disk_path)
            .unwrap();
        clone.write_all(b"diverged").unwrap();
        clone.sync_all().unwrap();
        assert_eq!(fs::read(&image).unwrap(), original);
        assert_ne!(fs::read(&results[0].root_disk_path).unwrap(), original);
        assert_eq!(fs::read(&results[1].root_disk_path).unwrap(), original);
        assert_eq!(
            fs::metadata(&image).unwrap().permissions().mode() & 0o222,
            0
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_backend_builds_private_nocloud_iso() {
        let directory = test_directory("vm-native-cidata");
        let seed_directory = directory.join("seed");
        fs::create_dir_all(&seed_directory).unwrap();
        fs::write(seed_directory.join("meta-data"), "instance-id: test\n").unwrap();
        fs::write(seed_directory.join("network-config"), "version: 2\n").unwrap();
        fs::write(seed_directory.join("user-data"), "#cloud-config\n").unwrap();
        let destination = directory.join("cidata.iso");

        NativeVmDiskBackend
            .create_cloud_init_iso(&seed_directory, &destination)
            .unwrap();

        assert!(destination.is_file());
        assert!(fs::metadata(&destination).unwrap().len() > 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn vm_create_json_matches_golden_contract() {
        let result = VmCreateResult {
            id: "web".to_string(),
            bundle_path: PathBuf::from("/state/vms/web"),
            source: ImageSealResult {
                name: "ubuntu-base".to_string(),
                source_path: PathBuf::from("/images/ubuntu-base.raw"),
                image_format: "raw".to_string(),
                marker_path: PathBuf::from("/images/ubuntu-base-abc.sealed.json"),
                already_sealed: true,
            },
            root_disk_path: PathBuf::from("/state/vms/web/disk.raw"),
            data_disk_path: PathBuf::from("/state/vms/web/dataDisk.raw"),
            cidata_path: PathBuf::from("/state/vms/web/cidata.iso"),
            agent_token_path: PathBuf::from("/state/vms/web/agent.token"),
            data_disk_gib: 64,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: Vec::new(),
            mounts: Vec::new(),
            clone_mode: CloneMode::Linked,
            filesystem: "apfs".to_string(),
            identity: VmIdentity {
                instance_id: "123e4567-e89b-42d3-a456-426614174000".to_string(),
                hostname: "web".to_string(),
                fqdn: "web".to_string(),
                mac_addresses: vec!["02:12:34:56:78:9a".to_string()],
            },
            network: Some(network::VmNetworkSelection {
                network: "lan".to_string(),
                cidr: "10.70.0.0/24".to_string(),
                ip: "10.70.0.10".to_string(),
                gateway: "10.70.0.0".to_string(),
                dns: "10.70.0.0".to_string(),
                project: Some("edge-dmz".to_string()),
                prefix: 24,
                automatic: true,
                created: true,
                backend: "vmnet".to_string(),
            }),
            networks: Vec::new(),
        };
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/vm-create.json")).unwrap();
        assert_eq!(vm_create_json(&result), expected);
    }

    #[test]
    fn event_options_parse_filter_list() {
        let args = ["--filter", "vm.*,apply.*"].into_iter().map(str::to_string);
        assert_eq!(
            parse_events_options(args).unwrap(),
            EventsOptions {
                filter: Some("vm.*,apply.*".to_string()),
            }
        );
    }

    #[test]
    fn event_filter_accepts_exact_and_suffix_wildcards() {
        assert!(valid_event_filter("vm.*, apply.failed"));
        assert!(valid_event_filter("*"));
        assert!(!valid_event_filter("vm.*.failed"));
        assert!(!valid_event_filter("vm.,"));
    }

    #[test]
    fn parses_available_kib_from_df() {
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                  /dev/disk3s1 100000 25000 75000 25% /System/Volumes/Data\n";
        assert_eq!(parse_df_available_kib(df), Some(75_000));
    }

    #[test]
    fn detects_virtualization_entitlement() {
        let plist = "<key>com.apple.security.virtualization</key><true/>";
        assert!(has_virtualization_entitlement(plist));
        assert!(!has_virtualization_entitlement("<dict></dict>"));
    }

    #[test]
    fn parses_apfs_from_mount_output() {
        let mounts = "/dev/disk3s1s1 on / (apfs, sealed, local, read-only)\n\
                      devfs on /dev (devfs, local, nobrowse)\n";
        assert_eq!(
            filesystem_for_mount_point(mounts, "/").as_deref(),
            Some("apfs")
        );
    }

    #[test]
    fn resolver_requires_localhost_and_configured_port() {
        let directory =
            std::env::temp_dir().join(format!("vzctl-doctor-resolver-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("demo.vz.test");
        fs::write(&path, "nameserver 127.0.0.1\nport 15353\n").unwrap();
        assert!(resolver_points_to(&path, 15353));
        assert!(!resolver_points_to(&path, 15354));
        fs::remove_dir_all(directory).unwrap();
    }

    fn test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vzctl-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn prepare_sealed_test_image(directory: &Path, images_directory: &Path) -> PathBuf {
        fs::create_dir_all(images_directory).unwrap();
        let image = directory.join("base.raw");
        fs::write(&image, b"sealed base blocks").unwrap();
        seal_image_in_dir(
            &ImageSealOptions {
                input: image.to_string_lossy().to_string(),
                tag: Some("v1".to_string()),
                format: OutputFormat::Json,
            },
            &RecordingSealBackend::new("raw"),
            images_directory,
        )
        .unwrap();
        image
    }

    struct RecordingVmDiskBackend {
        filesystem: String,
        fail_clone: bool,
        calls: RefCell<Vec<&'static str>>,
        seeds: RefCell<Vec<(String, String, String)>>,
    }

    impl RecordingVmDiskBackend {
        fn new(filesystem: &str) -> Self {
            Self {
                filesystem: filesystem.to_string(),
                fail_clone: false,
                calls: RefCell::new(Vec::new()),
                seeds: RefCell::new(Vec::new()),
            }
        }

        fn failing_clone(filesystem: &str) -> Self {
            Self {
                filesystem: filesystem.to_string(),
                fail_clone: true,
                calls: RefCell::new(Vec::new()),
                seeds: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }

        fn seeds(&self) -> Vec<(String, String, String)> {
            self.seeds.borrow().clone()
        }
    }

    impl VmDiskBackend for RecordingVmDiskBackend {
        fn filesystem_type(&self, _path: &Path) -> Option<String> {
            Some(self.filesystem.clone())
        }

        fn clone_linked(&self, source: &Path, destination: &Path) -> Result<(), io::Error> {
            self.calls.borrow_mut().push("linked");
            if self.fail_clone {
                return Err(io::Error::other("injected clonefile failure"));
            }
            fs::copy(source, destination).map(|_| ())
        }

        fn copy_full(&self, source: &Path, destination: &Path) -> Result<(), io::Error> {
            self.calls.borrow_mut().push("full");
            fs::copy(source, destination).map(|_| ())
        }

        fn create_sparse(&self, path: &Path, size_bytes: u64) -> Result<(), io::Error> {
            self.calls.borrow_mut().push("sparse");
            let file = File::create(path)?;
            file.set_len(size_bytes)
        }

        fn create_cloud_init_iso(
            &self,
            seed_directory: &Path,
            destination: &Path,
        ) -> Result<(), io::Error> {
            self.calls.borrow_mut().push("seed");
            self.seeds.borrow_mut().push((
                fs::read_to_string(seed_directory.join("meta-data"))?,
                fs::read_to_string(seed_directory.join("network-config"))?,
                fs::read_to_string(seed_directory.join("user-data"))?,
            ));
            fs::write(destination, b"test NoCloud ISO")
        }
    }

    struct RecordingSealBackend {
        image_format: String,
        calls: RefCell<Vec<&'static str>>,
    }

    impl RecordingSealBackend {
        fn new(image_format: &str) -> Self {
            Self {
                image_format: image_format.to_string(),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }

    impl ImageSealBackend for RecordingSealBackend {
        fn inspect_format(&self, _path: &Path) -> Result<String, SealFailure> {
            self.calls.borrow_mut().push("inspect");
            Ok(self.image_format.clone())
        }

        fn verify_preserved(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
            self.calls.borrow_mut().push("preserved");
            Ok(())
        }

        fn customize(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
            self.calls.borrow_mut().push("customize");
            Ok(())
        }

        fn verify_clone_safe(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
            self.calls.borrow_mut().push("clone-safe");
            Ok(())
        }
    }
}
