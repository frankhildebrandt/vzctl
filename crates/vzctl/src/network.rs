use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_SUPERVISOR: u8 = 10;
pub(crate) const EXIT_NETWORK: u8 = 17;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Debug, Eq, PartialEq)]
enum Operation {
    Create {
        name: String,
        cidr: String,
        mode: String,
        nat_egress: bool,
        metadata: Metadata,
    },
    Attach {
        vm_id: String,
        network: String,
        ip: String,
        metadata: Metadata,
    },
    List,
    Detach {
        vm_id: String,
        network: String,
    },
    Delete {
        name: String,
    },
    DefaultShow,
    DefaultSet {
        name: String,
        cidr: String,
    },
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Metadata {
    labels: BTreeMap<String, String>,
    project: Option<String>,
    stack: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    operation: Operation,
    format: Format,
}

#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) code: u8,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VmNetworkSelection {
    pub(crate) network: String,
    pub(crate) cidr: String,
    pub(crate) ip: String,
    pub(crate) gateway: String,
    pub(crate) dns: String,
    pub(crate) project: Option<String>,
    pub(crate) prefix: u8,
    pub(crate) automatic: bool,
    pub(crate) created: bool,
    /// `vmnet` or `docker` (logical; no helper NIC).
    pub(crate) backend: String,
}

impl VmNetworkSelection {
    pub(crate) fn is_docker_backend(&self) -> bool {
        self.backend == "docker"
    }
}

pub(crate) fn command(args: impl Iterator<Item = String>, socket_path: &Path) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let options = match parse(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "net", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let command = options.command();
    let (method, params) = options.request();
    match rpc(socket_path, method, params) {
        Ok(result) => {
            let envelope = success_envelope(command, result);
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human => print_human(command, &envelope),
            }
            ExitCode::SUCCESS
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
            Operation::Create { .. } => "net.create",
            Operation::Attach { .. } => "net.attach",
            Operation::List => "net.list",
            Operation::Detach { .. } => "net.detach",
            Operation::Delete { .. } => "net.delete",
            Operation::DefaultShow => "net.default.show",
            Operation::DefaultSet { .. } => "net.default.set",
        }
    }

    fn request(&self) -> (&'static str, Value) {
        match &self.operation {
            Operation::Create {
                name,
                cidr,
                mode,
                nat_egress,
                metadata,
            } => (
                "net.create",
                metadata.params(json!({
                    "name": name,
                    "cidr": cidr,
                    "mode": mode,
                    "nat_egress": nat_egress,
                })),
            ),
            Operation::Attach {
                vm_id,
                network,
                ip,
                metadata,
            } => (
                "net.attach",
                metadata.params(json!({ "vm_id": vm_id, "network": network, "ip": ip })),
            ),
            Operation::List => ("net.list", json!({})),
            Operation::Detach { vm_id, network } => {
                ("net.detach", json!({ "vm_id": vm_id, "network": network }))
            }
            Operation::Delete { name } => ("net.delete", json!({ "name": name })),
            Operation::DefaultShow => ("net.default.show", json!({})),
            Operation::DefaultSet { name, cidr } => {
                ("net.default.set", json!({ "name": name, "cidr": cidr }))
            }
        }
    }
}

