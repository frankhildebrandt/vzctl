use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const API_VERSION: &str = "vzctl.dev/v1";
const DEFAULT_CONFIG: &str = "hypernetwork.config.yaml";
const DEFAULT_DNS_PORT: u16 = 15353;
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_SUPERVISOR: u8 = 10;
pub(crate) const EXIT_RESOLVER: u8 = 19;
pub(crate) const EXIT_DNS_QUERY: u8 = 20;
pub(crate) const BIND_HELPER_LABEL: &str = "com.vzctl.dns-bind";
pub(crate) const BIND_HELPER_LIBEXEC: &str = "/usr/local/libexec/vzctl/vz-dns-bind";
pub(crate) const BIND_HELPER_MARKER: &str = "/usr/local/libexec/vzctl/dns-bind.managed";
pub(crate) const BIND_HELPER_PLIST: &str = "/Library/LaunchDaemons/com.vzctl.dns-bind.plist";
pub(crate) const BIND_HELPER_SOCKET_DEFAULT: &str = "/var/run/vzctl/dns-bind.sock";

const MANAGED_MARKER: &str = "# managed-by: vzctl";
const REVERSE_RESOLVER_DOMAIN: &str = "in-addr.arpa";
const REVERSE_SCOPE_MARKER: &str = "# scope: ipv4-reverse";
const DEFAULT_DNS_SERVER: &str = "127.0.0.1:15353";
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
const DNS_HEADER_LENGTH: usize = 12;
const BIND_HELPER_SOCKET: &str = BIND_HELPER_SOCKET_DEFAULT;
const BIND_HELPER_LOG_DIR: &str = "/Library/Logs/vzctl";
const BIND_HELPER_PLIST_TEMPLATE: &str =
    include_str!("../../../daemon/launchd/com.vzctl.dns-bind.plist.template");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Install,
    Uninstall,
    InstallBindHelper,
    UninstallBindHelper,
    Query,
    Status,
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Install => "dns.install-resolver",
            Self::Uninstall => "dns.uninstall-resolver",
            Self::InstallBindHelper => "dns.install-bind-helper",
            Self::UninstallBindHelper => "dns.uninstall-bind-helper",
            Self::Query => "dns.query",
            Self::Status => "dns.status",
        }
    }

    fn is_bind_helper(self) -> bool {
        matches!(self, Self::InstallBindHelper | Self::UninstallBindHelper)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueryType {
    A,
    Aaaa,
    Ptr,
}

impl QueryType {
    fn code(self) -> u16 {
        match self {
            Self::A => 1,
            Self::Aaaa => 28,
            Self::Ptr => 12,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Ptr => "PTR",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    action: Action,
    project: Option<String>,
    config: PathBuf,
    config_explicit: bool,
    format: Format,
    query_name: Option<String>,
    query_type: QueryType,
    server: String,
    allow_uid: Option<u32>,
}

#[derive(Debug, Eq, PartialEq)]
struct Scope {
    project: String,
    owner: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Change {
    Installed,
    Updated,
    Unchanged,
    Removed,
    Absent,
}

impl Change {
    fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
            Self::Removed => "removed",
            Self::Absent => "absent",
        }
    }
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
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let command_hint = match args.first().map(String::as_str) {
        Some("query") => "dns.query",
        Some("status") => "dns.status",
        Some("install-bind-helper") => "dns.install-bind-helper",
        Some("uninstall-bind-helper") => "dns.uninstall-bind-helper",
        _ => "dns",
    };
    let options = match parse(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, command_hint, &failure);
            return ExitCode::from(failure.code);
        }
    };
    if options.action == Action::Query {
        return query_command(&options);
    }
    if options.action == Action::Status {
        return status_command(&options, socket_path);
    }
    if options.action.is_bind_helper() {
        let result = match options.action {
            Action::InstallBindHelper => install_bind_helper(options.allow_uid),
            Action::UninstallBindHelper => uninstall_bind_helper(),
            _ => unreachable!(),
        };
        return match result {
            Ok((path, change)) => {
                emit_bind_helper_success(options.format, options.action, &path, change);
                ExitCode::SUCCESS
            }
            Err(failure) => {
                emit_failure(options.format, options.action.command(), &failure);
                ExitCode::from(failure.code)
            }
        };
    }
    let scope = match resolve_scope(&options) {
        Ok(scope) => scope,
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            return ExitCode::from(failure.code);
        }
    };
    let resolver_dir = std::env::var_os("VZCTL_RESOLVER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/resolver"));
    let port = match dns_port() {
        Ok(port) => port,
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            return ExitCode::from(failure.code);
        }
    };
    let result = match options.action {
        Action::Install => install(&resolver_dir, &scope, port),
        Action::Uninstall => uninstall(&resolver_dir, &scope),
        Action::InstallBindHelper
        | Action::UninstallBindHelper
        | Action::Query
        | Action::Status => unreachable!("handled above"),
    };
    match result {
        Ok((path, change)) => {
            emit_success(options.format, options.action, &scope, &path, port, change);
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let action = match args.next().as_deref() {
        Some("install-resolver") => Action::Install,
        Some("uninstall-resolver") => Action::Uninstall,
        Some("install-bind-helper") => Action::InstallBindHelper,
        Some("uninstall-bind-helper") => Action::UninstallBindHelper,
        Some("query") => Action::Query,
        Some("status") => Action::Status,
        _ => return Err(Failure::new(EXIT_USAGE, usage())),
    };
    let mut project = None;
    let mut config = PathBuf::from(DEFAULT_CONFIG);
    let mut config_explicit = false;
    let mut format = Format::Human;
    let mut query_name = None;
    let mut query_type = QueryType::A;
    let mut server = DEFAULT_DNS_SERVER.to_string();
    let mut allow_uid = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--project" => {
                if matches!(action, Action::Query | Action::Status) || action.is_bind_helper() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "--project is not valid for this dns command",
                    ));
                }
                let value = next_value(&mut args, "--project requires a project")?;
                if project.replace(value).is_some() {
                    return Err(Failure::new(EXIT_USAGE, "--project may only be used once"));
                }
            }
            "--config" => {
                if matches!(action, Action::Query | Action::Status) || action.is_bind_helper() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "--config is not valid for this dns command",
                    ));
                }
                let value = next_value(&mut args, "--config requires a path")?;
                if config_explicit {
                    return Err(Failure::new(EXIT_USAGE, "--config may only be used once"));
                }
                config = PathBuf::from(value);
                config_explicit = true;
            }
            "--format" => {
                let value = next_value(&mut args, "--format requires human or json")?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    _ => {
                        return Err(Failure::new(
                            EXIT_USAGE,
                            format!("unsupported dns format: {value}"),
                        ))
                    }
                };
            }
            "--allow-uid" => {
                if action != Action::InstallBindHelper {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "--allow-uid is only valid for dns install-bind-helper",
                    ));
                }
                let value = next_value(&mut args, "--allow-uid requires a numeric uid")?;
                let parsed = value.parse::<u32>().map_err(|_| {
                    Failure::new(EXIT_INVALID, format!("invalid --allow-uid: {value}"))
                })?;
                if allow_uid.replace(parsed).is_some() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "--allow-uid may only be used once",
                    ));
                }
            }
            "--type" => {
                if action != Action::Query {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "--type is only valid for dns query",
                    ));
                }
                let value = next_value(&mut args, "--type requires A, AAAA, or PTR")?;
                query_type = match value.to_ascii_uppercase().as_str() {
                    "A" => QueryType::A,
                    "AAAA" => QueryType::Aaaa,
                    "PTR" => QueryType::Ptr,
                    _ => {
                        return Err(Failure::new(
                            EXIT_INVALID,
                            format!("unsupported DNS query type: {value}"),
                        ))
                    }
                };
            }
            "--server" => {
                if action != Action::Query {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "--server is only valid for dns query",
                    ));
                }
                server = next_value(&mut args, "--server requires an IP:port")?;
            }
            "-h" | "--help" => return Err(Failure::new(EXIT_USAGE, usage())),
            _ if action == Action::Query && !argument.starts_with('-') => {
                if query_name.replace(argument).is_some() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "dns query accepts exactly one name",
                    ));
                }
            }
            _ => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown dns option: {argument}"),
                ))
            }
        }
    }
    if action == Action::Query && query_name.is_none() {
        return Err(Failure::new(EXIT_USAGE, "dns query requires a name"));
    }
    Ok(Options {
        action,
        project,
        config,
        config_explicit,
        format,
        query_name,
        query_type,
        server,
        allow_uid,
    })
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    message: &'static str,
) -> Result<String, Failure> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::new(EXIT_USAGE, message))
}

