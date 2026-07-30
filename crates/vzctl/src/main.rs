use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

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

fn main() -> ExitCode {
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
        Some("apply") => apply_stub(args),
        Some(other) => {
            eprintln!("unknown command: {other}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn print_help() {
    println!(
        "\
vzctl — Environments-as-Code for macOS Virtualization (Alpha stub)

Commands:
  doctor [--format human|json] [--min-free-gib N]
                      Check host baseline and supervisor health
  version [--format human|json]
  apply [--resume|--abort]   (stub — see ADR 0003)
  help

Stable exit codes:
  0   success (warnings allowed)
  2   usage or unknown command
  3   invalid input or validation
  5   incomplete apply journal
  6   apply lease held
  10  supervisor socket or health is bad
  11  macOS 26 baseline is not met
  12  command unavailable or not implemented"
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

fn apply_stub(args: impl Iterator<Item = String>) -> ExitCode {
    let mut resume = false;
    let mut abort = false;

    for arg in args {
        match arg.as_str() {
            "--resume" => resume = true,
            "--abort" => abort = true,
            _ => {
                eprintln!("unknown apply option: {arg}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    if resume && abort {
        eprintln!("apply accepts only one of --resume or --abort");
        return ExitCode::from(EXIT_INVALID_INPUT);
    }

    eprintln!(
        "apply is not implemented yet; journal/reconcile remains tracked by ADR 0003 \
         (future blockers: exit {EXIT_INCOMPLETE_JOURNAL}=incomplete journal, \
         exit {EXIT_LEASE_HELD}=lease held)"
    );
    ExitCode::from(EXIT_UNAVAILABLE)
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
    checks.push(check_supervisor());

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
            "custom vmnet API baseline is available; live network creation is not tested",
            json!({
                "live_create_tested": false,
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

    Check::new(
        "supervisor.health",
        CheckStatus::Ok,
        format!(
            "supervisor {} (pid {}, db ok)",
            result["version"].as_str().unwrap_or("unknown"),
            result["pid"]
        ),
        json!({
            "socket": path,
            "running": true,
            "version": result["version"],
            "pid": result["pid"],
            "db_ok": true,
        }),
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
}