impl Metadata {
    fn params(&self, mut base: Value) -> Value {
        let object = base.as_object_mut().expect("base params are an object");
        object.insert("labels".to_string(), json!(self.labels));
        object.insert("project".to_string(), json!(self.project));
        object.insert("stack".to_string(), json!(self.stack));
        base
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let operation = args
        .next()
        .ok_or_else(|| usage("usage: vzctl net create|attach|list|detach|delete|default ..."))?;
    let rest = args.collect::<Vec<_>>();
    match operation.as_str() {
        "create" => parse_create(rest),
        "attach" => parse_attach(rest),
        "list" => {
            let (format, values, metadata) = parse_flags(rest, &[])?;
            if !values.is_empty() || metadata != Metadata::default() {
                return Err(usage("net list accepts only --format human|json"));
            }
            Ok(Options {
                operation: Operation::List,
                format,
            })
        }
        "detach" => parse_detach(rest),
        "delete" => parse_delete(rest),
        "default" => parse_default(rest),
        _ => Err(usage(format!("unknown net command: {operation}"))),
    }
}

fn parse_default(args: Vec<String>) -> Result<Options, Failure> {
    let action = args
        .first()
        .ok_or_else(|| usage("usage: vzctl net default show|set ..."))?;
    match action.as_str() {
        "show" => {
            let (format, values, metadata) = parse_flags(args[1..].to_vec(), &[])?;
            if !values.is_empty() || metadata != Metadata::default() {
                return Err(usage("net default show accepts only --format human|json"));
            }
            Ok(Options {
                operation: Operation::DefaultShow,
                format,
            })
        }
        "set" => {
            let name = positional(&args[1..], "net default set requires a network name")?;
            let (format, values, metadata) = parse_flags(args[2..].to_vec(), &["--cidr"])?;
            if metadata != Metadata::default() {
                return Err(usage("net default set does not accept metadata"));
            }
            let cidr = required(&values, "--cidr")?;
            validate_cidr(&cidr)?;
            Ok(Options {
                operation: Operation::DefaultSet { name, cidr },
                format,
            })
        }
        _ => Err(usage(format!("unknown net default command: {action}"))),
    }
}

fn parse_create(args: Vec<String>) -> Result<Options, Failure> {
    let name = positional(&args, "net create requires a network name")?;
    let (format, values, metadata) =
        parse_flags(args[1..].to_vec(), &["--cidr", "--mode", "--nat-egress"])?;
    let cidr = required(&values, "--cidr")?;
    validate_cidr(&cidr)?;
    let mode = values
        .get("--mode")
        .cloned()
        .unwrap_or_else(|| "shared".to_string());
    if mode != "shared" {
        return Err(invalid(
            "bridged mode is unsupported in v0.1; use --mode shared",
        ));
    }
    let nat_egress = match values.get("--nat-egress").map(String::as_str) {
        None | Some("true") | Some("1") | Some("yes") => true,
        Some("false") | Some("0") | Some("no") => false,
        Some(other) => {
            return Err(invalid(format!(
                "invalid --nat-egress {other:?}; use true|false"
            )));
        }
    };
    Ok(Options {
        operation: Operation::Create {
            name,
            cidr,
            mode,
            nat_egress,
            metadata,
        },
        format,
    })
}

fn parse_attach(args: Vec<String>) -> Result<Options, Failure> {
    let vm_id = positional(&args, "net attach requires a VM id")?;
    let (format, values, metadata) = parse_flags(args[1..].to_vec(), &["--network", "--ip"])?;
    let ip = required(&values, "--ip")?;
    ip.parse::<Ipv4Addr>()
        .map_err(|_| invalid(format!("invalid IPv4 address: {ip}")))?;
    Ok(Options {
        operation: Operation::Attach {
            vm_id,
            network: required(&values, "--network")?,
            ip,
            metadata,
        },
        format,
    })
}

fn validate_cidr(value: &str) -> Result<(), Failure> {
    let (address, prefix) = value
        .split_once('/')
        .ok_or_else(|| invalid(format!("invalid IPv4 CIDR: {value}")))?;
    let address = address
        .parse::<Ipv4Addr>()
        .map_err(|_| invalid(format!("invalid IPv4 CIDR: {value}")))?;
    let prefix = prefix
        .parse::<u8>()
        .ok()
        .filter(|prefix| (8..=30).contains(prefix))
        .ok_or_else(|| invalid(format!("invalid IPv4 CIDR prefix: {value}")))?;
    let raw = u32::from(address);
    let mask = u32::MAX << (32 - u32::from(prefix));
    if raw & mask != raw {
        return Err(invalid(format!(
            "CIDR must use its network address: {value}"
        )));
    }
    Ok(())
}

fn parse_detach(args: Vec<String>) -> Result<Options, Failure> {
    let vm_id = positional(&args, "net detach requires a VM id")?;
    let (format, values, metadata) = parse_flags(args[1..].to_vec(), &["--network"])?;
    if metadata != Metadata::default() {
        return Err(usage("net detach does not accept metadata"));
    }
    Ok(Options {
        operation: Operation::Detach {
            vm_id,
            network: required(&values, "--network")?,
        },
        format,
    })
}

fn parse_delete(args: Vec<String>) -> Result<Options, Failure> {
    let name = positional(&args, "net delete requires a network name")?;
    let (format, values, metadata) = parse_flags(args[1..].to_vec(), &[])?;
    if !values.is_empty() || metadata != Metadata::default() {
        return Err(usage("net delete accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Delete { name },
        format,
    })
}

fn positional(args: &[String], message: &str) -> Result<String, Failure> {
    match args.first() {
        Some(value) if !value.starts_with('-') && !value.is_empty() => Ok(value.clone()),
        _ => Err(usage(message)),
    }
}

fn parse_flags(
    args: Vec<String>,
    command_flags: &[&str],
) -> Result<(Format, BTreeMap<String, String>, Metadata), Failure> {
    let mut format = Format::Human;
    let mut values = BTreeMap::new();
    let mut metadata = Metadata::default();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let needs_value = flag == "--format"
            || flag == "--label"
            || flag == "--project"
            || flag == "--stack"
            || command_flags.contains(&flag.as_str());
        if !needs_value {
            return Err(usage(format!("unknown network option: {flag}")));
        }
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| usage(format!("{flag} requires a value")))?
            .clone();
        match flag.as_str() {
            "--format" => {
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    _ => return Err(usage(format!("unsupported network format: {value}"))),
                }
            }
            "--label" => {
                let (key, value) = value
                    .split_once('=')
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or_else(|| invalid(format!("label must be key=value: {value}")))?;
                metadata.labels.insert(key.to_string(), value.to_string());
            }
            "--project" => metadata.project = Some(value),
            "--stack" => metadata.stack = Some(value),
            _ => {
                if values.insert(flag.clone(), value).is_some() {
                    return Err(usage(format!("duplicate option: {flag}")));
                }
            }
        }
        index += 2;
    }
    Ok((format, values, metadata))
}