fn resolve_scope(options: &Options) -> Result<Scope, Failure> {
    let config = crate::config::config_path(&options.config);
    let config_exists = config.is_file();
    if options.config_explicit && !config_exists {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("config does not exist: {}", options.config.display()),
        ));
    }
    let config_project = if config_exists {
        Some(project_from_config(&config)?)
    } else {
        None
    };
    if let (Some(explicit), Some(configured)) = (&options.project, &config_project) {
        if explicit != configured {
            return Err(Failure::new(
                EXIT_INVALID,
                format!(
                    "--project {explicit} does not match spec.project {configured} in {}",
                    config.display()
                ),
            ));
        }
    }
    let project = options.project.clone().or(config_project).ok_or_else(|| {
        Failure::new(
            EXIT_INVALID,
            format!("no --project and no {DEFAULT_CONFIG}"),
        )
    })?;
    validate_project(&project)?;
    let owner = if config_exists {
        let canonical = fs::canonicalize(&config).map_err(|error| {
            Failure::new(
                EXIT_INVALID,
                format!("cannot resolve config {}: {error}", config.display()),
            )
        })?;
        format!(
            "config-{:016x}",
            stable_hash(canonical.as_os_str().as_encoded_bytes())
        )
    } else {
        format!("project-{project}")
    };
    Ok(Scope { project, owner })
}

fn project_from_config(path: &Path) -> Result<String, Failure> {
    let text = fs::read_to_string(path).map_err(|error| {
        Failure::new(
            EXIT_INVALID,
            format!("cannot read config {}: {error}", path.display()),
        )
    })?;
    let document: YamlValue = serde_yaml::from_str(&text).map_err(|error| {
        Failure::new(
            EXIT_INVALID,
            format!("cannot parse config {}: {error}", path.display()),
        )
    })?;
    document
        .get("spec")
        .and_then(|spec| spec.get("project"))
        .and_then(YamlValue::as_str)
        .filter(|project| !project.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            Failure::new(
                EXIT_INVALID,
                format!("config {} has no spec.project", path.display()),
            )
        })
}

fn validate_project(project: &str) -> Result<(), Failure> {
    let valid = project.len() <= 63
        && project
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && project
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && project
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);
    if valid {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_INVALID,
            "project must be a lowercase DNS label (a-z, 0-9, hyphen; max 63 characters)",
        ))
    }
}

fn dns_port() -> Result<u16, Failure> {
    match std::env::var("VZCTL_DNS_PORT") {
        Ok(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| Failure::new(EXIT_INVALID, format!("invalid VZCTL_DNS_PORT: {value}"))),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_DNS_PORT),
        Err(error) => Err(Failure::new(
            EXIT_INVALID,
            format!("invalid VZCTL_DNS_PORT: {error}"),
        )),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct DnsAnswer {
    name: String,
    record_type: String,
    class: String,
    ttl: u32,
    data: String,
}

#[derive(Debug, Eq, PartialEq)]
struct QueryResponse {
    name: String,
    query_type: QueryType,
    server: String,
    rcode: u8,
    authoritative: bool,
    truncated: bool,
    answers: Vec<DnsAnswer>,
}

fn query_command(options: &Options) -> ExitCode {
    let result = execute_query(
        options
            .query_name
            .as_deref()
            .expect("query name was parsed"),
        options.query_type,
        &options.server,
    );
    match result {
        Ok(response) => {
            let exit_code = if response.rcode == 0 && !response.truncated {
                0
            } else {
                EXIT_DNS_QUERY
            };
            emit_query(options.format, &response, exit_code);
            ExitCode::from(exit_code)
        }
        Err(failure) => {
            emit_query_failure(options.format, options, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn status_command(options: &Options, socket_path: &Path) -> ExitCode {
    match dns_status(socket_path) {
        Ok(dns) => {
            let ok = dns["ok"].as_bool().unwrap_or(false);
            let exit_code = if ok { 0 } else { EXIT_SUPERVISOR };
            let listeners = dns["listeners"]
                .as_array()
                .map(|values| values.len())
                .unwrap_or(0);
            let records = dns["records"].as_u64().unwrap_or(0);
            let message = if ok {
                format!("dns ok: {listeners} listener(s), {records} record(s)")
            } else {
                format!(
                    "dns not ok: {}",
                    dns["last_error"]
                        .as_str()
                        .unwrap_or("listeners or zone unhealthy")
                )
            };
            let sections = crate::observability::dns_sections(&dns);
            let envelope = json!({
                "apiVersion": API_VERSION,
                "command": "dns.status",
                "status": if ok { "ok" } else { "fail" },
                "exit_code": exit_code,
                "summary": {
                    "message": message,
                    "ok": ok,
                },
                "dns": dns,
                "host_resolver": sections["host_resolver"],
                "bridge_dns": sections["bridge_dns"],
                "upstream": sections["upstream"],
                "last_probe": sections["last_probe"],
            });
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human => {
                    println!("{message}");
                    println!(
                        "  host_resolver: {}",
                        if sections["host_resolver"]["ok"] == true {
                            "OK"
                        } else {
                            "FAIL"
                        }
                    );
                    println!(
                        "  bridge_dns: {}",
                        if sections["bridge_dns"]["ok"] == true {
                            "OK"
                        } else {
                            "FAIL"
                        }
                    );
                    println!("  upstream: {}", sections["upstream"]["name"]);
                    if let Some(listeners) = dns["listeners"].as_array() {
                        for listener in listeners {
                            if let Some(value) = listener.as_str() {
                                println!("  listener: {value}");
                            }
                        }
                    }
                    if let Some(error) = dns["last_error"].as_str() {
                        eprintln!("  last_error: {error}");
                    }
                }
            }
            ExitCode::from(exit_code)
        }
        Err(failure) => {
            emit_failure(options.format, "dns.status", &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn dns_status(socket_path: &Path) -> Result<Value, Failure> {
    rpc(socket_path, "dns.status", json!({}))
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("supervisor socket {}: {error}", socket_path.display()),
        )
    })?;
    let timeout = Some(Duration::from_secs(5));
    stream
        .set_read_timeout(timeout)
        .and_then(|_| stream.set_write_timeout(timeout))
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
        return Err(Failure::new(
            EXIT_SUPERVISOR,
            error["message"]
                .as_str()
                .unwrap_or("supervisor rpc error")
                .to_string(),
        ));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "supervisor response has no result"))
}

/// Resolve A records via the host listener. Empty vec means NXDOMAIN/no answers.
pub(crate) fn lookup_a_addresses(name: &str) -> Result<Vec<String>, String> {
    match execute_query(name, QueryType::A, DEFAULT_DNS_SERVER) {
        Ok(response) if response.rcode == 0 && !response.truncated => Ok(response
            .answers
            .into_iter()
            .filter(|answer| answer.record_type == "A")
            .map(|answer| answer.data)
            .collect()),
        Ok(_) => Err(format!("host DNS lookup for {name} failed")),
        Err(failure) => Err(failure.message),
    }
}

fn execute_query(
    name: &str,
    query_type: QueryType,
    server: &str,
) -> Result<QueryResponse, Failure> {
    let canonical_name = validate_query_name(name)?;
    let server_address = server.parse::<SocketAddr>().map_err(|_| {
        Failure::new(
            EXIT_INVALID,
            format!("invalid DNS server {server}; expected IP:port"),
        )
    })?;
    if server_address.port() == 0 {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("invalid DNS server {server}; port must be greater than zero"),
        ));
    }
    let transaction_id = transaction_id();
    let request = build_query(transaction_id, &canonical_name, query_type);
    let bind_address = match server_address.ip() {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    };
    let socket = UdpSocket::bind(bind_address)
        .map_err(|error| dns_query_failure(server, "create UDP socket", error))?;
    socket
        .set_read_timeout(Some(DNS_TIMEOUT))
        .map_err(|error| dns_query_failure(server, "set UDP timeout", error))?;
    socket
        .connect(server_address)
        .map_err(|error| dns_query_failure(server, "connect UDP socket", error))?;
    socket
        .send(&request)
        .map_err(|error| dns_query_failure(server, "send UDP query", error))?;

    let mut buffer = [0_u8; u16::MAX as usize];
    let count = socket
        .recv(&mut buffer)
        .map_err(|error| dns_query_failure(server, "receive UDP response", error))?;
    parse_response(
        &buffer[..count],
        transaction_id,
        &canonical_name,
        query_type,
        server,
    )
}

fn validate_query_name(name: &str) -> Result<String, Failure> {
    let canonical = name.trim_end_matches('.').to_ascii_lowercase();
    let valid = !canonical.is_empty()
        && canonical.len() <= 253
        && canonical.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        });
    if valid {
        Ok(canonical)
    } else {
        Err(Failure::new(
            EXIT_INVALID,
            "DNS name must contain non-empty ASCII labels of at most 63 characters",
        ))
    }
}

