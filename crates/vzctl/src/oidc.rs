//! Embedded Dex OIDC CLI (v0.2 / #46).

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_FAILED: u8 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

pub(crate) fn command(
    args: impl Iterator<Item = String>,
    state_dir: &Path,
    _socket: &Path,
) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let mut iter = args.into_iter();
    let Some(subcommand) = iter.next() else {
        usage();
        return ExitCode::from(EXIT_USAGE);
    };
    match subcommand.as_str() {
        "status" => match parse_format(iter) {
            Ok(format) => status(format, state_dir),
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        "clients" => match parse_format(iter) {
            Ok(format) => clients(format, state_dir),
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        "token" => {
            eprintln!("vzctl oidc token: use Auth Code + PKCE via browser against the issuer");
            ExitCode::from(EXIT_USAGE)
        }
        "-h" | "--help" | "help" => {
            usage();
            ExitCode::from(EXIT_USAGE)
        }
        other => {
            eprintln!("unknown oidc subcommand: {other}");
            usage();
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn usage() {
    eprintln!(
        "usage: vzctl oidc status [--format human|json]
       vzctl oidc clients [--project P] [--format human|json]
       vzctl oidc token [--client ID]"
    );
}

fn parse_format(mut args: impl Iterator<Item = String>) -> Result<(Format, Option<String>), String> {
    let mut format = Format::Human;
    let mut project = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => return Err(format!("unsupported format: {value}")),
                    None => return Err("--format requires human or json".into()),
                };
            }
            "--project" => {
                project = Some(
                    args.next()
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| "--project requires a value".to_string())?,
                );
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok((format, project))
}

pub(crate) fn project_oidc_dir(state_dir: &Path, project: &str) -> PathBuf {
    state_dir.join("projects").join(project).join("oidc")
}

pub(crate) fn clients_path(state_dir: &Path, project: &str) -> PathBuf {
    project_oidc_dir(state_dir, project).join("clients.json")
}

fn status(format_project: (Format, Option<String>), state_dir: &Path) -> ExitCode {
    let (format, project) = format_project;
    let runtime = state_dir.join("runtime").join("oidc");
    let pid_path = runtime.join("dex.pid");
    let running = pid_path
        .exists()
        .then(|| fs::read_to_string(&pid_path).ok())
        .flatten()
        .and_then(|raw| raw.trim().parse::<i32>().ok())
        .map(|pid| path_exists(&format!("/proc/{pid}")).then_some(pid).or_else(|| {
            // macOS: check with kill -0
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .ok()
                .filter(|s| s.success())
                .map(|_| pid)
        }))
        .flatten();
    let data = json!({
        "running": running.is_some(),
        "pid": running,
        "project": project,
        "runtime": runtime,
    });
    emit(format, "oidc.status", data);
    ExitCode::SUCCESS
}

fn clients(format_project: (Format, Option<String>), state_dir: &Path) -> ExitCode {
    let (format, project) = format_project;
    let Some(project) = project else {
        eprintln!("oidc clients requires --project");
        return ExitCode::from(EXIT_USAGE);
    };
    let path = clients_path(state_dir, &project);
    let data = if path.exists() {
        match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("parse clients.json: {e}");
                    return ExitCode::from(EXIT_FAILED);
                }
            },
            Err(e) => {
                eprintln!("read clients.json: {e}");
                return ExitCode::from(EXIT_FAILED);
            }
        }
    } else {
        json!({ "clients": [] })
    };
    emit(format, "oidc.clients", data);
    ExitCode::SUCCESS
}

fn path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn emit(format: Format, command: &str, data: Value) {
    match format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "apiVersion": API_VERSION,
                    "kind": "Result",
                    "ok": true,
                    "command": command,
                    "data": data,
                }))
                .unwrap()
            );
        }
        Format::Human => {
            println!("{}", serde_json::to_string_pretty(&data).unwrap());
        }
    }
}