fn required(values: &BTreeMap<String, String>, flag: &str) -> Result<String, Failure> {
    values
        .get(flag)
        .cloned()
        .ok_or_else(|| usage(format!("{flag} is required")))
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format")
        .and_then(|pair| (pair[1] == "json").then_some(Format::Json))
        .unwrap_or(Format::Human)
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| Failure {
        code: EXIT_SUPERVISOR,
        message: format!("supervisor socket {}: {error}", socket_path.display()),
    })?;
    let timeout = Some(Duration::from_secs(5));
    stream
        .set_read_timeout(timeout)
        .and_then(|_| stream.set_write_timeout(timeout))
        .map_err(|error| Failure {
            code: EXIT_SUPERVISOR,
            message: format!("supervisor timeout setup: {error}"),
        })?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    writeln!(stream, "{request}").map_err(|error| Failure {
        code: EXIT_SUPERVISOR,
        message: format!("supervisor request: {error}"),
    })?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| Failure {
            code: EXIT_SUPERVISOR,
            message: format!("supervisor response: {error}"),
        })?;
    let response: Value = serde_json::from_str(&line).map_err(|error| Failure {
        code: EXIT_SUPERVISOR,
        message: format!("invalid supervisor response: {error}"),
    })?;
    if let Some(error) = response.get("error") {
        let rpc_code = error["code"].as_i64().unwrap_or(-32031);
        return Err(Failure {
            code: if rpc_code == -32602 {
                EXIT_INVALID
            } else {
                EXIT_NETWORK
            },
            message: error["message"]
                .as_str()
                .unwrap_or("network operation failed")
                .to_string(),
        });
    }
    response.get("result").cloned().ok_or_else(|| Failure {
        code: EXIT_SUPERVISOR,
        message: "supervisor response has no result".to_string(),
    })
}