fn transaction_id() -> u16 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as u16) ^ (std::process::id() as u16)
}

fn build_query(transaction_id: u16, name: &str, query_type: QueryType) -> Vec<u8> {
    let mut packet = Vec::with_capacity(DNS_HEADER_LENGTH + name.len() + 6);
    append_u16(&mut packet, transaction_id);
    append_u16(&mut packet, 0x0100);
    append_u16(&mut packet, 1);
    append_u16(&mut packet, 0);
    append_u16(&mut packet, 0);
    append_u16(&mut packet, 0);
    for label in name.split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    append_u16(&mut packet, query_type.code());
    append_u16(&mut packet, 1);
    packet
}

fn parse_response(
    packet: &[u8],
    transaction_id: u16,
    query_name: &str,
    query_type: QueryType,
    server: &str,
) -> Result<QueryResponse, Failure> {
    if packet.len() < DNS_HEADER_LENGTH {
        return Err(protocol_failure(
            server,
            "response is shorter than DNS header",
        ));
    }
    if read_u16(packet, 0) != Some(transaction_id) {
        return Err(protocol_failure(
            server,
            "response transaction ID does not match",
        ));
    }
    let flags = read_u16(packet, 2).expect("header length checked");
    if flags & 0x8000 == 0 {
        return Err(protocol_failure(server, "packet is not a DNS response"));
    }
    let question_count = read_u16(packet, 4).expect("header length checked") as usize;
    let answer_count = read_u16(packet, 6).expect("header length checked") as usize;
    if question_count != 1 {
        return Err(protocol_failure(
            server,
            "response must contain exactly one question",
        ));
    }
    let mut offset = DNS_HEADER_LENGTH;
    let response_name = decode_name(packet, &mut offset)?;
    let response_type = read_u16(packet, offset)
        .ok_or_else(|| protocol_failure(server, "truncated response question type"))?;
    let response_class = read_u16(packet, offset + 2)
        .ok_or_else(|| protocol_failure(server, "truncated response question class"))?;
    checked_advance(packet, &mut offset, 4, server)?;
    if response_name.to_ascii_lowercase() != query_name
        || response_type != query_type.code()
        || response_class != 1
    {
        return Err(protocol_failure(
            server,
            "response question does not match query",
        ));
    }

    let mut answers = Vec::with_capacity(answer_count);
    for _ in 0..answer_count {
        let name = decode_name(packet, &mut offset)?;
        let record_type = read_u16(packet, offset)
            .ok_or_else(|| protocol_failure(server, "truncated answer type"))?;
        let class = read_u16(packet, offset + 2)
            .ok_or_else(|| protocol_failure(server, "truncated answer class"))?;
        let ttl = read_u32(packet, offset + 4)
            .ok_or_else(|| protocol_failure(server, "truncated answer TTL"))?;
        let data_length = read_u16(packet, offset + 8)
            .ok_or_else(|| protocol_failure(server, "truncated answer length"))?
            as usize;
        checked_advance(packet, &mut offset, 10, server)?;
        let data_offset = offset;
        checked_advance(packet, &mut offset, data_length, server)?;
        let data = decode_record_data(packet, data_offset, data_length, record_type, server)?;
        answers.push(DnsAnswer {
            name,
            record_type: record_type_name(record_type),
            class: if class == 1 {
                "IN".to_string()
            } else {
                class.to_string()
            },
            ttl,
            data,
        });
    }

    Ok(QueryResponse {
        name: query_name.to_string(),
        query_type,
        server: server.to_string(),
        rcode: (flags & 0x000f) as u8,
        authoritative: flags & 0x0400 != 0,
        truncated: flags & 0x0200 != 0,
        answers,
    })
}

fn decode_name(packet: &[u8], offset: &mut usize) -> Result<String, Failure> {
    let mut labels = Vec::new();
    let mut cursor = *offset;
    let mut jumped = false;
    let mut jumps = 0;
    loop {
        let length = *packet
            .get(cursor)
            .ok_or_else(|| protocol_failure("DNS response", "truncated name"))?;
        if length == 0 {
            if !jumped {
                *offset = cursor + 1;
            }
            return Ok(labels.join("."));
        }
        if length & 0xc0 == 0xc0 {
            let next = *packet
                .get(cursor + 1)
                .ok_or_else(|| protocol_failure("DNS response", "truncated name pointer"))?;
            if !jumped {
                *offset = cursor + 2;
            }
            cursor = (usize::from(length & 0x3f) << 8) | usize::from(next);
            jumped = true;
            jumps += 1;
            if jumps > packet.len() {
                return Err(protocol_failure("DNS response", "name pointer loop"));
            }
            continue;
        }
        if length & 0xc0 != 0 || length > 63 {
            return Err(protocol_failure("DNS response", "invalid name label"));
        }
        let start = cursor + 1;
        let end = start + usize::from(length);
        let label = packet
            .get(start..end)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| protocol_failure("DNS response", "invalid name label data"))?;
        labels.push(label.to_string());
        cursor = end;
        if !jumped {
            *offset = cursor;
        }
    }
}

fn decode_record_data(
    packet: &[u8],
    offset: usize,
    length: usize,
    record_type: u16,
    server: &str,
) -> Result<String, Failure> {
    let bytes = packet
        .get(offset..offset + length)
        .ok_or_else(|| protocol_failure(server, "truncated answer data"))?;
    match (record_type, length) {
        (1, 4) => Ok(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string()),
        (28, 16) => {
            let octets: [u8; 16] = bytes
                .try_into()
                .map_err(|_| protocol_failure(server, "invalid AAAA answer"))?;
            Ok(Ipv6Addr::from(octets).to_string())
        }
        (2 | 5 | 12, _) => {
            let mut name_offset = offset;
            decode_name(packet, &mut name_offset)
        }
        _ => Ok(bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join("")),
    }
}

fn checked_advance(
    packet: &[u8],
    offset: &mut usize,
    count: usize,
    server: &str,
) -> Result<(), Failure> {
    let end = offset
        .checked_add(count)
        .filter(|end| *end <= packet.len())
        .ok_or_else(|| protocol_failure(server, "truncated DNS response"))?;
    *offset = end;
    Ok(())
}

