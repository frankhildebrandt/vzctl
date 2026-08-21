use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_SUPERVISOR: u8 = 10;
pub(crate) const EXIT_ROUTE: u8 = 18;
const DEFAULT_CONFIG: &str = "hypernetwork.config.yaml";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Apply,
    Plan,
    Status,
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Apply => "route.apply",
            Self::Plan => "route.plan",
            Self::Status => "route.status",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    action: Action,
    router: Option<String>,
    config: Option<PathBuf>,
    format: Format,
}

#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
}

pub(crate) fn command(args: impl Iterator<Item = String>, socket_path: &Path) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let options = match parse(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "route", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let policies = match policies_for(&options) {
        Ok(policies) => policies,
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            return ExitCode::from(failure.code);
        }
    };
    let params = json!({
        "router": options.router,
        "policies": policies,
    });
    match rpc(socket_path, options.action.command(), params) {
        Ok(result) => {
            let routers = result["routers"].as_array().cloned().unwrap_or_default();
            let changed = result["changed"].as_bool().unwrap_or(false);
            let rule_count = routers
                .iter()
                .map(|router| router["rules"].as_array().map(Vec::len).unwrap_or(0))
                .sum::<usize>();
            let envelope = success_envelope(options.action.command(), routers, changed, rule_count);
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human => print_human(&envelope),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            let failure = if options.action == Action::Status
                && failure.message.contains("no active vzctl nftables")
            {
                Failure::new(
                    failure.code,
                    format!(
                        "{}; empty nftables can also mean policy-drop with no extra allows — check route plan --format json",
                        failure.message
                    ),
                )
            } else {
                failure
            };
            emit_failure(options.format, options.action.command(), &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let action = match args.next().as_deref() {
        Some("apply") => Action::Apply,
        Some("plan") => Action::Plan,
        Some("status") => Action::Status,
        _ => return Err(Failure::new(EXIT_USAGE, usage())),
    };
    let mut router = None;
    let mut config = None;
    let mut format = Format::Human;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--router" => {
                let value = args
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Failure::new(EXIT_USAGE, "--router requires a VM id"))?;
                if router.replace(value).is_some() {
                    return Err(Failure::new(EXIT_USAGE, "--router may only be used once"));
                }
            }
            "--config" if action != Action::Status => {
                let value = args
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Failure::new(EXIT_USAGE, "--config requires a path"))?;
                if config.replace(PathBuf::from(value)).is_some() {
                    return Err(Failure::new(EXIT_USAGE, "--config may only be used once"));
                }
            }
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => {
                        return Err(Failure::new(
                            EXIT_USAGE,
                            format!("unsupported route format: {value}"),
                        ))
                    }
                    None => {
                        return Err(Failure::new(EXIT_USAGE, "--format requires human or json"))
                    }
                }
            }
            "-h" | "--help" => return Err(Failure::new(EXIT_USAGE, usage())),
            _ => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown route option: {argument}"),
                ))
            }
        }
    }
    Ok(Options {
        action,
        router,
        config,
        format,
    })
}

fn policies_for(options: &Options) -> Result<Value, Failure> {
    if options.action == Action::Status {
        return Ok(json!([]));
    }
    let explicit = options.config.as_deref();
    let default = Path::new(DEFAULT_CONFIG);
    let path = explicit.or_else(|| default.is_file().then_some(default));
    let Some(path) = path else {
        return Ok(json!([]));
    };
    let source = fs::read_to_string(path).map_err(|error| {
        Failure::new(
            EXIT_INVALID,
            format!("read policy config {}: {error}", path.display()),
        )
    })?;
    let root: Value = serde_yaml::from_str(&source).map_err(|error| {
        Failure::new(
            EXIT_INVALID,
            format!("parse policy config {}: {error}", path.display()),
        )
    })?;
    let policies = root
        .pointer("/spec/policies")
        .or_else(|| root.get("policies"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    validate_policies(&policies)?;
    Ok(policies)
}

fn validate_policies(policies: &Value) -> Result<(), Failure> {
    let policies = policies
        .as_array()
        .ok_or_else(|| Failure::new(EXIT_INVALID, "policies must be an array"))?;
    let mut names = std::collections::BTreeSet::new();
    for policy in policies {
        let policy = policy
            .as_object()
            .ok_or_else(|| Failure::new(EXIT_INVALID, "each policy must be an object"))?;
        let name = required_string(policy, "name")?;
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
        {
            return Err(Failure::new(
                EXIT_INVALID,
                "policy name may only contain letters, digits, dot, dash, and underscore",
            ));
        }
        if !names.insert(name) {
            return Err(Failure::new(EXIT_INVALID, "policy names must be unique"));
        }
        required_string(policy, "network")?;
        if required_string(policy, "forward")? != "deny-all" {
            return Err(Failure::new(
                EXIT_INVALID,
                format!("policy {name} forward must be deny-all"),
            ));
        }
        let allows = policy
            .get("allow")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for allow in allows {
            let allow = allow.as_object().ok_or_else(|| {
                Failure::new(EXIT_INVALID, format!("invalid allow rule in policy {name}"))
            })?;
            required_string(allow, "to")?;
            let proto = required_string(allow, "proto")?;
            if !["tcp", "udp", "icmp"].contains(&proto) {
                return Err(Failure::new(
                    EXIT_INVALID,
                    format!("policy {name} proto must be tcp, udp, or icmp"),
                ));
            }
            let ports = allow
                .get("ports")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if proto == "icmp" && !ports.is_empty() {
                return Err(Failure::new(
                    EXIT_INVALID,
                    format!("policy {name} ICMP allow must not declare ports"),
                ));
            }
            if proto != "icmp" && ports.is_empty() {
                return Err(Failure::new(
                    EXIT_INVALID,
                    format!("policy {name} TCP/UDP allow requires ports"),
                ));
            }
            for port in ports {
                if !matches!(port.as_u64(), Some(1..=65_535)) {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        format!("policy {name} port must be 1...65535"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, Failure> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::new(EXIT_INVALID, format!("policy {key} must be a string")))
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| Format::Json)
        .unwrap_or(Format::Human)
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("supervisor socket {}: {error}", socket_path.display()),
        )
    })?;
    let timeout = Some(Duration::from_secs(35));
    stream
        .set_read_timeout(timeout)
        .and_then(|_| stream.set_write_timeout(timeout))
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor timeout: {error}")))?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    writeln!(stream, "{request}")
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor write: {error}")))?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor read: {error}")))?;
    let value: Value = serde_json::from_str(&response).map_err(|error| {
        Failure::new(EXIT_SUPERVISOR, format!("invalid supervisor JSON: {error}"))
    })?;
    if let Some(error) = value["error"].as_object() {
        let code = error["code"].as_i64().unwrap_or(-32018);
        let message = error["message"]
            .as_str()
            .unwrap_or("route operation failed")
            .to_string();
        return Err(Failure::new(
            if code == -32602 {
                EXIT_INVALID
            } else {
                EXIT_ROUTE
            },
            message,
        ));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "supervisor response has no result"))
}

