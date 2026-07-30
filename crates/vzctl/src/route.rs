use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_SUPERVISOR: u8 = 10;
pub(crate) const EXIT_ROUTE: u8 = 18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    router: Option<String>,
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
            emit_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    let params = json!({ "router": options.router });
    match rpc(socket_path, "route.apply", params) {
        Ok(result) => {
            let envelope = json!({
                "apiVersion": API_VERSION,
                "command": "route.apply",
                "status": "ok",
                "exit_code": 0,
                "summary": {
                    "message": format!(
                        "{} router configuration(s) applied",
                        result["routers"].as_array().map(Vec::len).unwrap_or(0)
                    ),
                    "changed": result["changed"].as_bool().unwrap_or(false),
                },
                "routers": result["routers"],
            });
            match options.format {
                Format::Json => println!("{envelope}"),
                Format::Human => print_human(&envelope),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    match args.next().as_deref() {
        Some("apply") => {}
        _ => return Err(Failure::new(EXIT_USAGE, usage())),
    }
    let mut router = None;
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
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => {
                        return Err(Failure::new(
                            EXIT_USAGE,
                            format!("unsupported route apply format: {value}"),
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
                    format!("unknown route apply option: {argument}"),
                ))
            }
        }
    }
    Ok(Options { router, format })
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
            .unwrap_or("route apply failed")
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
            .unwrap_or("routes applied"),
        envelope["summary"]["changed"].as_bool().unwrap_or(false)
    );
    for router in envelope["routers"].as_array().into_iter().flatten() {
        println!(
            "  {}: {} network(s), {}",
            router["vm_id"].as_str().unwrap_or("?"),
            router["networks"].as_array().map(Vec::len).unwrap_or(0),
            if router["changed"].as_bool().unwrap_or(false) {
                "changed"
            } else {
                "unchanged"
            }
        );
    }
}

fn emit_failure(format: Format, failure: &Failure) {
    match format {
        Format::Human => eprintln!("{}", failure.message),
        Format::Json => eprintln!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": "route.apply",
                "status": "error",
                "exit_code": failure.code,
                "error": { "message": failure.message },
            })
        ),
    }
}

fn usage() -> &'static str {
    "usage: vzctl route apply [--router <vm-id>] [--format human|json]"
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
    fn parses_apply_and_optional_router() {
        let options = parse(
            ["apply", "--router", "edge-router", "--format", "json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(
            options,
            Options {
                router: Some("edge-router".to_string()),
                format: Format::Json,
            }
        );
    }

    #[test]
    fn rejects_unknown_route_command() {
        let failure = parse(["list"].into_iter().map(str::to_string)).unwrap_err();
        assert_eq!(failure.code, EXIT_USAGE);
    }
}