fn read_u16(packet: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        packet.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(packet: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        packet.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn append_u16(packet: &mut Vec<u8>, value: u16) {
    packet.extend_from_slice(&value.to_be_bytes());
}

fn record_type_name(record_type: u16) -> String {
    match record_type {
        1 => "A".to_string(),
        2 => "NS".to_string(),
        5 => "CNAME".to_string(),
        12 => "PTR".to_string(),
        28 => "AAAA".to_string(),
        value => format!("TYPE{value}"),
    }
}

fn rcode_name(rcode: u8) -> &'static str {
    match rcode {
        0 => "NOERROR",
        1 => "FORMERR",
        2 => "SERVFAIL",
        3 => "NXDOMAIN",
        4 => "NOTIMP",
        5 => "REFUSED",
        _ => "UNKNOWN",
    }
}

fn dns_query_failure(server: &str, operation: &str, error: io::Error) -> Failure {
    Failure::new(EXIT_DNS_QUERY, format!("{operation} via {server}: {error}"))
}

fn protocol_failure(server: &str, message: &str) -> Failure {
    Failure::new(
        EXIT_DNS_QUERY,
        format!("invalid DNS response from {server}: {message}"),
    )
}

fn install_bind_helper(explicit_uid: Option<u32>) -> Result<(PathBuf, Change), Failure> {
    let allow_uid = match explicit_uid {
        Some(uid) => uid,
        None => allow_uid()?,
    };
    if allow_uid == 0 {
        return Err(Failure::new(
            EXIT_INVALID,
            "allow-uid must be a non-root user uid (pass --allow-uid or run via sudo from your account)",
        ));
    }
    let source = bind_helper_source_binary()?;
    let libexec_dir = Path::new(BIND_HELPER_LIBEXEC)
        .parent()
        .ok_or_else(|| Failure::new(EXIT_RESOLVER, "invalid bind-helper libexec path"))?;
    fs::create_dir_all(libexec_dir)
        .map_err(|error| io_failure("create directory", libexec_dir, error))?;
    fs::set_permissions(libexec_dir, fs::Permissions::from_mode(0o755))
        .map_err(|error| io_failure("set permissions on", libexec_dir, error))?;

    let binary = PathBuf::from(BIND_HELPER_LIBEXEC);
    let marker = PathBuf::from(BIND_HELPER_MARKER);
    let plist = PathBuf::from(BIND_HELPER_PLIST);
    let desired_marker = bind_helper_marker_content(allow_uid);
    let desired_plist = bind_helper_plist_content(allow_uid)?;

    let change = match read_existing(&marker)? {
        None => Change::Installed,
        Some(existing) => {
            ensure_bind_helper_owned(&marker, &existing)?;
            let binary_same = binary.is_file()
                && fs::read(&binary).ok().as_deref() == fs::read(&source).ok().as_deref();
            let plist_same = read_existing(&plist)?.as_deref() == Some(desired_plist.as_str());
            if existing == desired_marker && binary_same && plist_same {
                launchctl_kickstart_bind_helper()?;
                return Ok((plist, Change::Unchanged));
            }
            Change::Updated
        }
    };

    fs::copy(&source, &binary).map_err(|error| io_failure("install", &binary, error))?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
        .map_err(|error| io_failure("set permissions on", &binary, error))?;
    atomic_write(&marker, desired_marker.as_bytes())
        .map_err(|error| io_failure("write", &marker, error))?;
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o644))
        .map_err(|error| io_failure("set permissions on", &marker, error))?;

    let log_dir = Path::new(BIND_HELPER_LOG_DIR);
    fs::create_dir_all(log_dir).map_err(|error| io_failure("create directory", log_dir, error))?;
    atomic_write(&plist, desired_plist.as_bytes())
        .map_err(|error| io_failure("write", &plist, error))?;
    fs::set_permissions(&plist, fs::Permissions::from_mode(0o644))
        .map_err(|error| io_failure("set permissions on", &plist, error))?;

    launchctl_bootout_bind_helper();
    launchctl_bootstrap_bind_helper(&plist)?;
    launchctl_kickstart_bind_helper()?;
    Ok((plist, change))
}

fn uninstall_bind_helper() -> Result<(PathBuf, Change), Failure> {
    let plist = PathBuf::from(BIND_HELPER_PLIST);
    let marker = PathBuf::from(BIND_HELPER_MARKER);
    let binary = PathBuf::from(BIND_HELPER_LIBEXEC);
    match read_existing(&marker)? {
        None if !plist.exists() && !binary.exists() => Ok((plist, Change::Absent)),
        None => Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "bind-helper marker missing at {}; refusing to remove unmanaged files",
                marker.display()
            ),
        )),
        Some(existing) => {
            ensure_bind_helper_owned(&marker, &existing)?;
            launchctl_bootout_bind_helper();
            let cleanup = std::process::Command::new(&binary)
                .arg("cleanup")
                .status()
                .map_err(|error| {
                    Failure::new(
                        EXIT_RESOLVER,
                        format!("bind-helper network cleanup failed: {error}"),
                    )
                })?;
            if !cleanup.success() {
                return Err(Failure::new(
                    EXIT_RESOLVER,
                    "bind-helper network cleanup failed; aliases and PF anchor were not confirmed removed",
                ));
            }
            if plist.exists() {
                fs::remove_file(&plist).map_err(|error| io_failure("remove", &plist, error))?;
            }
            if binary.exists() {
                fs::remove_file(&binary).map_err(|error| io_failure("remove", &binary, error))?;
            }
            fs::remove_file(&marker).map_err(|error| io_failure("remove", &marker, error))?;
            let _ = fs::remove_file(BIND_HELPER_SOCKET);
            Ok((plist, Change::Removed))
        }
    }
}

fn allow_uid() -> Result<u32, Failure> {
    if let Ok(value) = std::env::var("SUDO_UID") {
        return value
            .parse::<u32>()
            .map_err(|_| Failure::new(EXIT_INVALID, format!("invalid SUDO_UID: {value}")));
    }
    if let Ok(value) = std::env::var("VZCTL_DNS_BIND_ALLOW_UID") {
        return value.parse::<u32>().map_err(|_| {
            Failure::new(
                EXIT_INVALID,
                format!("invalid VZCTL_DNS_BIND_ALLOW_UID: {value}"),
            )
        });
    }
    let uid = unsafe { libc::getuid() };
    if uid == 0 {
        return Err(Failure::new(
            EXIT_INVALID,
            "cannot infer allow-uid as root; pass --allow-uid <uid> or run via sudo from your user",
        ));
    }
    Ok(uid)
}

fn bind_helper_source_binary() -> Result<PathBuf, Failure> {
    if let Some(path) = std::env::var_os("VZCTL_DNS_BIND_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(Failure::new(
            EXIT_INVALID,
            format!("VZCTL_DNS_BIND_BIN is not a file: {}", path.display()),
        ));
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("vz-dns-bind"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin/vz-dns-bind"));
    }
    candidates.push(PathBuf::from("daemon/.build/release/vz-dns-bind"));
    candidates.push(PathBuf::from("daemon/.build/debug/vz-dns-bind"));
    for path in candidates {
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(Failure::new(
        EXIT_INVALID,
        "vz-dns-bind binary not found; run make release or set VZCTL_DNS_BIND_BIN",
    ))
}

fn bind_helper_marker_content(allow_uid: u32) -> String {
    format!("{MANAGED_MARKER}\n# allow-uid: {allow_uid}\n")
}

fn bind_helper_plist_content(allow_uid: u32) -> Result<String, Failure> {
    let log = format!("{BIND_HELPER_LOG_DIR}/dns-bind.log");
    let err = format!("{BIND_HELPER_LOG_DIR}/dns-bind.error.log");
    Ok(BIND_HELPER_PLIST_TEMPLATE
        .replace("__BINARY_PATH__", BIND_HELPER_LIBEXEC)
        .replace("__ALLOW_UID__", &allow_uid.to_string())
        .replace("__SOCKET_PATH__", BIND_HELPER_SOCKET)
        .replace("__LOG_PATH__", &log)
        .replace("__ERROR_LOG_PATH__", &err))
}

fn ensure_bind_helper_owned(path: &Path, content: &str) -> Result<(), Failure> {
    if content.lines().any(|line| line == MANAGED_MARKER)
        && content
            .lines()
            .any(|line| line.starts_with("# allow-uid: "))
    {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "bind-helper collision at {}; file is not managed by vzctl",
                path.display()
            ),
        ))
    }
}

fn launchctl_bootout_bind_helper() {
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &format!("system/{BIND_HELPER_LABEL}")])
        .status();
}

fn launchctl_bootstrap_bind_helper(plist: &Path) -> Result<(), Failure> {
    let status = std::process::Command::new("launchctl")
        .args(["bootstrap", "system", &plist.display().to_string()])
        .status()
        .map_err(|error| {
            Failure::new(
                EXIT_RESOLVER,
                format!("launchctl bootstrap failed: {error}; run this command with sudo"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_RESOLVER,
            "launchctl bootstrap failed; run this command with sudo",
        ))
    }
}

fn launchctl_kickstart_bind_helper() -> Result<(), Failure> {
    let status = std::process::Command::new("launchctl")
        .args(["kickstart", "-k", &format!("system/{BIND_HELPER_LABEL}")])
        .status()
        .map_err(|error| {
            Failure::new(
                EXIT_RESOLVER,
                format!("launchctl kickstart failed: {error}"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_RESOLVER,
            "launchctl kickstart failed; run this command with sudo",
        ))
    }
}

fn emit_bind_helper_success(format: Format, action: Action, path: &Path, change: Change) {
    let message = format!("bind-helper {}: {}", change.as_str(), path.display());
    match format {
        Format::Human => println!("{message}"),
        Format::Json => println!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": action.command(),
                "status": "ok",
                "exit_code": 0,
                "summary": {
                    "message": message,
                    "change": change.as_str(),
                },
                "bind_helper": {
                    "label": BIND_HELPER_LABEL,
                    "plist": path,
                    "binary": BIND_HELPER_LIBEXEC,
                    "socket": BIND_HELPER_SOCKET,
                    "managed": true,
                }
            })
        ),
    }
}