fn print_human(envelope: &Value) {
    println!(
        "{} (changed: {})",
        envelope["summary"]["message"]
            .as_str()
            .unwrap_or("route operation complete"),
        envelope["summary"]["changed"].as_bool().unwrap_or(false)
    );
    for router in envelope["routers"].as_array().into_iter().flatten() {
        println!(
            "  {}: default {}, {} allow rule(s), {}{}",
            router["vm_id"].as_str().unwrap_or("?"),
            router["forward_policy"].as_str().unwrap_or("drop"),
            router["rules"].as_array().map(Vec::len).unwrap_or(0),
            if router["changed"].as_bool().unwrap_or(false) {
                "changed"
            } else {
                "unchanged"
            },
            if router["rules"].as_array().map(Vec::len).unwrap_or(0) == 0 {
                " (no extra allows; policy-drop may still be active)"
            } else {
                ""
            }
        );
        for change in router["policy_changes"].as_array().into_iter().flatten() {
            println!(
                "    {} policy {}",
                change["operation"].as_str().unwrap_or("?"),
                change["policy"].as_str().unwrap_or("?")
            );
        }
    }
}

fn success_envelope(command: &str, routers: Vec<Value>, changed: bool, rule_count: usize) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "command": command,
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!(
                "{} router configuration(s), {} active allow rule(s)",
                routers.len(),
                rule_count
            ),
            "changed": changed,
        },
        "routers": routers,
    })
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    match format {
        Format::Human => eprintln!("{}", failure.message),
        Format::Json => eprintln!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": command,
                "status": "error",
                "exit_code": failure.code,
                "error": { "message": failure.message },
            })
        ),
    }
}

fn usage() -> &'static str {
    "usage: vzctl route apply|plan [--config <path>] [--router <vm-id>] [--format human|json]\n       vzctl route status [--router <vm-id>] [--format human|json]"
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

    #[test]
    fn parses_policy_operations() {
        let options = parse(
            [
                "plan",
                "--config",
                "edge.yaml",
                "--router",
                "edge-router",
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
                action: Action::Plan,
                router: Some("edge-router".to_string()),
                config: Some(PathBuf::from("edge.yaml")),
                format: Format::Json,
            }
        );
    }

    #[test]
    fn extracts_and_validates_full_environment_policies() {
        let root: Value = serde_yaml::from_str(
            r#"
spec:
  policies:
    - name: dmz-default
      network: dmz
      forward: deny-all
      allow:
        - { to: lan, proto: tcp, ports: [5432] }
        - { to: dmz, proto: icmp }
"#,
        )
        .unwrap();
        let policies = root.pointer("/spec/policies").unwrap();
        validate_policies(policies).unwrap();
        assert_eq!(policies[0]["allow"][0]["ports"][0], 5432);
    }

    #[test]
    fn edge_dmz_example_matches_policy_schema() {
        let root: Value = serde_yaml::from_str(include_str!(
            "../../../examples/edge-dmz/hypernetwork.config.yaml"
        ))
        .unwrap();
        let policies = root.pointer("/spec/policies").unwrap();
        validate_policies(policies).unwrap();
        assert_eq!(policies[0]["name"], "dmz-default");
    }

    #[test]
    fn rejects_ports_for_icmp() {
        let policies = json!([{
            "name": "bad",
            "network": "dmz",
            "forward": "deny-all",
            "allow": [{ "to": "lan", "proto": "icmp", "ports": [80] }]
        }]);
        assert_eq!(validate_policies(&policies).unwrap_err().code, EXIT_INVALID);
    }

    #[test]
    fn route_status_envelope_is_cli_v1() {
        let actual = success_envelope(
            "route.status",
            vec![json!({
                "vm_id": "router",
                "changed": false,
                "active": true,
                "forward_policy": "drop",
                "networks": [],
                "policies": [],
                "rules": [],
                "policy_changes": [],
            })],
            false,
            0,
        );
        let expected: Value =
            serde_json::from_str(include_str!("../tests/golden/route-status.json")).unwrap();
        assert_eq!(actual, expected);
    }
}
