use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_SUPERVISOR: u8 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
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
    let mut iter = args.into_iter().peekable();
    let Some(subcommand) = iter.next() else {
        eprintln!("usage: vzctl port list [--project P] [--stack S] [--format human|json]");
        return ExitCode::from(EXIT_USAGE);
    };
    match subcommand.as_str() {
        "list" => match parse_list_options(iter) {
            Ok((format, project, stack)) => list(format, project, stack, socket_path),
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        "-h" | "--help" | "help" => {
            eprintln!("usage: vzctl port list [--project P] [--stack S] [--format human|json]");
            ExitCode::from(EXIT_USAGE)
        }
        other => {
            eprintln!("unknown port subcommand: {other}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn parse_list_options(
    mut args: impl Iterator<Item = String>,
) -> Result<(Format, Option<String>, Option<String>), String> {
    let mut format = Format::Human;
    let mut project = None;
    let mut stack = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => return Err(format!("unsupported port list format: {value}")),
                    None => return Err("--format requires human or json".into()),
                };
            }
            "--project" => {
                project = Some(
                    args.next()
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| "--project requires a value".to_string())?,
                );
            }
            "--stack" => {
                stack = Some(
                    args.next()
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| "--stack requires a value".to_string())?,
                );
            }
            other => return Err(format!("unknown port list option: {other}")),
        }
    }
    Ok((format, project, stack))
}

fn list(
    format: Format,
    project: Option<String>,
    stack: Option<String>,
    socket_path: &Path,
) -> ExitCode {
    let mut params = serde_json::Map::new();
    if let Some(project) = project {
        params.insert("project".into(), json!(project));
    }
    if let Some(stack) = stack {
        params.insert("stack".into(), json!(stack));
    }
    match rpc(socket_path, "port.list", Value::Object(params)) {
        Ok(result) => {
            let ports = result.get("ports").cloned().unwrap_or_else(|| json!([]));
            match format {
                Format::Human => {
                    if let Some(items) = ports.as_array() {
                        if items.is_empty() {
                            println!("No port forwards");
                        } else {
                            println!(
                                "{:<17} {:<18} {:<12} {}",
                                "BIND", "TARGET", "STATE", "SOURCE"
                            );
                            for item in items {
                                println!(
                                    "{:<17} {:<18} {:<12} {}",
                                    format!(
                                        "{}:{}",
                                        item["bind"].as_str().unwrap_or("127.0.0.1"),
                                        item["host_port"].as_u64().unwrap_or(0)
                                    ),
                                    format!(
                                        "{}:{}",
                                        item["guest_ip"].as_str().unwrap_or("?"),
                                        item["guest_port"].as_u64().unwrap_or(0)
                                    ),
                                    item["state"].as_str().unwrap_or("unknown"),
                                    item["source"].as_str().unwrap_or("-"),
                                );
                            }
                        }
                    }
                    ExitCode::SUCCESS
                }
                Format::Json => {
                    let count = ports.as_array().map(Vec::len).unwrap_or(0);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "apiVersion": API_VERSION,
                            "command": "port.list",
                            "status": "ok",
                            "exit_code": 0,
                            "summary": { "count": count },
                            "ports": ports,
                        }))
                        .expect("json")
                    );
                    ExitCode::SUCCESS
                }
            }
        }
        Err(failure) => {
            eprintln!("{}", failure.message);
            if format == Format::Json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "apiVersion": API_VERSION,
                        "command": "port.list",
                        "status": "fail",
                        "exit_code": failure.code,
                        "summary": { "message": failure.message },
                    }))
                    .expect("json")
                );
            }
            ExitCode::from(failure.code)
        }
    }
}

fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot connect to supervisor: {error}"),
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    writeln!(stream, "{request}").map_err(|error| {
        Failure::new(EXIT_SUPERVISOR, format!("supervisor write failed: {error}"))
    })?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|error| {
            Failure::new(EXIT_SUPERVISOR, format!("supervisor read failed: {error}"))
        })?;
    let response: Value = serde_json::from_str(&line).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("invalid supervisor response: {error}"),
        )
    })?;
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("supervisor error");
        let code = error
            .get("data")
            .and_then(|data| data.get("exit_code"))
            .and_then(Value::as_u64)
            .unwrap_or(EXIT_SUPERVISOR as u64) as u8;
        return Err(Failure::new(code, message.to_string()));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}