fn install(resolver_dir: &Path, scope: &Scope, port: u16) -> Result<(PathBuf, Change), Failure> {
    ensure_resolver_dir(resolver_dir)?;
    let path = resolver_path(resolver_dir, &scope.project);
    let desired = resolver_content(scope, port);
    let forward_change = match read_existing(&path)? {
        None => Change::Installed,
        Some(existing) => {
            ensure_owned(&path, &existing, scope)?;
            if existing == desired {
                Change::Unchanged
            } else {
                Change::Updated
            }
        }
    };

    let reverse_path = reverse_resolver_path(resolver_dir);
    let reverse_desired = reverse_resolver_content(port);
    let reverse_change = match read_existing(&reverse_path)? {
        None => Change::Installed,
        Some(existing) => {
            ensure_reverse_owned(&reverse_path, &existing)?;
            if existing == reverse_desired {
                Change::Unchanged
            } else {
                Change::Updated
            }
        }
    };

    // Publish the shared reverse scope first. An extra forwarding scope is safe if
    // publishing the project scope fails afterwards; the inverse would make host
    // PTR lookups silently bypass vz-edge.
    if reverse_change != Change::Unchanged {
        atomic_write(&reverse_path, reverse_desired.as_bytes())
            .map_err(|error| io_failure("write", &reverse_path, error))?;
    }
    if forward_change != Change::Unchanged {
        atomic_write(&path, desired.as_bytes())
            .map_err(|error| io_failure("write", &path, error))?;
    }
    let change = if forward_change == Change::Unchanged && reverse_change != Change::Unchanged {
        Change::Updated
    } else {
        forward_change
    };
    Ok((path, change))
}

fn uninstall(resolver_dir: &Path, scope: &Scope) -> Result<(PathBuf, Change), Failure> {
    let path = resolver_path(resolver_dir, &scope.project);
    let existing = read_existing(&path)?;
    if let Some(content) = existing.as_deref() {
        ensure_owned(&path, content, scope)?;
    }

    let has_other_project = has_other_managed_resolver(resolver_dir, &path)?;
    let reverse_path = reverse_resolver_path(resolver_dir);
    let reverse = if has_other_project {
        None
    } else {
        read_existing(&reverse_path)?
    };
    if let Some(content) = reverse.as_deref() {
        ensure_reverse_owned(&reverse_path, content)?;
    }

    let change = if existing.is_some() {
        remove_regular_file(&path)?;
        Change::Removed
    } else {
        Change::Absent
    };
    if reverse.is_some() {
        remove_regular_file(&reverse_path)?;
    }
    Ok((path, change))
}

fn remove_regular_file(path: &Path) -> Result<(), Failure> {
    let before = fs::symlink_metadata(path).map_err(|error| io_failure("inspect", path, error))?;
    let after = fs::symlink_metadata(path).map_err(|error| io_failure("inspect", path, error))?;
    if before.dev() != after.dev() || before.ino() != after.ino() || !after.file_type().is_file() {
        return Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver changed during cleanup; refusing to remove {}",
                path.display()
            ),
        ));
    }
    fs::remove_file(path).map_err(|error| io_failure("remove", path, error))
}

fn ensure_resolver_dir(path: &Path) -> Result<(), Failure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver directory must not be a symlink: {}",
                path.display()
            ),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(Failure::new(
            EXIT_RESOLVER,
            format!("resolver path is not a directory: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| io_failure("create directory", path, error))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|error| io_failure("set permissions on", path, error))
        }
        Err(error) => Err(io_failure("inspect", path, error)),
    }
}

fn read_existing(path: &Path) -> Result<Option<String>, Failure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_failure("inspect", path, error)),
    };
    if !metadata.file_type().is_file() {
        return Err(Failure::new(
            EXIT_RESOLVER,
            format!("resolver target is not a regular file: {}", path.display()),
        ));
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|error| io_failure("read", path, error))
}

fn ensure_owned(path: &Path, content: &str, scope: &Scope) -> Result<(), Failure> {
    let project_marker = format!("# project: {}", scope.project);
    let owner_marker = format!("# owner: {}", scope.owner);
    if content.lines().any(|line| line == MANAGED_MARKER)
        && content.lines().any(|line| line == project_marker)
        && content.lines().any(|line| line == owner_marker)
    {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver collision at {}; file is not owned by this project/config",
                path.display()
            ),
        ))
    }
}

fn ensure_reverse_owned(path: &Path, content: &str) -> Result<(), Failure> {
    if content.lines().any(|line| line == MANAGED_MARKER)
        && content.lines().any(|line| line == REVERSE_SCOPE_MARKER)
    {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver collision at {}; reverse scope is not managed by vzctl",
                path.display()
            ),
        ))
    }
}

