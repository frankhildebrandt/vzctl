use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_UNAVAILABLE: u8 = 12;
const EXIT_SERVICES: u8 = 25;

const AGENT_GONE_TIMEOUT_TENTHS: u32 = 100;
const NET_AGENT_GONE_TIMEOUT_TENTHS: u32 = 600;
const SOCKET_READY_TIMEOUT_TENTHS: u32 = 100;
const HELPER_STOP_TIMEOUT_TENTHS: u32 = 600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Status,
    Start,
    Stop,
    Restart,
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Status => "services.status",
            Self::Start => "services.start",
            Self::Stop => "services.stop",
            Self::Restart => "services.restart",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServiceId {
    Net,
    Edge,
    Supervisor,
    All,
}

impl ServiceId {
    fn parse(value: Option<&str>) -> Result<Self, Failure> {
        match value.unwrap_or("all") {
            "all" => Ok(Self::All),
            "net" => Ok(Self::Net),
            "edge" => Ok(Self::Edge),
            "supervisor" => Ok(Self::Supervisor),
            other => Err(Failure::new(
                EXIT_INVALID,
                format!("unknown service: {other} (expected all, net, edge, or supervisor)"),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Net => "net",
            Self::Edge => "edge",
            Self::Supervisor => "supervisor",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ServiceSpec {
    id: ServiceId,
    label: &'static str,
    plist_name: &'static str,
    socket_name: Option<&'static str>,
    kickstart_kill: bool,
    stop_wait_tenths: u32,
}

const SERVICE_NET: ServiceSpec = ServiceSpec {
    id: ServiceId::Net,
    label: "com.vzctl.net",
    plist_name: "com.vzctl.net.plist",
    socket_name: Some("net.sock"),
    kickstart_kill: false,
    stop_wait_tenths: NET_AGENT_GONE_TIMEOUT_TENTHS,
};

const SERVICE_EDGE: ServiceSpec = ServiceSpec {
    id: ServiceId::Edge,
    label: "com.vzctl.edge",
    plist_name: "com.vzctl.edge.plist",
    socket_name: Some("edge.sock"),
    kickstart_kill: true,
    stop_wait_tenths: AGENT_GONE_TIMEOUT_TENTHS,
};

const SERVICE_SUPERVISOR: ServiceSpec = ServiceSpec {
    id: ServiceId::Supervisor,
    label: "com.vzctl.supervisor",
    plist_name: "com.vzctl.supervisor.plist",
    socket_name: None,
    kickstart_kill: true,
    stop_wait_tenths: AGENT_GONE_TIMEOUT_TENTHS,
};

const ALL_SERVICES: [ServiceSpec; 3] = [SERVICE_NET, SERVICE_EDGE, SERVICE_SUPERVISOR];

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

#[derive(Debug, Clone)]
struct ServiceStatus {
    spec: ServiceSpec,
    plist: PathBuf,
    loaded: bool,
    socket: Option<PathBuf>,
    socket_ready: Option<bool>,
}

#[derive(Debug, Clone)]
struct ServiceChange {
    spec: ServiceSpec,
    action: Action,
    changed: bool,
    message: String,
}

struct Options {
    action: Action,
    target: ServiceId,
    format: Format,
}

pub(crate) fn command(args: impl Iterator<Item = String>) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let mut iter = args.into_iter().peekable();
    let Some(subcommand) = iter.next() else {
        print_usage();
        return ExitCode::from(EXIT_USAGE);
    };
    match subcommand.as_str() {
        "status" | "start" | "stop" | "restart" => match parse_options(&subcommand, iter) {
            Ok(options) => run(options),
            Err(failure) => {
                eprintln!("{}", failure.message);
                ExitCode::from(failure.code)
            }
        },
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown services subcommand: {other}");
            print_usage();
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn print_usage() {
    eprintln!(
        "\
usage: vzctl services status [--format human|json]
       vzctl services start  [all|net|edge|supervisor] [--format human|json]
       vzctl services stop   [all|net|edge|supervisor] [--format human|json]
       vzctl services restart [all|net|edge|supervisor] [--format human|json]"
    );
}

fn parse_options(
    action_name: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<Options, Failure> {
    let action = match action_name {
        "status" => Action::Status,
        "start" => Action::Start,
        "stop" => Action::Stop,
        "restart" => Action::Restart,
        _ => unreachable!(),
    };
    let mut target = ServiceId::All;
    let mut format = Format::Human;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => {
                        return Err(Failure::new(
                            EXIT_USAGE,
                            format!("unsupported services format: {value}"),
                        ));
                    }
                    None => {
                        return Err(Failure::new(EXIT_USAGE, "--format requires human or json"));
                    }
                };
            }
            "--" => break,
            other if other.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown services option: {other}"),
                ));
            }
            other => {
                if target != ServiceId::All {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "only one service target may be specified",
                    ));
                }
                target = ServiceId::parse(Some(other))?;
            }
        }
    }
    Ok(Options {
        action,
        target,
        format,
    })
}

