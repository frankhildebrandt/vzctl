mod nats;
mod prereq;
mod sources;
mod sql;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use self::nats::{
    build_kv_get_argv, build_kv_watch_argv, build_streams_argv, build_sub_argv, plan_nats_exec,
};
use self::prereq::{doctor_json, ensure_data_sources, exec_json, exec_with_timing};
use self::sources::{
    list_sources_filtered, load_stack, resolve_nats_source, resolve_sql_source, source_json,
    sql_env, SourceType,
};
use self::sql::{
    build_mysql_argv, build_psql_argv, parse_mysql_tsv, parse_psql_tsv, validate_read_only_sql,
};

const API_VERSION: &str = "vzctl.dev/v1";
const DEFAULT_CONFIG: &str = "hypernetwork.config.yaml";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_SUPERVISOR: u8 = 10;
pub(crate) const EXIT_DEBUG: u8 = 26;

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

pub(crate) fn command(args: impl Iterator<Item = String>, socket_path: &Path) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let command_hint = match args.first().map(String::as_str) {
        Some("sources") => "debug.sources",
        Some("doctor") => "debug.doctor",
        Some("postgres") => "debug.postgres",
        Some("mysql") => "debug.mysql",
        Some("nats") => "debug.nats",
        _ => "debug",
    };
    match parse(args.into_iter()) {
        Ok(options) => run(options, socket_path),
        Err(failure) => {
            emit_failure(requested_format, command_hint, &failure);
            ExitCode::from(failure.code)
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Action {
    SourcesList {
        filter: Option<SourceType>,
    },
    Doctor,
    PostgresQuery {
        source: Option<String>,
        sql: String,
        max_rows: u32,
        timeout_ms: u64,
    },
    MysqlQuery {
        source: Option<String>,
        sql: String,
        max_rows: u32,
        timeout_ms: u64,
    },
    NatsStreams {
        source: Option<String>,
    },
    NatsSub {
        source: Option<String>,
        subject: String,
        count: u32,
        timeout_ms: u64,
    },
    NatsKvGet {
        source: Option<String>,
        bucket: String,
        key: String,
    },
    NatsKvWatch {
        source: Option<String>,
        bucket: String,
        key: Option<String>,
        timeout_ms: u64,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    config: PathBuf,
    format: Format,
    action: Action,
}

fn run(options: Options, socket_path: &Path) -> ExitCode {
    let stack = match load_stack(&options.config) {
        Ok(stack) => stack,
        Err(message) => {
            emit_failure(
                options.format,
                command_for_action(&options.action),
                &Failure::new(EXIT_INVALID, message),
            );
            return ExitCode::from(EXIT_INVALID);
        }
    };
    if let Err(message) = ensure_data_sources(&stack.environment.spec.observability.data_sources) {
        if !matches!(options.action, Action::SourcesList { .. }) {
            emit_failure(
                options.format,
                command_for_action(&options.action),
                &Failure::new(EXIT_INVALID, message),
            );
            return ExitCode::from(EXIT_INVALID);
        }
    }

    match options.action {
        Action::SourcesList { filter } => sources_list(&stack, filter, options.format),
        Action::Doctor => doctor(&stack, socket_path, options.format),
        Action::PostgresQuery {
            source,
            sql,
            max_rows,
            timeout_ms,
        } => sql_query(
            &stack,
            SourceType::Postgres,
            source.as_deref(),
            sql,
            max_rows,
            timeout_ms,
            socket_path,
            options.format,
        ),
        Action::MysqlQuery {
            source,
            sql,
            max_rows,
            timeout_ms,
        } => sql_query(
            &stack,
            SourceType::Mysql,
            source.as_deref(),
            sql,
            max_rows,
            timeout_ms,
            socket_path,
            options.format,
        ),
        Action::NatsStreams { source } => {
            nats_streams(&stack, source.as_deref(), socket_path, options.format)
        }
        Action::NatsSub {
            source,
            subject,
            count,
            timeout_ms,
        } => nats_sub(
            &stack,
            source.as_deref(),
            &subject,
            count,
            timeout_ms,
            socket_path,
            options.format,
        ),
        Action::NatsKvGet {
            source,
            bucket,
            key,
        } => nats_kv_get(
            &stack,
            source.as_deref(),
            &bucket,
            &key,
            socket_path,
            options.format,
        ),
        Action::NatsKvWatch {
            source,
            bucket,
            key,
            timeout_ms,
        } => nats_kv_watch(
            &stack,
            source.as_deref(),
            &bucket,
            key.as_deref(),
            timeout_ms,
            socket_path,
            options.format,
        ),
    }
}

fn sources_list(
    stack: &sources::LoadedStack,
    filter: Option<SourceType>,
    format: Format,
) -> ExitCode {
    let payload = list_sources_filtered(&stack.environment, filter);
    emit_success(
        format,
        "debug.sources.list",
        json!({
            "summary": { "message": "data sources listed" },
            "sources": payload,
        }),
    );
    ExitCode::SUCCESS
}

fn doctor(stack: &sources::LoadedStack, socket_path: &Path, format: Format) -> ExitCode {
    match doctor_json(stack, socket_path) {
        Ok(payload) => {
            let ok = payload["ok"].as_bool().unwrap_or(false);
            emit_success(
                format,
                "debug.doctor",
                json!({
                    "summary": {
                        "message": if ok { "debug prerequisites ok" } else { "debug prerequisites failed" },
                        "ok": ok,
                    },
                    "doctor": payload,
                }),
            );
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(EXIT_DEBUG)
            }
        }
        Err(message) => {
            emit_failure(format, "debug.doctor", &Failure::new(EXIT_DEBUG, message));
            ExitCode::from(EXIT_DEBUG)
        }
    }
}

fn sql_query(
    stack: &sources::LoadedStack,
    source_type: SourceType,
    source_name: Option<&str>,
    sql: String,
    max_rows: u32,
    timeout_ms: u64,
    socket_path: &Path,
    format: Format,
) -> ExitCode {
    let command = match source_type {
        SourceType::Postgres => "debug.postgres.query",
        SourceType::Mysql => "debug.mysql.query",
        SourceType::Nats => "debug.query",
    };
    let sql = match validate_read_only_sql(&sql) {
        Ok(value) => value,
        Err(message) => {
            emit_failure(format, command, &Failure::new(EXIT_INVALID, message));
            return ExitCode::from(EXIT_INVALID);
        }
    };
    let limited = if sql.to_ascii_uppercase().contains("LIMIT") {
        sql
    } else {
        format!("{sql} LIMIT {}", max_rows.max(1))
    };
    let resolved = match resolve_sql_source(stack, source_type, source_name) {
        Ok(value) => value,
        Err(message) => {
            emit_failure(format, command, &Failure::new(EXIT_INVALID, message));
            return ExitCode::from(EXIT_INVALID);
        }
    };
    let argv = match source_type {
        SourceType::Postgres => build_psql_argv(
            &resolved.host,
            resolved.port,
            &resolved.user,
            &resolved.database,
            &limited,
        ),
        SourceType::Mysql => build_mysql_argv(
            &resolved.host,
            resolved.port,
            &resolved.user,
            &resolved.database,
            &limited,
        ),
        SourceType::Nats => unreachable!(),
    };
    let env = sql_env(&resolved);
    let (outcome, duration_ms) =
        match exec_with_timing(&resolved.via.vm_id, &argv, &env, timeout_ms, socket_path) {
            Ok(value) => value,
            Err(message) => {
                emit_failure(format, command, &Failure::new(EXIT_SUPERVISOR, message));
                return ExitCode::from(EXIT_SUPERVISOR);
            }
        };
    if outcome.exit != 0 {
        emit_failure_payload(
            format,
            command,
            EXIT_DEBUG,
            json!({
                "summary": {
                    "message": format!("guest exec failed with exit {}", outcome.exit),
                    "exit": outcome.exit,
                },
                "source": source_json(source_type, &resolved.name, &resolved.via),
                "sql": limited,
                "exec": exec_json(&outcome, duration_ms),
            }),
            outcome.stderr.trim(),
        );
        return ExitCode::from(EXIT_DEBUG);
    }
    let (columns, rows) = match source_type {
        SourceType::Postgres => parse_psql_tsv(&outcome.stdout),
        SourceType::Mysql => parse_mysql_tsv(&outcome.stdout),
        SourceType::Nats => unreachable!(),
    };
    let truncated = rows.len() >= max_rows as usize;
    emit_success(
        format,
        command,
        json!({
            "summary": {
                "message": format!("{} rows", rows.len()),
                "rowCount": rows.len(),
            },
            "source": source_json(source_type, &resolved.name, &resolved.via),
            "sql": limited,
            "result": {
                "columns": columns,
                "rows": rows,
                "rowCount": rows.len(),
                "truncated": truncated,
            },
            "exec": exec_json(&outcome, duration_ms),
        }),
    );
    ExitCode::SUCCESS
}

fn nats_streams(
    stack: &sources::LoadedStack,
    source_name: Option<&str>,
    socket_path: &Path,
    format: Format,
) -> ExitCode {
    run_nats(
        stack,
        source_name,
        build_streams_argv(),
        "debug.nats.streams",
        socket_path,
        format,
        |outcome, duration_ms, source| {
            json!({
                "summary": { "message": "nats streams listed" },
                "source": source,
                "streams": parse_json_or_text(&outcome.stdout),
                "exec": exec_json(outcome, duration_ms),
            })
        },
    )
}

fn nats_sub(
    stack: &sources::LoadedStack,
    source_name: Option<&str>,
    subject: &str,
    count: u32,
    timeout_ms: u64,
    socket_path: &Path,
    format: Format,
) -> ExitCode {
    let timeout_secs = (timeout_ms / 1000).max(1);
    run_nats(
        stack,
        source_name,
        build_sub_argv(subject, count.max(1), timeout_secs),
        "debug.nats.sub",
        socket_path,
        format,
        |outcome, duration_ms, source| {
            json!({
                "summary": { "message": "nats subscribe finished" },
                "source": source,
                "subject": subject,
                "messages": parse_ndjson_or_lines(&outcome.stdout),
                "exec": exec_json(outcome, duration_ms),
            })
        },
    )
}

fn nats_kv_get(
    stack: &sources::LoadedStack,
    source_name: Option<&str>,
    bucket: &str,
    key: &str,
    socket_path: &Path,
    format: Format,
) -> ExitCode {
    run_nats(
        stack,
        source_name,
        build_kv_get_argv(bucket, key),
        "debug.nats.kv.get",
        socket_path,
        format,
        |outcome, duration_ms, source| {
            json!({
                "summary": { "message": "nats kv get finished" },
                "source": source,
                "bucket": bucket,
                "key": key,
                "value": parse_json_or_text(&outcome.stdout),
                "exec": exec_json(outcome, duration_ms),
            })
        },
    )
}

fn nats_kv_watch(
    stack: &sources::LoadedStack,
    source_name: Option<&str>,
    bucket: &str,
    key: Option<&str>,
    timeout_ms: u64,
    socket_path: &Path,
    format: Format,
) -> ExitCode {
    let timeout_secs = (timeout_ms / 1000).max(1);
    run_nats(
        stack,
        source_name,
        build_kv_watch_argv(bucket, key, timeout_secs),
        "debug.nats.kv.watch",
        socket_path,
        format,
        |outcome, duration_ms, source| {
            json!({
                "summary": { "message": "nats kv watch finished" },
                "source": source,
                "bucket": bucket,
                "key": key,
                "events": parse_ndjson_or_lines(&outcome.stdout),
                "exec": exec_json(outcome, duration_ms),
            })
        },
    )
}

fn run_nats(
    stack: &sources::LoadedStack,
    source_name: Option<&str>,
    inner_argv: Vec<String>,
    command: &'static str,
    socket_path: &Path,
    format: Format,
    build_payload: impl FnOnce(&crate::vm::ExecOutcome, u64, Value) -> Value,
) -> ExitCode {
    let resolved = match resolve_nats_source(stack, source_name) {
        Ok(value) => value,
        Err(message) => {
            emit_failure(format, command, &Failure::new(EXIT_INVALID, message));
            return ExitCode::from(EXIT_INVALID);
        }
    };
    let plan = match plan_nats_exec(&resolved, inner_argv) {
        Ok(value) => value,
        Err(message) => {
            emit_failure(format, command, &Failure::new(EXIT_INVALID, message));
            return ExitCode::from(EXIT_INVALID);
        }
    };
    let (outcome, duration_ms) =
        match exec_with_timing(&plan.via.vm_id, &plan.cmd, &plan.env, 120_000, socket_path) {
            Ok(value) => value,
            Err(message) => {
                emit_failure(format, command, &Failure::new(EXIT_SUPERVISOR, message));
                return ExitCode::from(EXIT_SUPERVISOR);
            }
        };
    let source = source_json(SourceType::Nats, &resolved.name, &plan.via);
    if outcome.exit != 0 {
        emit_failure_payload(
            format,
            command,
            EXIT_DEBUG,
            json!({
                "summary": {
                    "message": format!("guest exec failed with exit {}", outcome.exit),
                    "exit": outcome.exit,
                },
                "source": source,
                "exec": exec_json(&outcome, duration_ms),
            }),
            outcome.stderr.trim(),
        );
        return ExitCode::from(EXIT_DEBUG);
    }
    emit_success(
        format,
        command,
        build_payload(&outcome, duration_ms, source),
    );
    ExitCode::SUCCESS
}

fn parse_json_or_text(stdout: &str) -> Value {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| json!({ "text": trimmed }))
}

fn parse_ndjson_or_lines(stdout: &str) -> Value {
    let lines: Vec<Value> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_json_or_text(line))
        .collect();
    json!(lines)
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let args: Vec<String> = args.collect();
    let sub = args.first().map(String::as_str);
    let mut config = PathBuf::from(DEFAULT_CONFIG);
    let mut format = Format::Human;
    let mut source = None;
    let mut max_rows = 100_u32;
    let mut timeout_ms = 30_000_u64;
    let mut count = 10_u32;
    let mut subject = None;
    let mut bucket = None;
    let mut key = None;
    let mut filter = None;
    let mut tail: Vec<String> = Vec::new();
    let mut index = 1_usize;
    while index < args.len() {
        match args[index].as_str() {
            "-C" => {
                index += 1;
                config = PathBuf::from(next_arg(&args, index, "-C")?);
            }
            "--format" => {
                index += 1;
                format = match next_arg(&args, index, "--format")?.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(Failure::new(
                            EXIT_USAGE,
                            format!("unsupported debug format: {other}"),
                        ))
                    }
                };
            }
            "--source" => {
                index += 1;
                source = Some(next_arg(&args, index, "--source")?);
            }
            "--type" => {
                index += 1;
                filter = Some(
                    SourceType::parse(&next_arg(&args, index, "--type")?)
                        .map_err(|message| Failure::new(EXIT_USAGE, message))?,
                );
            }
            "--max-rows" => {
                index += 1;
                max_rows = next_arg(&args, index, "--max-rows")?
                    .parse()
                    .map_err(|_| Failure::new(EXIT_USAGE, "--max-rows requires an integer"))?;
            }
            "--timeout" | "--timeout-ms" => {
                index += 1;
                let raw = next_arg(&args, index, args[index - 1].as_str())?;
                timeout_ms = parse_timeout_ms(&raw)?;
            }
            "--count" => {
                index += 1;
                count = next_arg(&args, index, "--count")?
                    .parse()
                    .map_err(|_| Failure::new(EXIT_USAGE, "--count requires an integer"))?;
            }
            "--subject" => {
                index += 1;
                subject = Some(next_arg(&args, index, "--subject")?);
            }
            "--bucket" => {
                index += 1;
                bucket = Some(next_arg(&args, index, "--bucket")?);
            }
            "--key" => {
                index += 1;
                key = Some(next_arg(&args, index, "--key")?);
            }
            "--" => {
                tail.extend(args[index + 1..].iter().cloned());
                break;
            }
            other if other.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown debug option: {other}"),
                ));
            }
            other => tail.push(other.to_string()),
        }
        index += 1;
    }

    let action = match sub {
        Some("sources") if tail.first().map(String::as_str) == Some("list") => {
            Action::SourcesList { filter }
        }
        Some("doctor") => Action::Doctor,
        Some("postgres") if tail.first().map(String::as_str) == Some("query") => {
            let sql = tail.get(1).cloned().ok_or_else(|| {
                Failure::new(
                    EXIT_USAGE,
                    "usage: vzctl debug postgres query [options] -- SQL",
                )
            })?;
            Action::PostgresQuery {
                source,
                sql,
                max_rows,
                timeout_ms,
            }
        }
        Some("mysql") if tail.first().map(String::as_str) == Some("query") => {
            let sql = tail.get(1).cloned().ok_or_else(|| {
                Failure::new(
                    EXIT_USAGE,
                    "usage: vzctl debug mysql query [options] -- SQL",
                )
            })?;
            Action::MysqlQuery {
                source,
                sql,
                max_rows,
                timeout_ms,
            }
        }
        Some("nats") => match tail.first().map(String::as_str) {
            Some("streams") => Action::NatsStreams { source },
            Some("sub") => Action::NatsSub {
                source,
                subject: subject
                    .ok_or_else(|| Failure::new(EXIT_USAGE, "nats sub requires --subject"))?,
                count,
                timeout_ms,
            },
            Some("kv") => match tail.get(1).map(String::as_str) {
                Some("get") => Action::NatsKvGet {
                    source,
                    bucket: bucket
                        .ok_or_else(|| Failure::new(EXIT_USAGE, "kv get requires --bucket"))?,
                    key: key.ok_or_else(|| Failure::new(EXIT_USAGE, "kv get requires --key"))?,
                },
                Some("watch") => Action::NatsKvWatch {
                    source,
                    bucket: bucket
                        .ok_or_else(|| Failure::new(EXIT_USAGE, "kv watch requires --bucket"))?,
                    key,
                    timeout_ms,
                },
                _ => {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "usage: vzctl debug nats kv get|watch ...",
                    ))
                }
            },
            _ => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    "usage: vzctl debug nats streams|sub|kv ...",
                ))
            }
        },
        _ => {
            return Err(Failure::new(
                EXIT_USAGE,
                "usage: vzctl debug sources list|doctor|postgres query|mysql query|nats ...",
            ))
        }
    };
    Ok(Options {
        config,
        format,
        action,
    })
}