fn has_other_managed_resolver(resolver_dir: &Path, current: &Path) -> Result<bool, Failure> {
    let entries = match fs::read_dir(resolver_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_failure("read directory", resolver_dir, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| io_failure("read directory", resolver_dir, error))?;
        let path = entry.path();
        if path == current || path == reverse_resolver_path(resolver_dir) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.ends_with(".vz.test") {
            continue;
        }
        if let Some(content) = read_existing(&path)? {
            if content.lines().any(|line| line == MANAGED_MARKER)
                && content.lines().any(|line| line.starts_with("# project: "))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn resolver_path(resolver_dir: &Path, project: &str) -> PathBuf {
    resolver_dir.join(format!("{project}.vz.test"))
}

fn reverse_resolver_path(resolver_dir: &Path) -> PathBuf {
    resolver_dir.join(REVERSE_RESOLVER_DOMAIN)
}

fn resolver_content(scope: &Scope, port: u16) -> String {
    format!(
        "{MANAGED_MARKER}\n# project: {}\n# owner: {}\nnameserver 127.0.0.1\nport {port}\n",
        scope.project, scope.owner
    )
}

fn reverse_resolver_content(port: u16) -> String {
    format!("{MANAGED_MARKER}\n{REVERSE_SCOPE_MARKER}\nnameserver 127.0.0.1\nport {port}\n")
}

fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "resolver has no parent"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid resolver filename"))?;
    let mut last_error = None;
    for attempt in 0..100 {
        let temporary = parent.join(format!(".{file_name}.tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(mut file) => {
                let result = publish_temporary(&mut file, &temporary, path, content);
                if result.is_err() {
                    let _ = fs::remove_file(&temporary);
                }
                return result;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no temp name")))
}

fn publish_temporary(
    file: &mut File,
    temporary: &Path,
    destination: &Path,
    content: &[u8],
) -> io::Result<()> {
    file.write_all(content)?;
    file.sync_all()?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    fs::rename(temporary, destination)?;
    File::open(
        destination
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent"))?,
    )?
    .sync_all()
}

fn io_failure(operation: &str, path: &Path, error: io::Error) -> Failure {
    let sudo = if error.kind() == io::ErrorKind::PermissionDenied {
        "; run this command with sudo"
    } else {
        ""
    };
    Failure::new(
        EXIT_RESOLVER,
        format!("{operation} {}: {error}{sudo}", path.display()),
    )
}

fn emit_success(
    format: Format,
    action: Action,
    scope: &Scope,
    path: &Path,
    port: u16,
    change: Change,
) {
    let message = format!("resolver {}: {}", change.as_str(), path.display());
    match format {
        Format::Human => println!("{message}"),
        Format::Json => println!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": action.command(),
                "status": "ok",
                "exit_code": 0,
                "summary": {
                    "message": message,
                    "change": change.as_str(),
                },
                "resolver": {
                    "project": scope.project,
                    "domain": format!("{}.vz.test", scope.project),
                    "path": path,
                    "nameserver": "127.0.0.1",
                    "port": port,
                    "managed": true,
                }
            })
        ),
    }
}

fn emit_query(format: Format, response: &QueryResponse, exit_code: u8) {
    match format {
        Format::Human => {
            for answer in &response.answers {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    answer.name, answer.ttl, answer.class, answer.record_type, answer.data
                );
            }
            if response.answers.is_empty() {
                println!(
                    "{} {}: {} (no answers)",
                    response.name,
                    response.query_type.as_str(),
                    rcode_name(response.rcode)
                );
            }
            if response.rcode != 0 {
                eprintln!(
                    "DNS server {} returned {}",
                    response.server,
                    rcode_name(response.rcode)
                );
            }
        }
        Format::Json => println!("{}", query_json(response, exit_code)),
    }
}

fn emit_query_failure(format: Format, options: &Options, failure: &Failure) {
    match format {
        Format::Human => eprintln!("{}", failure.message),
        Format::Json => println!("{}", query_failure_json(options, failure)),
    }
}

fn query_failure_json(options: &Options, failure: &Failure) -> serde_json::Value {
    json!({
        "apiVersion": API_VERSION,
        "command": "dns.query",
        "status": "fail",
        "exit_code": failure.code,
        "summary": {
            "message": failure.message,
            "answers": 0,
            "rcode": serde_json::Value::Null,
        },
        "query": {
            "name": options.query_name,
            "type": options.query_type.as_str(),
            "server": options.server,
        },
        "rcode": serde_json::Value::Null,
        "rcode_code": serde_json::Value::Null,
        "authoritative": false,
        "truncated": false,
        "answers": [],
    })
}

fn query_json(response: &QueryResponse, exit_code: u8) -> serde_json::Value {
    let rcode = rcode_name(response.rcode);
    json!({
        "apiVersion": API_VERSION,
        "command": "dns.query",
        "status": if exit_code == 0 { "ok" } else { "fail" },
        "exit_code": exit_code,
        "summary": {
            "message": format!(
                "{} {} via {}: {} answer(s), {rcode}",
                response.name,
                response.query_type.as_str(),
                response.server,
                response.answers.len()
            ),
            "answers": response.answers.len(),
            "rcode": rcode,
        },
        "query": {
            "name": response.name,
            "type": response.query_type.as_str(),
            "server": response.server,
        },
        "rcode": rcode,
        "rcode_code": response.rcode,
        "authoritative": response.authoritative,
        "truncated": response.truncated,
        "answers": response.answers.iter().map(|answer| json!({
            "name": answer.name,
            "type": answer.record_type,
            "class": answer.class,
            "ttl": answer.ttl,
            "data": answer.data,
        })).collect::<Vec<_>>(),
    })
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    match format {
        Format::Human => eprintln!("{}", failure.message),
        Format::Json => println!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": command,
                "status": "fail",
                "exit_code": failure.code,
                "summary": { "message": failure.message },
            })
        ),
    }
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| Format::Json)
        .unwrap_or(Format::Human)
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn usage() -> &'static str {
    "usage: vzctl dns status [--format human|json]\n       vzctl dns query <name> [--type A|AAAA|PTR] [--server <IP:port>] [--format human|json]\n       vzctl dns install-resolver|uninstall-resolver [--project <name>] [--config <path>] [--format human|json]\n       vzctl dns install-bind-helper [--allow-uid <uid>]|uninstall-bind-helper [--format human|json]"
}

pub(crate) struct DoctorHostListenerCheck {
    pub(crate) id: &'static str,
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) details: Value,
}

/// Live UDP probe for the host DNS listener (catches stale :15353 sockets).
pub(crate) fn doctor_host_listener_check(port: u16, port_in_use: bool) -> DoctorHostListenerCheck {
    let server = format!("127.0.0.1:{port}");
    let details = json!({
        "server": server,
        "port_in_use": port_in_use,
    });
    if !port_in_use {
        return DoctorHostListenerCheck {
            id: "dns.host_probe",
            ok: true,
            message: format!("host DNS probe skipped; {server} is not in use"),
            details,
        };
    }
    match probe_host_listener(&server) {
        Ok(()) => DoctorHostListenerCheck {
            id: "dns.host_probe",
            ok: true,
            message: format!("host DNS responds on {server}"),
            details,
        },
        Err(error) => DoctorHostListenerCheck {
            id: "dns.host_probe",
            ok: false,
            message: format!("host DNS on {server} does not respond ({error})"),
            details: json!({
                "server": server,
                "port_in_use": port_in_use,
                "error": error,
            }),
        },
    }
}

fn probe_host_listener(server: &str) -> Result<(), String> {
    let address = server.parse::<SocketAddr>().map_err(|_| {
        format!("invalid DNS server {server}; expected IP:port")
    })?;
    let transaction_id = transaction_id();
    let request = build_query(transaction_id, "vz.test", QueryType::A);
    let bind_address = match address.ip() {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    };
    let socket = UdpSocket::bind(bind_address)
        .map_err(|error| format!("create UDP socket: {error}"))?;
    socket
        .set_read_timeout(Some(DNS_TIMEOUT))
        .map_err(|error| format!("set UDP timeout: {error}"))?;
    socket
        .connect(address)
        .map_err(|error| format!("connect UDP socket: {error}"))?;
    socket
        .send(&request)
        .map_err(|error| format!("send UDP query: {error}"))?;
    let mut buffer = [0_u8; 512];
    let count = socket
        .recv(&mut buffer)
        .map_err(|error| format!("receive UDP response: {error}"))?;
    if count >= DNS_HEADER_LENGTH {
        Ok(())
    } else {
        Err("response is shorter than DNS header".into())
    }
}

pub(crate) struct DoctorBindHelperCheck {
    pub(crate) id: &'static str,
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) details: Value,
}