fn run(options: Options) -> ExitCode {
    if let Err(message) = ensure_macos() {
        return finish_err(
            options.format,
            options.action,
            Failure::new(EXIT_UNAVAILABLE, message),
        );
    }
    match options.action {
        Action::Status => match collect_status() {
            Ok(statuses) => emit_status(options.format, statuses),
            Err(failure) => finish_err(options.format, options.action, failure),
        },
        Action::Start => match start_target(options.target) {
            Ok(changes) => emit_changes(options.format, Action::Start, changes, 0),
            Err(failure) => finish_err(options.format, options.action, failure),
        },
        Action::Stop => match stop_target(options.target) {
            Ok(changes) => emit_changes(options.format, Action::Stop, changes, 0),
            Err(failure) => finish_err(options.format, options.action, failure),
        },
        Action::Restart => match restart_target(options.target) {
            Ok(changes) => emit_changes(options.format, Action::Restart, changes, 0),
            Err(failure) => finish_err(options.format, options.action, failure),
        },
    }
}

fn finish_err(format: Format, action: Action, failure: Failure) -> ExitCode {
    match format {
        Format::Human => eprintln!("error: {}", failure.message),
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "apiVersion": API_VERSION,
                "command": action.command(),
                "status": "fail",
                "exit_code": failure.code,
                "summary": { "message": failure.message },
            }))
            .unwrap_or_else(|_| "{}".into())
        ),
    }
    ExitCode::from(failure.code)
}

fn ensure_macos() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("vzctl services is only supported on macOS".into())
    }
}

fn launch_agents_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library").join("LaunchAgents")
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

fn launchd_domain() -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

fn launchd_target(label: &str) -> String {
    format!("{}/{}", launchd_domain(), label)
}

fn plist_path(spec: ServiceSpec) -> PathBuf {
    launch_agents_dir().join(spec.plist_name)
}

fn ensure_plist(spec: ServiceSpec) -> Result<PathBuf, Failure> {
    let path = plist_path(spec);
    if path.is_file() {
        Ok(path)
    } else {
        Err(Failure::new(
            EXIT_UNAVAILABLE,
            format!(
                "launch agent {} is not installed (missing {}); run make install",
                spec.label,
                path.display()
            ),
        ))
    }
}

fn ensure_plists_for(target: ServiceId) -> Result<(), Failure> {
    for spec in specs_for_target(target) {
        ensure_plist(spec)?;
    }
    Ok(())
}

fn specs_for_target(target: ServiceId) -> Vec<ServiceSpec> {
    match target {
        ServiceId::All => ALL_SERVICES.to_vec(),
        ServiceId::Net => vec![SERVICE_NET],
        ServiceId::Edge => vec![SERVICE_EDGE],
        ServiceId::Supervisor => vec![SERVICE_SUPERVISOR],
    }
}

fn stop_order(target: ServiceId) -> Vec<ServiceSpec> {
    match target {
        ServiceId::All => vec![SERVICE_SUPERVISOR, SERVICE_EDGE, SERVICE_NET],
        ServiceId::Net => vec![SERVICE_NET],
        ServiceId::Edge => vec![SERVICE_EDGE],
        ServiceId::Supervisor => vec![SERVICE_SUPERVISOR],
    }
}

