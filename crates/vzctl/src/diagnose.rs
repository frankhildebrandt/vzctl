//! Bounded, allowlisted support bundle. Never collect raw config, logs, env or credentials.
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIMIT: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(3);
#[path = "diagnose/status.rs"]
mod status;

pub(crate) fn command(args: impl Iterator<Item = String>, state: &Path, socket: &Path) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let json_output = args
        .windows(2)
        .any(|p| p[0] == "--format" && p[1] == "json");
    let (output, stack) = match parse(args) {
        Ok(options) => options,
        Err(message) => return emit_error(json_output, 2, &message),
    };
    // create_new avoids overwrite and symlink traversal at the destination.
    let mut file = match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&output) {
        Ok(file) => file,
        Err(_) => return emit_error(json_output, 3, "Ausgabedatei kann nicht neu angelegt werden (existiert bereits oder Verzeichnis fehlt)."),
    };
    let bundle = collect(state, socket, stack.as_deref());
    let encoded = serde_json::to_vec_pretty(&bundle).expect("bundle is JSON");
    if encoded.len() as u64 > LIMIT
        || file
            .write_all(&encoded)
            .and_then(|_| file.sync_all())
            .is_err()
    {
        drop(file);
        let _ = fs::remove_file(&output);
        return emit_error(
            json_output,
            3,
            "Diagnosebundle konnte nicht vollständig geschrieben werden.",
        );
    }
    let warnings = bundle["warnings"].as_array().map(Vec::len).unwrap_or(0);
    if json_output {
        println!(
            "{}",
            json!({"apiVersion":"vzctl.dev/v1","command":"diagnose",
            "status": if warnings == 0 { "ok" } else { "warn" }, "exit_code":0,
            "summary":{"message":"Diagnosebundle erstellt","warnings":warnings,"bytes":encoded.len()},
            "path":output})
        );
    } else {
        println!(
            "Diagnosebundle: {} ({} Bytes, {warnings} Hinweise)",
            output.display(),
            encoded.len()
        );
    }
    ExitCode::SUCCESS
}

fn parse(args: Vec<String>) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut args = args.into_iter();
    let mut output = None;
    let mut stack = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" if output.is_none() => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output benötigt einen Pfad")?,
                ))
            }
            "-C" if stack.is_none() => {
                stack = Some(PathBuf::from(
                    args.next().ok_or("-C benötigt einen Stack-Pfad")?,
                ));
            }
            "--format" => match args.next().as_deref() {
                Some("json" | "human") => {}
                _ => return Err("--format benötigt human|json".into()),
            },
            _ => return Err(format!("Unbekannte oder doppelte Diagnose-Option: {arg}")),
        }
    }
    output.map(|output| (output, stack)).ok_or_else(|| {
        "Aufruf: vzctl diagnose [-C STACK] --output <neue-Datei.json> [--format human|json]".into()
    })
}

fn emit_error(json_output: bool, code: u8, message: &str) -> ExitCode {
    eprintln!("{message}");
    if json_output {
        println!(
            "{}",
            json!({"apiVersion":"vzctl.dev/v1","command":"diagnose","status":"fail",
            "exit_code":code,"summary":{"message":message}})
        );
    }
    ExitCode::from(code)
}

fn collect(state: &Path, socket: &Path, stack: Option<&Path>) -> Value {
    let mut warnings = Vec::new();
    let health = match rpc(socket, "daemon.health") {
        Some(value) => health_summary(&value),
        None => {
            warnings.push("supervisor_health_unavailable");
            Value::Null
        }
    };
    let vms = match rpc(socket, "vm.list") {
        Some(Value::Array(vms)) => Value::Array(
            vms.iter()
                .take(64)
                .map(|vm| selected(vm, &["vm_id", "state", "pid"]))
                .collect(),
        ),
        _ => {
            warnings.push("vm_inventory_unavailable");
            Value::Null
        }
    };
    let journal = match journal_summary(&state.join("state.sqlite")) {
        Some(value) => value,
        None => {
            warnings.push("journal_summary_unavailable");
            Value::Null
        }
    };
    let services = ["net.sock", "edge.sock", "vz.sock"].map(|name| {
        let path = if name == "vz.sock" {
            socket.to_path_buf()
        } else {
            state.join(name)
        };
        json!({"service":name.trim_end_matches(".sock"),"socket_present":
            fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())})
    });
    let (controllers, kubernetes) = if let Some(stack) = stack {
        let executable = std::env::current_exe().ok();
        let mut section = |group| match executable
            .as_deref()
            .and_then(|exe| status::collect(exe, group, stack, state, TIMEOUT).ok())
        {
            Some(value) => {
                if value["status"] != "ok" {
                    warnings.push(if group == "controller" {
                        "controller_status_degraded"
                    } else {
                        "kubernetes_status_degraded"
                    });
                }
                let projected = status::project(&value, group);
                if projected["truncated"] == true
                    || projected["clusters"].as_array().is_some_and(|clusters| {
                        clusters.iter().any(|cluster| cluster["truncated"] == true)
                    })
                {
                    warnings.push(if group == "controller" {
                        "controller_status_truncated"
                    } else {
                        "kubernetes_status_truncated"
                    });
                }
                projected
            }
            None => {
                warnings.push(if group == "controller" {
                    "controller_status_unavailable"
                } else {
                    "kubernetes_status_unavailable"
                });
                json!({"state":"unavailable","code":"StatusCollectionUnavailable"})
            }
        };
        (section("controller"), section("kubernetes"))
    } else {
        warnings.push("stack_status_not_requested");
        (
            json!({"state":"not-requested"}),
            json!({"state":"not-requested"}),
        )
    };
    json!({
        "apiVersion":"vzctl.dev/diagnose/v1",
        "created_at_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "cli_version":env!("CARGO_PKG_VERSION"),
        "host":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "logical_cpus":std::thread::available_parallelism().ok().map(|n| n.get()),
            "memory_mib":crate::resource_budget::host_memory_mib()},
        "services":services,"supervisor":health,"vms":vms,"journals":journal,
        "controllers":controllers,"kubernetes":kubernetes,
        "warnings":warnings,
        "collection":{"max_bundle_bytes":LIMIT,"max_vms":64,"max_journals":32,
            "stack_requested":stack.is_some(),"max_stack_section_bytes":16384,
            "raw_logs":false,"config_contents":false,"environment":false,"credentials":false},
    })
}