fn parse_timeout_ms(raw: &str) -> Result<u64, Failure> {
    if let Some(stripped) = raw.strip_suffix('s') {
        let secs: u64 = stripped
            .parse()
            .map_err(|_| Failure::new(EXIT_USAGE, "invalid --timeout value"))?;
        return Ok(secs.saturating_mul(1000));
    }
    raw.parse()
        .map_err(|_| Failure::new(EXIT_USAGE, "invalid --timeout-ms value"))
}

fn next_arg(args: &[String], index: usize, flag: &str) -> Result<String, Failure> {
    args.get(index)
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_USAGE, format!("{flag} requires a value")))
}

fn requested_format(args: &[String]) -> Format {
    let mut format = Format::Human;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--format" {
            if let Some(value) = iter.next() {
                format = match value.as_str() {
                    "json" => Format::Json,
                    _ => Format::Human,
                };
            }
        }
    }
    format
}

fn command_for_action(action: &Action) -> &'static str {
    match action {
        Action::SourcesList { .. } => "debug.sources.list",
        Action::Doctor => "debug.doctor",
        Action::PostgresQuery { .. } => "debug.postgres.query",
        Action::MysqlQuery { .. } => "debug.mysql.query",
        Action::NatsStreams { .. } => "debug.nats.streams",
        Action::NatsSub { .. } => "debug.nats.sub",
        Action::NatsKvGet { .. } => "debug.nats.kv.get",
        Action::NatsKvWatch { .. } => "debug.nats.kv.watch",
    }
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn emit_success(format: Format, command: &str, payload: Value) {
    match format {
        Format::Json => {
            let mut envelope = payload.as_object().cloned().unwrap_or_default();
            envelope.insert("apiVersion".into(), json!(API_VERSION));
            envelope.insert("command".into(), json!(command));
            envelope.insert("status".into(), json!("ok"));
            envelope.insert("exit_code".into(), json!(0));
            println!(
                "{}",
                serde_json::to_string_pretty(&Value::Object(envelope)).unwrap()
            );
        }
        Format::Human => {
            if let Some(message) = payload["summary"]["message"].as_str() {
                println!("{message}");
            }
        }
    }
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    emit_failure_payload(
        format,
        command,
        failure.code,
        json!({
            "summary": { "message": failure.message },
        }),
        failure.message.as_str(),
    );
}

fn emit_failure_payload(
    format: Format,
    command: &str,
    exit_code: u8,
    payload: Value,
    stderr_hint: &str,
) {
    match format {
        Format::Json => {
            let mut envelope = payload.as_object().cloned().unwrap_or_default();
            envelope.insert("apiVersion".into(), json!(API_VERSION));
            envelope.insert("command".into(), json!(command));
            envelope.insert("status".into(), json!("fail"));
            envelope.insert("exit_code".into(), json!(exit_code));
            println!(
                "{}",
                serde_json::to_string_pretty(&Value::Object(envelope)).unwrap()
            );
        }
        Format::Human => {
            eprintln!("{stderr_hint}");
        }
    }
}