fn start_order(target: ServiceId) -> Vec<ServiceSpec> {
    match target {
        ServiceId::All => ALL_SERVICES.to_vec(),
        ServiceId::Net => vec![SERVICE_NET],
        ServiceId::Edge => vec![SERVICE_EDGE],
        ServiceId::Supervisor => vec![SERVICE_SUPERVISOR],
    }
}

fn agent_loaded(label: &str) -> bool {
    Command::new("launchctl")
        .arg("print")
        .arg(launchd_target(label))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn bootout(label: &str) {
    let _ = Command::new("launchctl")
        .args(["bootout", &launchd_target(label)])
        .status();
}

fn bootstrap(plist: &Path) -> Result<(), Failure> {
    let status = Command::new("launchctl")
        .args(["bootstrap", &launchd_domain(), &plist.display().to_string()])
        .status()
        .map_err(|error| {
            Failure::new(
                EXIT_SERVICES,
                format!("launchctl bootstrap failed: {error}"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_SERVICES,
            format!("launchctl bootstrap failed for {}", plist.display()),
        ))
    }
}

fn kickstart(label: &str, kill: bool) -> Result<(), Failure> {
    let target = launchd_target(label);
    let status = if kill {
        Command::new("launchctl")
            .args(["kickstart", "-k", &target])
            .status()
    } else {
        Command::new("launchctl")
            .args(["kickstart", &target])
            .status()
    }
    .map_err(|error| {
        Failure::new(
            EXIT_SERVICES,
            format!("launchctl kickstart failed: {error}"),
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_SERVICES,
            format!("launchctl kickstart failed for {label}"),
        ))
    }
}

fn wait_agent_gone(label: &str, timeout_tenths: u32) -> bool {
    for _ in 0..timeout_tenths {
        if !agent_loaded(label) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn wait_path_gone(path: &Path, timeout_tenths: u32) -> bool {
    for _ in 0..timeout_tenths {
        if !path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn wait_socket(path: &Path, timeout_tenths: u32) -> bool {
    for _ in 0..timeout_tenths {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn socket_path(spec: ServiceSpec) -> Option<PathBuf> {
    spec.socket_name.map(|name| state_dir().join(name))
}

fn collect_status() -> Result<Vec<ServiceStatus>, Failure> {
    let mut statuses = Vec::new();
    for spec in ALL_SERVICES {
        let plist = plist_path(spec);
        let loaded = plist.is_file() && agent_loaded(spec.label);
        let socket = socket_path(spec);
        let socket_ready = socket.as_ref().map(|path| path.exists());
        statuses.push(ServiceStatus {
            spec,
            plist,
            loaded,
            socket,
            socket_ready,
        });
    }
    Ok(statuses)
}

fn stop_vm_helpers() -> Result<(), Failure> {
    let socket = supervisor_socket_path();
    if supervisor_reachable(&socket) {
        if stop_vm_helpers_via_supervisor(&socket).is_ok() {
            return Ok(());
        }
    }
    stop_vm_helpers_via_signal()
}

fn supervisor_reachable(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).is_ok()
}

fn stop_vm_helpers_via_supervisor(socket_path: &Path) -> Result<(), Failure> {
    let records = rpc(socket_path, "vm.list", json!({}))?;
    let Some(vms) = records.as_array() else {
        return Ok(());
    };
    for record in vms {
        let state = record["state"].as_str().unwrap_or("");
        if matches!(state, "starting" | "running") {
            let vm_id = record["vm_id"].as_str().ok_or_else(|| {
                Failure::new(EXIT_SERVICES, "vm.list returned entry without vm_id")
            })?;
            rpc(socket_path, "vm.stop", json!({ "vm_id": vm_id }))?;
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let records = rpc(socket_path, "vm.list", json!({}))?;
        let active = records
            .as_array()
            .into_iter()
            .flatten()
            .any(|record| matches!(record["state"].as_str(), Some("starting" | "running")));
        if !active {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(Failure::new(
                EXIT_SERVICES,
                "VM helpers did not stop before timeout",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn stop_vm_helpers_via_signal() -> Result<(), Failure> {
    let uid = unsafe { libc::getuid() };
    let output = Command::new("/usr/bin/pgrep")
        .args(["-U", &uid.to_string(), "-x", "vz-helper"])
        .output()
        .map_err(|error| Failure::new(EXIT_SERVICES, format!("pgrep failed: {error}")))?;
    let pids = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse::<i32>().ok())
        .collect::<Vec<_>>();
    if pids.is_empty() {
        return Ok(());
    }
    eprintln!("stopping running VM helpers gracefully…");
    for pid in &pids {
        let _ = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    for _ in 0..HELPER_STOP_TIMEOUT_TENTHS {
        let alive = pids.iter().any(|pid| {
            Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        });
        if !alive {
            eprintln!("VM helpers stopped cleanly");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(Failure::new(
        EXIT_SERVICES,
        "vz-helper still running; refusing to stop vz-net",
    ))
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SERVICES,
            format!("connect {}: {error}", socket_path.display()),
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| Failure::new(EXIT_SERVICES, error.to_string()))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| Failure::new(EXIT_SERVICES, error.to_string()))?;
    let request = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
    writeln!(stream, "{request}")
        .map_err(|error| Failure::new(EXIT_SERVICES, format!("{method}: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| Failure::new(EXIT_SERVICES, format!("{method}: {error}")))?;
    let response: Value = serde_json::from_str(&line)
        .map_err(|error| Failure::new(EXIT_SERVICES, format!("{method}: {error}")))?;
    if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
        return Err(Failure::new(
            EXIT_SERVICES,
            format!(
                "{method}: {}",
                error["message"].as_str().unwrap_or("rpc error")
            ),
        ));
    }
    Ok(response["result"].clone())
}

fn stop_one(spec: ServiceSpec) -> Result<ServiceChange, Failure> {
    let plist = ensure_plist(spec)?;
    if !agent_loaded(spec.label) {
        if let Some(socket) = socket_path(spec) {
            if !agent_loaded(spec.label) && socket.exists() {
                eprintln!(
                    "warn: stale {} without loaded {}; removing",
                    socket.display(),
                    spec.label
                );
                let _ = fs::remove_file(&socket);
            }
        }
        return Ok(ServiceChange {
            spec,
            action: Action::Stop,
            changed: false,
            message: format!("{} was not loaded", spec.label),
        });
    }
    if spec.id == ServiceId::Net {
        eprintln!("stopping vz-net gracefully (vmnet ref release)…");
    }
    bootout(spec.label);
    if !wait_agent_gone(spec.label, spec.stop_wait_tenths) {
        if spec.id == ServiceId::Net {
            return Err(Failure::new(
                EXIT_SERVICES,
                format!(
                    "vz-net did not exit cleanly after bootout; refusing SIGKILL (would orphan CIDRs until reboot)"
                ),
            ));
        }
        return Err(Failure::new(
            EXIT_SERVICES,
            format!("{} still loaded after bootout", spec.label),
        ));
    }
    if spec.id == ServiceId::Net {
        if let Some(socket) = socket_path(spec) {
            if !wait_path_gone(&socket, 50) {
                eprintln!(
                    "warn: {} still present after vz-net exit; removing stale socket",
                    socket.display()
                );
                let _ = fs::remove_file(&socket);
            }
        }
        if Command::new("/usr/bin/pgrep")
            .args(["-x", "vz-net"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Err(Failure::new(
                EXIT_SERVICES,
                "vz-net process still running after launchd bootout; refuse stop to avoid orphaned CIDRs",
            ));
        }
        eprintln!("vz-net stopped cleanly");
    }
    let _ = plist;
    Ok(ServiceChange {
        spec,
        action: Action::Stop,
        changed: true,
        message: format!("stopped {}", spec.label),
    })
}

fn stop_target(target: ServiceId) -> Result<Vec<ServiceChange>, Failure> {
    ensure_plists_for(target)?;
    if matches!(target, ServiceId::All | ServiceId::Net) {
        stop_vm_helpers()?;
    }
    let mut changes = Vec::new();
    for spec in stop_order(target) {
        changes.push(stop_one(spec)?);
    }
    Ok(changes)
}

fn require_socket_ready(spec: ServiceSpec, dependency: &str) -> Result<(), Failure> {
    let Some(socket) = socket_path(spec) else {
        return Ok(());
    };
    if socket.exists() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_SERVICES,
            format!(
                "{} requires {}; start {} first",
                dependency,
                socket.display(),
                spec.id.as_str()
            ),
        ))
    }
}

fn start_one(spec: ServiceSpec) -> Result<ServiceChange, Failure> {
    let plist = ensure_plist(spec)?;
    if spec.id == ServiceId::Edge {
        require_socket_ready(SERVICE_NET, "vz-edge")?;
    }
    if spec.id == ServiceId::Supervisor {
        require_socket_ready(SERVICE_NET, "vz-supervisor")?;
        require_socket_ready(SERVICE_EDGE, "vz-supervisor")?;
    }
    let was_loaded = agent_loaded(spec.label);
    if !was_loaded {
        bootstrap(&plist)?;
    }
    kickstart(spec.label, spec.kickstart_kill)?;
    Command::new("launchctl")
        .arg("print")
        .arg(launchd_target(spec.label))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| Failure::new(EXIT_SERVICES, format!("launchctl print failed: {error}")))?
        .success()
        .then_some(())
        .ok_or_else(|| {
            Failure::new(
                EXIT_SERVICES,
                format!("{} is not loaded after start", spec.label),
            )
        })?;
    if let Some(socket) = socket_path(spec) {
        if !wait_socket(&socket, SOCKET_READY_TIMEOUT_TENTHS) {
            return Err(Failure::new(
                EXIT_SERVICES,
                format!(
                    "{} not ready after starting {}",
                    socket.display(),
                    spec.label
                ),
            ));
        }
    }
    Ok(ServiceChange {
        spec,
        action: Action::Start,
        changed: true,
        message: format!("started {}", spec.label),
    })
}

fn start_target(target: ServiceId) -> Result<Vec<ServiceChange>, Failure> {
    ensure_plists_for(target)?;
    let mut changes = Vec::new();
    for spec in start_order(target) {
        changes.push(start_one(spec)?);
    }
    Ok(changes)
}

fn restart_one(spec: ServiceSpec) -> Result<Vec<ServiceChange>, Failure> {
    let mut changes = Vec::new();
    if agent_loaded(spec.label) {
        kickstart(spec.label, true)?;
        if let Some(socket) = socket_path(spec) {
            if !wait_socket(&socket, SOCKET_READY_TIMEOUT_TENTHS) {
                return Err(Failure::new(
                    EXIT_SERVICES,
                    format!(
                        "{} not ready after restarting {}",
                        socket.display(),
                        spec.label
                    ),
                ));
            }
        }
        changes.push(ServiceChange {
            spec,
            action: Action::Restart,
            changed: true,
            message: format!("restarted {}", spec.label),
        });
    } else {
        changes.push(start_one(spec)?);
    }
    Ok(changes)
}

fn restart_target(target: ServiceId) -> Result<Vec<ServiceChange>, Failure> {
    ensure_plists_for(target)?;
    match target {
        ServiceId::All => {
            let mut changes = stop_target(ServiceId::All)?;
            for change in start_target(ServiceId::All)? {
                changes.push(ServiceChange {
                    spec: change.spec,
                    action: Action::Restart,
                    changed: change.changed,
                    message: change.message.replace("started", "restarted"),
                });
            }
            Ok(changes)
        }
        ServiceId::Net => {
            stop_vm_helpers()?;
            let mut changes = Vec::new();
            changes.push(stop_one(SERVICE_NET)?);
            changes.push(start_one(SERVICE_NET)?);
            for spec in [SERVICE_EDGE, SERVICE_SUPERVISOR] {
                changes.extend(restart_one(spec)?);
            }
            for change in &mut changes {
                change.action = Action::Restart;
            }
            Ok(changes)
        }
        ServiceId::Edge | ServiceId::Supervisor => {
            let spec = if target == ServiceId::Edge {
                SERVICE_EDGE
            } else {
                SERVICE_SUPERVISOR
            };
            restart_one(spec)
        }
    }
}

fn emit_status(format: Format, statuses: Vec<ServiceStatus>) -> ExitCode {
    match format {
        Format::Human => {
            println!(
                "{:<12} {:<22} {:<8} {:<12} {}",
                "SERVICE", "LABEL", "LOADED", "SOCKET", "PLIST"
            );
            for status in &statuses {
                let socket = status
                    .socket
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".into());
                let socket_state = match status.socket_ready {
                    Some(true) => "ready",
                    Some(false) => "missing",
                    None => "-",
                };
                println!(
                    "{:<12} {:<22} {:<8} {:<12} {}",
                    status.spec.id.as_str(),
                    status.spec.label,
                    if status.loaded { "yes" } else { "no" },
                    socket_state,
                    status.plist.display()
                );
                let _ = socket;
            }
            ExitCode::SUCCESS
        }
        Format::Json => {
            let services: Vec<Value> = statuses
                .iter()
                .map(|status| {
                    json!({
                        "id": status.spec.id.as_str(),
                        "label": status.spec.label,
                        "loaded": status.loaded,
                        "socket": status.socket,
                        "socket_ready": status.socket_ready,
                        "plist": status.plist,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "apiVersion": API_VERSION,
                    "command": "services.status",
                    "status": "ok",
                    "exit_code": 0,
                    "summary": { "count": services.len() },
                    "services": services,
                }))
                .unwrap()
            );
            ExitCode::SUCCESS
        }
    }
}

fn emit_changes(
    format: Format,
    action: Action,
    changes: Vec<ServiceChange>,
    exit_code: u8,
) -> ExitCode {
    match format {
        Format::Human => {
            for change in &changes {
                if change.changed || action == Action::Stop {
                    println!("{}", change.message);
                }
            }
            if action == Action::Restart
                && changes
                    .iter()
                    .any(|c| c.spec.id == ServiceId::Net && c.changed)
            {
                eprintln!("note: vz-edge and vz-supervisor were recycled after vz-net restart");
            }
            ExitCode::from(exit_code)
        }
        Format::Json => {
            let services: Vec<Value> = changes
                .iter()
                .map(|change| {
                    json!({
                        "id": change.spec.id.as_str(),
                        "label": change.spec.label,
                        "action": action.command().rsplit('.').next().unwrap_or(""),
                        "changed": change.changed,
                        "message": change.message,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "apiVersion": API_VERSION,
                    "command": action.command(),
                    "status": if exit_code == 0 { "ok" } else { "fail" },
                    "exit_code": exit_code,
                    "summary": {
                        "changed": changes.iter().filter(|c| c.changed).count(),
                    },
                    "services": services,
                }))
                .unwrap()
            );
            ExitCode::from(exit_code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_service_target() {
        assert_eq!(ServiceId::parse(Some("net")).unwrap(), ServiceId::Net);
        assert_eq!(ServiceId::parse(None).unwrap(), ServiceId::All);
        assert!(ServiceId::parse(Some("bogus")).is_err());
    }

    #[test]
    fn stop_order_all_stops_supervisor_before_net() {
        let order = stop_order(ServiceId::All);
        assert_eq!(order[0].id, ServiceId::Supervisor);
        assert_eq!(order[2].id, ServiceId::Net);
    }

    #[test]
    fn start_order_all_starts_net_before_supervisor() {
        let order = start_order(ServiceId::All);
        assert_eq!(order[0].id, ServiceId::Net);
        assert_eq!(order[2].id, ServiceId::Supervisor);
    }

    #[test]
    fn parse_options_defaults_to_all() {
        let options =
            parse_options("status", ["--format", "json"].into_iter().map(String::from)).unwrap();
        assert_eq!(options.target, ServiceId::All);
        assert_eq!(options.format, Format::Json);
        assert_eq!(options.action, Action::Status);
    }
}