fn health_summary(value: &Value) -> Value {
    let mut result = selected(
        value,
        &[
            "ok",
            "version",
            "pid",
            "uptime_ms",
            "db_ok",
            "networks",
            "network_orphans",
            "dns_ok",
            "vz_edge_ok",
            "vz_net_ok",
        ],
    );
    result["network_resilience"] = selected(&value["network_resilience"], &["state"]);
    result
}

fn selected(value: &Value, keys: &[&str]) -> Value {
    let mut result = serde_json::Map::new();
    for key in keys {
        let value = match &value[*key] {
            Value::Bool(v) => json!(v),
            Value::Number(v) => json!(v),
            Value::String(v) => json!(safe_text(v)),
            _ => Value::Null,
        };
        result.insert((*key).to_string(), value);
    }
    Value::Object(result)
}

fn safe_text(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if [
        "secret",
        "password",
        "token",
        "authorization",
        "private key",
        "://",
        "@",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return "[redacted]".into();
    }
    value.chars().filter(|c| !c.is_control()).take(96).collect()
}

fn rpc(socket: &Path, method: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc":"2.0","method":method,"params":{},"id":1})
    )
    .ok()?;
    stream.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + TIMEOUT;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.len() as u64 > LIMIT {
                    return None;
                }
                if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                    let response: Value = serde_json::from_slice(&bytes[..end]).ok()?;
                    return response.get("result").cloned();
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return None,
        }
    }
}

fn journal_summary(path: &Path) -> Option<Value> {
    if !path.is_file() {
        return None;
    }
    // Fixed projection excludes payload, error, labels, secrets and filesystem paths.
    let sql = "SELECT stack_id,generation,step,status,updated_at FROM journal ORDER BY status IN ('pending','running','failed') DESC, updated_at DESC LIMIT 32";
    let mut child = Command::new("/usr/bin/sqlite3")
        .args(["-readonly", "-json"])
        .arg(path)
        .arg(sql)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    });
    let deadline = Instant::now() + TIMEOUT;
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader.join().ok()??;
    if !success || bytes.len() as u64 > LIMIT {
        return None;
    }
    if bytes.is_empty() {
        return Some(json!([]));
    }
    let rows: Value = serde_json::from_slice(&bytes).ok()?;
    Some(Value::Array(
        rows.as_array()?
            .iter()
            .take(32)
            .map(|row| {
                selected(
                    row,
                    &["stack_id", "generation", "step", "status", "updated_at"],
                )
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_drops_payloads_and_redacts_sensitive_metadata() {
        let raw = json!({"ok":true,"version":"1.2","pid":123,"secret":"keep-out",
            "logs":["password=hidden"],"network_resilience":{"state":"password=hidden"}});
        let result = health_summary(&raw);
        let text = result.to_string();
        assert_eq!(result["ok"], true);
        assert_eq!(result["network_resilience"]["state"], "[redacted]");
        assert!(!text.contains("hidden"));
        assert!(!text.contains("keep-out"));
        assert!(!text.contains("logs"));
        assert_eq!(safe_text(&"a".repeat(1000)).len(), 96);
    }
    #[test]
    fn journal_projection_is_bounded_and_excludes_private_columns() {
        let path = std::env::temp_dir().join(format!(
            "vz-diag-db-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let status = Command::new("/usr/bin/sqlite3").arg(&path).arg(
            "CREATE TABLE journal(stack_id TEXT,generation INT,step TEXT,status TEXT,updated_at TEXT,payload TEXT,error TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<50) INSERT INTO journal SELECT 'demo:stack',x,'ensure_images','failed','2026-10-20','SECRET-PAYLOAD','SECRET-ERROR' FROM n;"
        ).status().unwrap();
        assert!(status.success());
        let summary = journal_summary(&path).unwrap();
        assert_eq!(summary.as_array().unwrap().len(), 32);
        assert!(!summary.to_string().contains("SECRET"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn destination_is_explicit() {
        assert!(parse(vec![]).is_err());
        assert!(parse(vec!["--output".into()]).is_err());
        assert_eq!(
            parse(vec!["--output".into(), "bundle.json".into()]).unwrap(),
            (PathBuf::from("bundle.json"), None)
        );
    }
}