pub(crate) fn ensure_vm_network(
    socket_path: &Path,
    vm_id: &str,
    requested_network: Option<&str>,
) -> Result<VmNetworkSelection, Failure> {
    let result = rpc(
        socket_path,
        "vm.network.ensure",
        json!({ "vm_id": vm_id, "network": requested_network }),
    )?;
    vm_network_selection(&result)
}

/// All current attachments for a VM (used after attach_nets so create can seed multi-NIC).
pub(crate) fn list_vm_attachments(
    socket_path: &Path,
    vm_id: &str,
) -> Result<Vec<VmNetworkSelection>, Failure> {
    let result = rpc(socket_path, "net.list", json!({}))?;
    let networks = result
        .get("networks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let network_by_name = networks
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            Some((name.to_string(), item.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut selections = Vec::new();
    for attachment in result
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if attachment.get("vm_id").and_then(Value::as_str) != Some(vm_id) {
            continue;
        }
        let network_name = attachment
            .get("network")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("attachment missing network"))?;
        let network = network_by_name
            .get(network_name)
            .ok_or_else(|| invalid(format!("attachment references unknown network {network_name}")))?;
        let wrapped = json!({
            "network": network,
            "attachment": attachment,
            "automatic": false,
            "created": false,
            "prefix": network
                .get("cidr")
                .and_then(Value::as_str)
                .and_then(|cidr| cidr.split_once('/').and_then(|(_, prefix)| prefix.parse::<u8>().ok()))
                .unwrap_or(24),
        });
        selections.push(vm_network_selection(&wrapped)?);
    }
    // vmnet NICs first (helper attachment order), docker-backend last (logical only).
    selections.sort_by(|left, right| {
        left.is_docker_backend()
            .cmp(&right.is_docker_backend())
            .then_with(|| left.network.cmp(&right.network))
    });
    Ok(selections)
}

fn vm_network_selection(result: &Value) -> Result<VmNetworkSelection, Failure> {
    let network = result
        .get("network")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("supervisor returned no network"))?;
    let attachment = result
        .get("attachment")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("supervisor returned no attachment"))?;
    let string = |object: &Map<String, Value>, key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| invalid(format!("supervisor returned no {key}")))
    };
    Ok(VmNetworkSelection {
        network: string(network, "name")?,
        cidr: string(network, "cidr")?,
        gateway: string(network, "gateway")?,
        dns: string(network, "dns")?,
        ip: string(attachment, "ip")?,
        project: attachment
            .get("project")
            .and_then(Value::as_str)
            .or_else(|| network.get("project").and_then(Value::as_str))
            .map(str::to_string),
        prefix: result
            .get("prefix")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| invalid("supervisor returned no network prefix"))?,
        automatic: result
            .get("automatic")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        created: result
            .get("created")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        backend: network
            .get("backend")
            .and_then(Value::as_str)
            .unwrap_or("vmnet")
            .to_string(),
    })
}

pub(crate) fn rollback_vm_network(socket_path: &Path, selection: &VmNetworkSelection, vm_id: &str) {
    if selection.created {
        let _ = rpc(
            socket_path,
            "net.detach",
            json!({ "vm_id": vm_id, "network": selection.network }),
        );
    }
}