/// Generate Dex static client entries for `clients: auto`.
pub(crate) fn auto_clients(
    project: &str,
    domain: &str,
    vm_names: &[String],
    route_hosts: &[(String, Vec<String>)],
) -> Value {
    let mut clients = Vec::new();
    for name in vm_names {
        let host = route_hosts
            .iter()
            .find(|(h, reqs)| reqs.iter().any(|r| r == "oidc") && h.contains(name.as_str()))
            .map(|(h, _)| h.clone())
            .unwrap_or_else(|| format!("{name}.svc.{domain}"));
        let secret = generate_secret(name);
        clients.push(json!({
            "id": name,
            "secret": secret,
            "redirectURIs": [format!("https://{host}/oauth2/callback")],
            "name": name,
            "public": false,
        }));
    }
    for (host, reqs) in route_hosts {
        if !reqs.iter().any(|r| r == "oidc") {
            continue;
        }
        let short = host.split('.').next().unwrap_or(host);
        if clients.iter().any(|c| c["id"] == short) {
            continue;
        }
        let secret = generate_secret(short);
        clients.push(json!({
            "id": short,
            "secret": secret,
            "redirectURIs": [format!("https://{host}/oauth2/callback")],
            "name": short,
            "public": false,
        }));
    }
    json!({
        "project": project,
        "issuer": format!("https://auth.svc.{domain}"),
        "clients": clients,
    })
}

fn generate_secret(seed: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"vzctl-oidc-v1:");
    hasher.update(seed.as_bytes());
    hasher.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos().to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("{:x}", hasher.finalize())
}

/// Persist clients JSON (secrets) under Application Support.
pub(crate) fn write_clients(state_dir: &Path, project: &str, data: &Value) -> Result<PathBuf, String> {
    let dir = project_oidc_dir(state_dir, project);
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = clients_path(state_dir, project);
    let raw = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("write {}: {e}", path.display()))?;
    let _ = fs::set_permissions(
        &path,
        fs::Permissions::from_mode(0o600),
    );
    Ok(path)
}

use std::os::unix::fs::PermissionsExt;

/// Render a minimal Dex config YAML.
pub(crate) fn render_dex_config(
    issuer: &str,
    listen: &str,
    clients: &Value,
    password_file: Option<&Path>,
    storage_dir: &Path,
) -> Result<String, String> {
    let mut static_clients = String::from("staticClients:\n");
    let list = clients
        .get("clients")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        static_clients.push_str("  []\n");
    } else {
        for client in &list {
            let id = client["id"].as_str().unwrap_or("client");
            let secret = client["secret"].as_str().unwrap_or("secret");
            let name = client["name"].as_str().unwrap_or(id);
            static_clients.push_str(&format!("  - id: {id}\n"));
            static_clients.push_str(&format!("    name: {name}\n"));
            static_clients.push_str(&format!("    secret: {secret}\n"));
            static_clients.push_str("    redirectURIs:\n");
            if let Some(uris) = client["redirectURIs"].as_array() {
                for uri in uris {
                    if let Some(u) = uri.as_str() {
                        static_clients.push_str(&format!("      - {u}\n"));
                    }
                }
            }
        }
    }

    let passwords = if let Some(path) = password_file.filter(|p| p.exists()) {
        let content = fs::read_to_string(path).map_err(|e| format!("read passwordFile: {e}"))?;
        format!("enablePasswordDB: true\nstaticPasswords:\n{content}")
    } else {
        // Dev default user (bcrypt for "password") — only when no file provided.
        String::from(
            r#"enablePasswordDB: true
staticPasswords:
  - email: "admin@vzctl.local"
    hash: "$2a$10$2b2cU8CPhOTaGrs1HRQuAueS7JTT5ZHsHSzYiFPm1leZck7FQjAha"
    username: "admin"
    userID: "08a8684b-db88-4b73-90a9-3cd1661f5466"
"#,
        )
    };

    Ok(format!(
        r#"issuer: {issuer}
storage:
  type: sqlite3
  config:
    file: {storage}/dex.db
web:
  http: {listen}
oauth2:
  skipApprovalScreen: true
{static_clients}
{passwords}
"#,
        issuer = issuer,
        listen = listen,
        storage = storage_dir.display(),
        static_clients = static_clients,
        passwords = passwords,
    ))
}