/// Doctor probe for privileged guest-DNS bind helper (LaunchDaemon + UDS).
pub(crate) fn doctor_bind_helper_check() -> DoctorBindHelperCheck {
    let guest_port = std::env::var("VZCTL_DNS_GUEST_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(53);
    let socket_path = std::env::var("VZCTL_DNS_BIND_SOCK")
        .unwrap_or_else(|_| BIND_HELPER_SOCKET_DEFAULT.to_string());
    let binary = Path::new(BIND_HELPER_LIBEXEC);
    let marker = Path::new(BIND_HELPER_MARKER);
    let plist = Path::new(BIND_HELPER_PLIST);
    let binary_present = binary.is_file();
    let marker_present = marker.is_file();
    let plist_present = plist.is_file();
    let managed = marker_present
        && fs::read_to_string(marker)
            .map(|content| {
                content.lines().any(|line| line == MANAGED_MARKER)
                    && content
                        .lines()
                        .any(|line| line.starts_with("# allow-uid: "))
            })
            .unwrap_or(false);
    let socket_present = Path::new(&socket_path).exists();
    let socket_connectable = UnixStream::connect(&socket_path).is_ok();
    let launchd_loaded = launchctl_bind_helper_loaded();

    let details = json!({
        "guest_port": guest_port,
        "requires_helper": guest_port > 0 && guest_port < 1024,
        "socket": socket_path,
        "socket_present": socket_present,
        "socket_connectable": socket_connectable,
        "binary": BIND_HELPER_LIBEXEC,
        "binary_present": binary_present,
        "plist": BIND_HELPER_PLIST,
        "plist_present": plist_present,
        "marker": BIND_HELPER_MARKER,
        "marker_present": marker_present,
        "managed": managed,
        "launchd_loaded": launchd_loaded,
        "label": BIND_HELPER_LABEL,
        "action": "install-bind-helper",
    });

    if guest_port >= 1024 {
        return DoctorBindHelperCheck {
            id: "dns.bind_helper",
            ok: true,
            message: format!(
                "guest DNS uses unprivileged port {guest_port}; bind-helper not required"
            ),
            details,
        };
    }

    if socket_connectable && binary_present && managed {
        return DoctorBindHelperCheck {
            id: "dns.bind_helper",
            ok: true,
            message: format!("dns bind-helper ready ({socket_path}; guest :{guest_port})"),
            details,
        };
    }

    let mut missing = Vec::new();
    if !binary_present {
        missing.push("binary");
    }
    if !managed {
        missing.push("managed marker");
    }
    if !plist_present {
        missing.push("LaunchDaemon plist");
    }
    if !launchd_loaded {
        missing.push("launchd job");
    }
    if !socket_connectable {
        missing.push("socket");
    }
    DoctorBindHelperCheck {
        id: "dns.bind_helper",
        ok: false,
        message: format!(
            "dns bind-helper missing ({}); run: sudo vzctl dns install-bind-helper",
            missing.join(", ")
        ),
        details,
    }
}

fn launchctl_bind_helper_loaded() -> bool {
    std::process::Command::new("launchctl")
        .args(["print", &format!("system/{BIND_HELPER_LABEL}")])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::os::unix::fs::symlink;
    use std::thread;

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vzctl-dns-{name}-{}-{unique}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    fn scope(owner: &str) -> Scope {
        Scope {
            project: "edge-dmz".to_string(),
            owner: owner.to_string(),
        }
    }

    #[test]
    fn doctor_host_listener_probe_skips_when_port_is_free() {
        let check = doctor_host_listener_check(15353, false);
        assert!(check.ok);
        assert_eq!(check.id, "dns.host_probe");
        assert!(check.message.contains("skipped"));
    }

    #[test]
    fn install_and_uninstall_are_idempotent() {
        let dir = temp_dir("idempotent");
        let scope = scope("config-a");
        let (path, first) = install(&dir, &scope, 15353).unwrap();
        assert_eq!(first, Change::Installed);
        let reverse = reverse_resolver_path(&dir);
        assert!(fs::read_to_string(&reverse)
            .unwrap()
            .contains(REVERSE_SCOPE_MARKER));
        assert_eq!(install(&dir, &scope, 15353).unwrap().1, Change::Unchanged);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(uninstall(&dir, &scope).unwrap().1, Change::Removed);
        assert!(!reverse.exists());
        assert_eq!(uninstall(&dir, &scope).unwrap().1, Change::Absent);
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn shared_reverse_resolver_survives_until_last_project_is_removed() {
        let dir = temp_dir("shared-reverse");
        let first = scope("config-a");
        let second = Scope {
            project: "payments".to_string(),
            owner: "config-b".to_string(),
        };
        install(&dir, &first, 15353).unwrap();
        install(&dir, &second, 15353).unwrap();
        let reverse = reverse_resolver_path(&dir);

        uninstall(&dir, &first).unwrap();
        assert!(reverse.exists());
        uninstall(&dir, &second).unwrap();
        assert!(!reverse.exists());
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn foreign_reverse_resolver_is_never_overwritten() {
        let dir = temp_dir("reverse-collision");
        let reverse = reverse_resolver_path(&dir);
        fs::write(&reverse, "nameserver 192.0.2.53\n").unwrap();

        assert_eq!(
            install(&dir, &scope("config-a"), 15353).unwrap_err().code,
            EXIT_RESOLVER
        );
        assert_eq!(
            fs::read_to_string(reverse).unwrap(),
            "nameserver 192.0.2.53\n"
        );
        assert!(!resolver_path(&dir, "edge-dmz").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn install_updates_port_for_same_owner() {
        let dir = temp_dir("update");
        let scope = scope("config-a");
        let (path, _) = install(&dir, &scope, 15353).unwrap();
        assert_eq!(install(&dir, &scope, 15354).unwrap().1, Change::Updated);
        assert!(fs::read_to_string(path).unwrap().contains("port 15354"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn collision_is_never_overwritten_or_removed() {
        let dir = temp_dir("collision");
        let first = scope("config-a");
        let second = scope("config-b");
        let (path, _) = install(&dir, &first, 15353).unwrap();
        let original = fs::read_to_string(&path).unwrap();
        assert_eq!(
            install(&dir, &second, 15353).unwrap_err().code,
            EXIT_RESOLVER
        );
        assert_eq!(uninstall(&dir, &second).unwrap_err().code, EXIT_RESOLVER);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn symlink_target_is_rejected() {
        let dir = temp_dir("symlink");
        let target = dir.join("foreign");
        fs::write(&target, "keep\n").unwrap();
        let path = resolver_path(&dir, "edge-dmz");
        symlink(&target, &path).unwrap();
        assert_eq!(
            install(&dir, &scope("config-a"), 15353).unwrap_err().code,
            EXIT_RESOLVER
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "keep\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scope_accepts_environment_directory() {
        let dir = temp_dir("config-dir");
        let config = dir.join(DEFAULT_CONFIG);
        fs::write(
            &config,
            "apiVersion: hypernetwork/v1\nspec:\n  project: edge-dmz\n",
        )
        .unwrap();
        let options = Options {
            action: Action::Install,
            project: None,
            config: dir.clone(),
            config_explicit: true,
            format: Format::Human,
            query_name: None,
            query_type: QueryType::A,
            server: DEFAULT_DNS_SERVER.to_string(),
            allow_uid: None,
        };
        let resolved = resolve_scope(&options).unwrap();
        assert_eq!(resolved.project, "edge-dmz");
        assert!(resolved.owner.starts_with("config-"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scope_uses_spec_project_and_config_identity() {
        let dir = temp_dir("config");
        let config = dir.join(DEFAULT_CONFIG);
        fs::write(
            &config,
            "apiVersion: hypernetwork/v1\nspec:\n  project: edge-dmz\n",
        )
        .unwrap();
        let options = Options {
            action: Action::Install,
            project: None,
            config: config.clone(),
            config_explicit: false,
            format: Format::Human,
            query_name: None,
            query_type: QueryType::A,
            server: DEFAULT_DNS_SERVER.to_string(),
            allow_uid: None,
        };
        let resolved = resolve_scope(&options).unwrap();
        assert_eq!(resolved.project, "edge-dmz");
        assert!(resolved.owner.starts_with("config-"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_noncanonical_project_labels() {
        assert!(validate_project("edge-dmz").is_ok());
        for project in ["Edge", "../edge", "edge.dmz", "-edge", "edge-", ""] {
            assert_eq!(validate_project(project).unwrap_err().code, EXIT_INVALID);
        }
    }

    #[test]
    fn json_success_uses_cli_v1_command_name() {
        let value: Value = json!({
            "apiVersion": API_VERSION,
            "command": Action::Install.command(),
            "status": "ok",
            "exit_code": 0
        });
        assert_eq!(value["command"], "dns.install-resolver");
        assert_eq!(value["apiVersion"], API_VERSION);
        assert_eq!(
            Action::InstallBindHelper.command(),
            "dns.install-bind-helper"
        );
        assert_eq!(
            Action::UninstallBindHelper.command(),
            "dns.uninstall-bind-helper"
        );
    }

    #[test]
    fn bind_helper_plist_substitutes_placeholders() {
        let plist = bind_helper_plist_content(501).unwrap();
        assert!(plist.contains(BIND_HELPER_LIBEXEC));
        assert!(plist.contains("<string>501</string>"));
        assert!(plist.contains(BIND_HELPER_SOCKET));
        assert!(plist.contains(BIND_HELPER_LABEL));
        assert!(!plist.contains("__BINARY_PATH__"));
        assert!(!plist.contains("__ALLOW_UID__"));
    }

    #[test]
    fn bind_helper_parse_rejects_project_flags() {
        let err = parse(
            ["install-bind-helper", "--project", "edge-dmz"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap_err();
        assert_eq!(err.code, EXIT_USAGE);
        let ok = parse(
            ["install-bind-helper", "--format", "json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(ok.action, Action::InstallBindHelper);
        assert_eq!(ok.format, Format::Json);
    }

    #[test]
    fn doctor_bind_helper_check_reports_missing_without_socket() {
        // Unprivileged guest-port override → ok without LaunchDaemon.
        std::env::set_var("VZCTL_DNS_GUEST_PORT", "15353");
        let check = doctor_bind_helper_check();
        std::env::remove_var("VZCTL_DNS_GUEST_PORT");
        assert!(check.ok, "{}", check.message);
        assert_eq!(check.id, "dns.bind_helper");
        assert_eq!(check.details["requires_helper"], false);

        std::env::set_var("VZCTL_DNS_GUEST_PORT", "53");
        std::env::set_var(
            "VZCTL_DNS_BIND_SOCK",
            format!(
                "/tmp/vzctl-dns-bind-missing-{}-{}.sock",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ),
        );
        let check = doctor_bind_helper_check();
        std::env::remove_var("VZCTL_DNS_GUEST_PORT");
        std::env::remove_var("VZCTL_DNS_BIND_SOCK");
        assert!(!check.ok, "{}", check.message);
        assert!(check.message.contains("install-bind-helper"));
        assert_eq!(check.details["action"], "install-bind-helper");
        assert_eq!(check.details["requires_helper"], true);
    }

    #[test]
    fn query_options_accept_name_flags_in_any_order() {
        let options = parse(
            [
                "query",
                "--format",
                "json",
                "--type",
                "aaaa",
                "--server",
                "[::1]:15353",
                "web.dmz.edge-dmz.vz.test.",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(options.action, Action::Query);
        assert_eq!(
            options.query_name.as_deref(),
            Some("web.dmz.edge-dmz.vz.test.")
        );
        assert_eq!(options.query_type, QueryType::Aaaa);
        assert_eq!(options.server, "[::1]:15353");
        assert_eq!(options.format, Format::Json);

        let reverse = parse(
            ["query", "10.0.80.10.in-addr.arpa", "--type", "PTR"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(reverse.query_type, QueryType::Ptr);
    }

    #[test]
    fn status_options_and_rpc() {
        let options = parse(
            ["status", "--format", "json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(options.action, Action::Status);
        assert_eq!(options.format, Format::Json);

        let dir = temp_dir("status-rpc");
        let socket = dir.join("vz.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert!(request.contains("dns.status"), "{request}");
            writeln!(
                stream,
                r#"{{"jsonrpc":"2.0","result":{{"ok":true,"listeners":["127.0.0.1:15353"],"records":2,"zones":1,"ttl":15,"upstream":"system","last_error":null}},"id":1}}"#
            )
            .unwrap();
        });
        let dns = dns_status(&socket).unwrap();
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
        assert_eq!(dns["ok"], true);
        assert_eq!(dns["records"], 2);
    }

    #[test]
    fn query_rejects_missing_name_and_unsupported_type() {
        let missing = parse(["query"].into_iter().map(str::to_string)).unwrap_err();
        assert_eq!(missing.code, EXIT_USAGE);
        let unsupported = parse(
            ["query", "web.vz.test", "--type", "MX"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap_err();
        assert_eq!(unsupported.code, EXIT_INVALID);
    }

    #[test]
    fn direct_udp_query_decodes_authoritative_a_answer() {
        let (server, handle) = dns_fixture(0, Some([10, 80, 0, 10]));
        let response = execute_query("web.dmz.edge-dmz.vz.test", QueryType::A, &server).unwrap();
        handle.join().unwrap();

        assert_eq!(response.rcode, 0);
        assert!(response.authoritative);
        assert!(!response.truncated);
        assert_eq!(
            response.answers,
            vec![DnsAnswer {
                name: "web.dmz.edge-dmz.vz.test".to_string(),
                record_type: "A".to_string(),
                class: "IN".to_string(),
                ttl: 15,
                data: "10.80.0.10".to_string(),
            }]
        );
    }

    #[test]
    fn nxdomain_keeps_rcode_and_answers_in_failure_envelope() {
        let (server, handle) = dns_fixture(3, None);
        let response =
            execute_query("missing.dmz.edge-dmz.vz.test", QueryType::A, &server).unwrap();
        handle.join().unwrap();
        let value = query_json(&response, EXIT_DNS_QUERY);

        assert_eq!(value["status"], "fail");
        assert_eq!(value["exit_code"], EXIT_DNS_QUERY);
        assert_eq!(value["rcode"], "NXDOMAIN");
        assert_eq!(value["rcode_code"], 3);
        assert_eq!(value["answers"], json!([]));
    }

    #[test]
    fn response_parser_decodes_aaaa_answer() {
        let transaction_id = 0x1234;
        let request = build_query(transaction_id, "db.dmz.edge-dmz.vz.test", QueryType::Aaaa);
        let mut response = Vec::new();
        append_u16(&mut response, transaction_id);
        append_u16(&mut response, 0x8480);
        append_u16(&mut response, 1);
        append_u16(&mut response, 1);
        append_u16(&mut response, 0);
        append_u16(&mut response, 0);
        response.extend_from_slice(&request[DNS_HEADER_LENGTH..]);
        append_u16(&mut response, 0xc00c);
        append_u16(&mut response, 28);
        append_u16(&mut response, 1);
        response.extend_from_slice(&15_u32.to_be_bytes());
        append_u16(&mut response, 16);
        response.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());

        let parsed = parse_response(
            &response,
            transaction_id,
            "db.dmz.edge-dmz.vz.test",
            QueryType::Aaaa,
            DEFAULT_DNS_SERVER,
        )
        .unwrap();
        assert_eq!(parsed.answers[0].record_type, "AAAA");
        assert_eq!(parsed.answers[0].data, "::1");
    }

    #[test]
    fn response_parser_decodes_ptr_answer() {
        let transaction_id = 0x5678;
        let query_name = "10.0.80.10.in-addr.arpa";
        let target = "web.dmz.edge-dmz.vz.test";
        let request = build_query(transaction_id, query_name, QueryType::Ptr);
        let mut response = Vec::new();
        append_u16(&mut response, transaction_id);
        append_u16(&mut response, 0x8480);
        append_u16(&mut response, 1);
        append_u16(&mut response, 1);
        append_u16(&mut response, 0);
        append_u16(&mut response, 0);
        response.extend_from_slice(&request[DNS_HEADER_LENGTH..]);
        append_u16(&mut response, 0xc00c);
        append_u16(&mut response, 12);
        append_u16(&mut response, 1);
        response.extend_from_slice(&15_u32.to_be_bytes());
        let mut encoded_target = Vec::new();
        for label in target.split('.') {
            encoded_target.push(label.len() as u8);
            encoded_target.extend_from_slice(label.as_bytes());
        }
        encoded_target.push(0);
        append_u16(&mut response, encoded_target.len() as u16);
        response.extend_from_slice(&encoded_target);

        let parsed = parse_response(
            &response,
            transaction_id,
            query_name,
            QueryType::Ptr,
            DEFAULT_DNS_SERVER,
        )
        .unwrap();
        assert_eq!(parsed.answers[0].record_type, "PTR");
        assert_eq!(parsed.answers[0].data, target);
    }

    #[test]
    fn transport_failure_envelope_keeps_query_shape() {
        let options = parse(
            ["query", "web.dmz.edge-dmz.vz.test", "--format", "json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        let value = query_failure_json(
            &options,
            &Failure::new(EXIT_DNS_QUERY, "receive UDP response: timed out"),
        );
        assert_eq!(value["command"], "dns.query");
        assert_eq!(value["exit_code"], EXIT_DNS_QUERY);
        assert!(value["rcode"].is_null());
        assert_eq!(value["answers"], json!([]));
        assert_eq!(value["query"]["server"], DEFAULT_DNS_SERVER);
    }

    #[test]
    fn query_json_matches_cli_v1_golden_shape() {
        let response = QueryResponse {
            name: "web.dmz.edge-dmz.vz.test".to_string(),
            query_type: QueryType::A,
            server: DEFAULT_DNS_SERVER.to_string(),
            rcode: 0,
            authoritative: true,
            truncated: false,
            answers: vec![DnsAnswer {
                name: "web.dmz.edge-dmz.vz.test".to_string(),
                record_type: "A".to_string(),
                class: "IN".to_string(),
                ttl: 15,
                data: "10.80.0.10".to_string(),
            }],
        };
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/dns-query.json")).unwrap();
        assert_eq!(query_json(&response, 0), expected);
    }

    fn dns_fixture(rcode: u16, address: Option<[u8; 4]>) -> (String, thread::JoinHandle<()>) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = socket.local_addr().unwrap().to_string();
        let handle = thread::spawn(move || {
            let mut request = [0_u8; 512];
            let (count, peer) = socket.recv_from(&mut request).unwrap();
            let mut response = Vec::new();
            response.extend_from_slice(&request[..2]);
            append_u16(&mut response, 0x8480 | rcode);
            append_u16(&mut response, 1);
            append_u16(&mut response, u16::from(address.is_some()));
            append_u16(&mut response, 0);
            append_u16(&mut response, 0);
            response.extend_from_slice(&request[DNS_HEADER_LENGTH..count]);
            if let Some(address) = address {
                append_u16(&mut response, 0xc00c);
                append_u16(&mut response, 1);
                append_u16(&mut response, 1);
                response.extend_from_slice(&15_u32.to_be_bytes());
                append_u16(&mut response, 4);
                response.extend_from_slice(&address);
            }
            socket.send_to(&response, peer).unwrap();
        });
        (endpoint, handle)
    }
}