fn success_envelope(command: &str, result: Value) -> Value {
    let mut envelope = Map::from_iter([
        ("apiVersion".to_string(), json!(API_VERSION)),
        ("command".to_string(), json!(command)),
        ("status".to_string(), json!("ok")),
        ("exit_code".to_string(), json!(0)),
    ]);
    match command {
        "net.list" => {
            let networks = result["networks"].as_array().map(Vec::len).unwrap_or(0);
            let attachments = result["attachments"].as_array().map(Vec::len).unwrap_or(0);
            envelope.insert(
                "summary".to_string(),
                json!({ "networks": networks, "attachments": attachments }),
            );
            envelope.insert("networks".to_string(), result["networks"].clone());
            envelope.insert("attachments".to_string(), result["attachments"].clone());
        }
        "net.attach" => {
            envelope.insert(
                "summary".to_string(),
                json!({
                    "message": format!(
                        "attached {} to {}",
                        result["vm_id"].as_str().unwrap_or("VM"),
                        result["network"].as_str().unwrap_or("network")
                    )
                }),
            );
            envelope.insert("attachment".to_string(), result);
        }
        "net.detach" => {
            envelope.insert(
                "summary".to_string(),
                json!({ "message": "network attachment detached" }),
            );
            envelope.insert("attachment".to_string(), result);
        }
        "net.delete" => {
            envelope.insert(
                "summary".to_string(),
                json!({
                    "message": format!(
                        "deleted network {}",
                        result["name"].as_str().unwrap_or("network")
                    )
                }),
            );
            envelope.insert("network".to_string(), result);
        }
        "net.default.show" | "net.default.set" => {
            let configured = !result.is_null();
            envelope.insert(
                "summary".to_string(),
                json!({
                    "message": if configured {
                        "default network configured"
                    } else {
                        "default network is not configured"
                    }
                }),
            );
            envelope.insert("default_network".to_string(), result);
        }
        _ => {
            envelope.insert(
                "summary".to_string(),
                json!({
                    "message": format!(
                        "created network {}",
                        result["name"].as_str().unwrap_or("network")
                    )
                }),
            );
            envelope.insert("network".to_string(), result);
        }
    }
    Value::Object(envelope)
}

fn print_human(command: &str, envelope: &Value) {
    match command {
        "net.list" => {
            for network in envelope["networks"].as_array().into_iter().flatten() {
                println!(
                    "{}\t{}\t{}\t{}",
                    network["name"].as_str().unwrap_or("-"),
                    network["cidr"].as_str().unwrap_or("-"),
                    network["mode"].as_str().unwrap_or("-"),
                    network["runtime_state"].as_str().unwrap_or("-")
                );
            }
            for attachment in envelope["attachments"].as_array().into_iter().flatten() {
                println!(
                    "  {}\t{}\t{}",
                    attachment["vm_id"].as_str().unwrap_or("-"),
                    attachment["network"].as_str().unwrap_or("-"),
                    attachment["ip"].as_str().unwrap_or("-")
                );
            }
        }
        "net.default.show" | "net.default.set" => {
            let network = &envelope["default_network"];
            if network.is_null() {
                println!("default network is not configured");
            } else {
                println!(
                    "{}\t{}\t{}\t{}",
                    network["name"].as_str().unwrap_or("-"),
                    network["cidr"].as_str().unwrap_or("-"),
                    network["mode"].as_str().unwrap_or("-"),
                    if network["network_exists"].as_bool().unwrap_or(false) {
                        "active"
                    } else {
                        "missing"
                    }
                );
            }
        }
        _ => println!(
            "{}",
            envelope["summary"]["message"]
                .as_str()
                .unwrap_or("network operation complete")
        ),
    }
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    match format {
        Format::Human => eprintln!("error: {}", failure.message),
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

fn usage(message: impl Into<String>) -> Failure {
    Failure {
        code: EXIT_USAGE,
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure {
        code: EXIT_INVALID,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_network_selection_prefers_attachment_project_for_guest_search_domain() {
        let selection = vm_network_selection(&json!({
            "network": {
                "name": "dmz",
                "cidr": "10.80.0.0/24",
                "gateway": "10.80.0.0",
                "dns": "10.80.0.0",
                "project": "network-project"
            },
            "attachment": {
                "ip": "10.80.0.10",
                "project": "edge-dmz"
            },
            "prefix": 24,
            "automatic": true,
            "created": true
        }))
        .unwrap();

        assert_eq!(selection.project.as_deref(), Some("edge-dmz"));
        assert_eq!(selection.gateway, "10.80.0.0");
        assert_eq!(selection.dns, "10.80.0.0");
    }

    #[test]
    fn parses_create_with_labels_and_metadata() {
        let options = parse(
            [
                "create",
                "dmz",
                "--cidr",
                "10.80.0.0/24",
                "--mode",
                "shared",
                "--label",
                "tier=edge",
                "--project",
                "demo",
                "--stack",
                "dev",
                "--format",
                "json",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(
            options,
            Options {
                operation: Operation::Create {
                    name: "dmz".to_string(),
                    cidr: "10.80.0.0/24".to_string(),
                    mode: "shared".to_string(),
                    nat_egress: true,
                    metadata: Metadata {
                        labels: BTreeMap::from([("tier".to_string(), "edge".to_string())]),
                        project: Some("demo".to_string()),
                        stack: Some("dev".to_string()),
                    },
                },
                format: Format::Json,
            }
        );
    }

    #[test]
    fn rejects_bridged_and_invalid_label() {
        let bridged = [
            "create",
            "dmz",
            "--cidr",
            "10.80.0.0/24",
            "--mode",
            "bridged",
        ]
        .into_iter()
        .map(str::to_string);
        assert_eq!(parse(bridged).unwrap_err().code, EXIT_INVALID);
        let bad_label = [
            "attach",
            "web",
            "--network",
            "dmz",
            "--ip",
            "10.80.0.10",
            "--label",
            "x",
        ]
        .into_iter()
        .map(str::to_string);
        assert_eq!(parse(bad_label).unwrap_err().code, EXIT_INVALID);
        let noncanonical = ["create", "dmz", "--cidr", "10.80.0.1/24"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(parse(noncanonical).unwrap_err().code, EXIT_INVALID);
        let invalid_ip = ["attach", "web", "--network", "dmz", "--ip", "not-an-ip"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(parse(invalid_ip).unwrap_err().code, EXIT_INVALID);
    }

    #[test]
    fn parses_default_show_and_set() {
        assert_eq!(
            parse(["default", "show"].into_iter().map(str::to_string)).unwrap(),
            Options {
                operation: Operation::DefaultShow,
                format: Format::Human,
            }
        );
        assert_eq!(
            parse(
                [
                    "default",
                    "set",
                    "lan",
                    "--cidr",
                    "10.70.0.0/24",
                    "--format",
                    "json"
                ]
                .into_iter()
                .map(str::to_string)
            )
            .unwrap(),
            Options {
                operation: Operation::DefaultSet {
                    name: "lan".to_string(),
                    cidr: "10.70.0.0/24".to_string(),
                },
                format: Format::Json,
            }
        );
    }

    #[test]
    fn default_show_envelope_is_cli_v1() {
        let envelope = success_envelope(
            "net.default.show",
            json!({
                "name": "lan",
                "cidr": "10.70.0.0/24",
                "mode": "shared",
                "access": "full",
                "nat_egress": true,
                "network_exists": true,
            }),
        );
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/net-default-show.json")).unwrap();
        assert_eq!(envelope, expected);
    }

    #[test]
    fn list_envelope_is_cli_v1() {
        let envelope = success_envelope(
            "net.list",
            json!({
                "networks": [{ "name": "dmz" }],
                "attachments": [{ "vm_id": "web" }],
            }),
        );
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/net-list.json")).unwrap();
        assert_eq!(envelope, expected);
    }
}
