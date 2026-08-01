//! Embedded Dex OIDC CLI (v0.2 / #46) + OIDC uplink federator.

use crate::config::OidcUplink;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_FAILED: u8 = 10;
const DEFAULT_SCOPES: &[&str] = &["openid", "profile", "email"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

/// Host-wide uplink defaults under Application Support.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostOidcConfig {
    #[serde(default)]
    pub(crate) uplink: Option<OidcUplink>,
}

/// Fully resolved uplink ready for Dex `connectors:`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedUplink {
    pub(crate) uplink_type: crate::config::OidcUplinkType,
    /// Set for `type: oidc`.
    pub(crate) issuer: Option<String>,
    /// Set for `type: microsoft` (Entra tenant).
    pub(crate) tenant: Option<String>,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) get_user_info: bool,
    pub(crate) source: UplinkSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UplinkSource {
    Host,
    Project,
    Merged,
}

impl UplinkSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Project => "project",
            Self::Merged => "merged",
        }
    }
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
        "usage: vzctl oidc status [--project P] [--format human|json]
       vzctl oidc clients [--project P] [--format human|json]
       vzctl oidc token [--client ID]"
    );
}

fn parse_format(
    mut args: impl Iterator<Item = String>,
) -> Result<(Format, Option<String>), String> {
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

pub(crate) fn host_uplink_path(state_dir: &Path) -> PathBuf {
    state_dir.join("config").join("oidc-uplink.yaml")
}

pub(crate) fn host_oidc_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("config").join("oidc")
}

pub(crate) fn host_secret_path(state_dir: &Path) -> PathBuf {
    host_oidc_dir(state_dir).join("client-secret")
}

/// Load host-wide uplink defaults (missing file → None).
pub(crate) fn load_host_uplink(state_dir: &Path) -> Result<Option<OidcUplink>, String> {
    let path = host_uplink_path(state_dir);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let host: HostOidcConfig =
        serde_yaml::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    Ok(host.uplink)
}

/// Merge host defaults with optional project override; resolve secret from files.
pub(crate) fn merge_uplink(
    state_dir: &Path,
    project: &str,
    host: Option<&OidcUplink>,
    project_uplink: Option<&OidcUplink>,
) -> Result<Option<ResolvedUplink>, String> {
    use crate::config::OidcUplinkType;

    if host.is_none() && project_uplink.is_none() {
        return Ok(None);
    }

    let source = match (host.is_some(), project_uplink.is_some()) {
        (true, true) => UplinkSource::Merged,
        (true, false) => UplinkSource::Host,
        (false, true) => UplinkSource::Project,
        (false, false) => unreachable!(),
    };

    let uplink_type = project_uplink
        .and_then(|u| u.uplink_type)
        .or_else(|| host.and_then(|u| u.uplink_type))
        .unwrap_or_default();

    let issuer = pick_str(
        project_uplink.and_then(|u| u.issuer.as_deref()),
        host.and_then(|u| u.issuer.as_deref()),
    );
    let tenant = pick_str(
        project_uplink.and_then(|u| u.tenant.as_deref()),
        host.and_then(|u| u.tenant.as_deref()),
    );
    let client_id = pick_str(
        project_uplink.and_then(|u| u.client_id.as_deref()),
        host.and_then(|u| u.client_id.as_deref()),
    );
    let default_scopes: Vec<String> = match uplink_type {
        OidcUplinkType::Oidc => DEFAULT_SCOPES.iter().map(|s| (*s).to_string()).collect(),
        OidcUplinkType::Github => vec!["read:user".into(), "user:email".into()],
        OidcUplinkType::Microsoft => {
            vec!["openid".into(), "profile".into(), "email".into()]
        }
        OidcUplinkType::Discord => vec!["identify".into(), "email".into()],
    };
    let scopes = project_uplink
        .and_then(|u| u.scopes.clone())
        .or_else(|| host.and_then(|u| u.scopes.clone()))
        .unwrap_or(default_scopes);
    let get_user_info = project_uplink
        .and_then(|u| u.get_user_info)
        .or_else(|| host.and_then(|u| u.get_user_info))
        .unwrap_or(true);

    let client_id = client_id.ok_or_else(|| {
        "oidc uplink incomplete: clientID required (set host defaults or oidc.uplink.clientID)"
            .to_string()
    })?;

    let issuer = match uplink_type {
        OidcUplinkType::Oidc => {
            let issuer = issuer.ok_or_else(|| {
                "oidc uplink incomplete: issuer required for type: oidc".to_string()
            })?;
            if !issuer.starts_with("https://") {
                return Err(format!(
                    "oidc uplink issuer must be https:// (got {issuer:?})"
                ));
            }
            Some(issuer)
        }
        _ => None,
    };

    let tenant = match uplink_type {
        OidcUplinkType::Microsoft => Some(tenant.unwrap_or_else(|| "common".into())),
        _ => tenant,
    };

    // Project secret wins when set and not "host"; otherwise use canonical host secret.
    let secret_path = resolve_secret_path(
        state_dir,
        project,
        project_uplink.and_then(|u| u.client_secret_file.as_deref()),
    )?;
    let client_secret = fs::read_to_string(&secret_path)
        .map_err(|e| format!("read uplink secret {}: {e}", secret_path.display()))?
        .trim()
        .to_string();
    if client_secret.is_empty() {
        return Err(format!(
            "oidc uplink secret file is empty: {}",
            secret_path.display()
        ));
    }

    Ok(Some(ResolvedUplink {
        uplink_type,
        issuer,
        tenant,
        client_id,
        client_secret,
        scopes,
        get_user_info,
        source,
    }))
}

fn pick_str(project: Option<&str>, host: Option<&str>) -> Option<String> {
    project
        .filter(|s| !s.is_empty())
        .or(host.filter(|s| !s.is_empty()))
        .map(|s| s.to_string())
}

fn resolve_secret_path(
    state_dir: &Path,
    project: &str,
    project_secret_ref: Option<&str>,
) -> Result<PathBuf, String> {
    match project_secret_ref {
        None | Some("") | Some("host") => {
            let path = host_secret_path(state_dir);
            if !path.exists() {
                return Err(format!(
                    "oidc uplink secret missing: {} (configure host client secret in Settings)",
                    path.display()
                ));
            }
            Ok(path)
        }
        Some(rel) if Path::new(rel).is_absolute() => {
            let path = PathBuf::from(rel);
            if !path.exists() {
                return Err(format!("oidc uplink secret missing: {}", path.display()));
            }
            Ok(path)
        }
        Some(rel) => {
            let path = project_oidc_dir(state_dir, project).join(rel);
            if !path.exists() {
                return Err(format!(
                    "oidc uplink secret missing: {} (or set clientSecretFile: host)",
                    path.display()
                ));
            }
            Ok(path)
        }
    }
}

fn status(format_project: (Format, Option<String>), state_dir: &Path) -> ExitCode {
    let (format, project) = format_project;
    // vz-edge owns IdP children under runtime/edge/oidc/{project}/.
    let (runtime, resolved_project) = resolve_oidc_runtime(state_dir, project.as_deref());
    let running = read_live_oidc_pid(&runtime);

    let host = match load_host_uplink(state_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_FAILED);
        }
    };

    let project_for_uplink = resolved_project.as_deref().or(project.as_deref());
    let uplink_summary = match project_for_uplink {
        Some(project_name) => {
            // Project YAML is not loaded here; report host + whether project secret exists.
            match merge_uplink(state_dir, project_name, host.as_ref(), None) {
                Ok(Some(resolved)) => json!({
                    "configured": true,
                    "type": uplink_type_str(resolved.uplink_type),
                    "issuer": resolved.issuer,
                    "tenant": resolved.tenant,
                    "clientID": resolved.client_id,
                    "source": resolved.source.as_str(),
                    "scopes": resolved.scopes,
                    "getUserInfo": resolved.get_user_info,
                }),
                Ok(None) => json!({ "configured": false }),
                Err(e) => json!({
                    "configured": host.is_some(),
                    "error": e,
                    "hostType": host.as_ref().map(|u| uplink_type_str(u.uplink_type.unwrap_or_default())),
                    "hostIssuer": host.as_ref().and_then(|u| u.issuer.clone()),
                    "hostClientID": host.as_ref().and_then(|u| u.client_id.clone()),
                }),
            }
        }
        None => match &host {
            Some(u) => json!({
                "configured": true,
                "type": uplink_type_str(u.uplink_type.unwrap_or_default()),
                "issuer": u.issuer,
                "tenant": u.tenant,
                "clientID": u.client_id,
                "source": "host",
                "scopes": u.scopes.clone().unwrap_or_else(|| {
                    DEFAULT_SCOPES.iter().map(|s| (*s).to_string()).collect()
                }),
                "getUserInfo": u.get_user_info.unwrap_or(true),
                "secretPresent": host_secret_path(state_dir).exists(),
            }),
            None => json!({
                "configured": false,
                "secretPresent": host_secret_path(state_dir).exists(),
            }),
        },
    };

    let data = json!({
        "running": running.is_some(),
        "pid": running,
        "project": resolved_project.or(project),
        "runtime": runtime,
        "uplink": uplink_summary,
    });
    emit(format, "oidc.status", data);
    ExitCode::SUCCESS
}

/// Resolve IdP runtime dir: edge-managed first, then legacy `runtime/oidc`.
fn resolve_oidc_runtime(state_dir: &Path, project: Option<&str>) -> (PathBuf, Option<String>) {
    let edge_root = state_dir.join("runtime").join("edge").join("oidc");
    let legacy_root = state_dir.join("runtime").join("oidc");

    if let Some(name) = project {
        let edge = edge_root.join(name);
        if edge.join("oidc.pid").exists() || edge.join("dex.pid").exists() || edge.is_dir() {
            return (edge, Some(name.to_string()));
        }
        let legacy = legacy_root.join(name);
        if legacy.is_dir() {
            return (legacy, Some(name.to_string()));
        }
        return (edge, Some(name.to_string()));
    }

    // No project: prefer any edge project with a live pid, else first edge dir, else legacy.
    if let Some(found) = find_oidc_project_dir(&edge_root, true) {
        return found;
    }
    if let Some(found) = find_oidc_project_dir(&edge_root, false) {
        return found;
    }
    if let Some(found) = find_oidc_project_dir(&legacy_root, true) {
        return found;
    }
    (legacy_root, None)
}

fn find_oidc_project_dir(root: &Path, require_live: bool) -> Option<(PathBuf, Option<String>)> {
    let mut entries = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if require_live && read_live_oidc_pid(&path).is_none() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        return Some((path, Some(name)));
    }
    None
}

fn read_live_oidc_pid(runtime: &Path) -> Option<i32> {
    for name in ["oidc.pid", "dex.pid"] {
        let pid_path = runtime.join(name);
        let Some(pid) = fs::read_to_string(&pid_path)
            .ok()
            .and_then(|raw| raw.trim().parse::<i32>().ok())
        else {
            continue;
        };
        if path_exists(&format!("/proc/{pid}")) {
            return Some(pid);
        }
        // macOS: check with kill -0
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .ok()
            .is_some_and(|s| s.success());
        if alive {
            return Some(pid);
        }
    }
    None
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

fn uplink_type_str(t: crate::config::OidcUplinkType) -> &'static str {
    use crate::config::OidcUplinkType;
    match t {
        OidcUplinkType::Oidc => "oidc",
        OidcUplinkType::Github => "github",
        OidcUplinkType::Microsoft => "microsoft",
        OidcUplinkType::Discord => "discord",
    }
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
pub(crate) fn write_clients(
    state_dir: &Path,
    project: &str,
    data: &Value,
) -> Result<PathBuf, String> {
    let dir = project_oidc_dir(state_dir, project);
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = clients_path(state_dir, project);
    let raw = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("write {}: {e}", path.display()))?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Ok(path)
}

/// Persist host uplink YAML (no secret inline) under Application Support.
#[cfg(test)]
pub(crate) fn write_host_uplink(state_dir: &Path, uplink: &OidcUplink) -> Result<PathBuf, String> {
    let path = host_uplink_path(state_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let mut to_store = uplink.clone();
    if to_store.uplink_type.is_none() {
        to_store.uplink_type = Some(crate::config::OidcUplinkType::Oidc);
    }
    // Always point secret at the canonical host secret file name when storing host defaults.
    if to_store
        .client_secret_file
        .as_deref()
        .is_none_or(|s| s.is_empty() || s == "host")
    {
        to_store.client_secret_file = Some("client-secret".into());
    }
    let cfg = HostOidcConfig {
        uplink: Some(to_store),
    };
    let raw = serde_yaml::to_string(&cfg).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("write {}: {e}", path.display()))?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Ok(path)
}

/// Write host client secret (0600).
#[cfg(test)]
pub(crate) fn write_host_secret(state_dir: &Path, secret: &str) -> Result<PathBuf, String> {
    let dir = host_oidc_dir(state_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = host_secret_path(state_dir);
    fs::write(&path, secret.trim()).map_err(|e| format!("write {}: {e}", path.display()))?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Ok(path)
}

/// Render JSON config for `vzctl-oidc-simple`.
pub(crate) fn render_simple_config(
    issuer: &str,
    listen: &str,
    clients: &Value,
    users: &[crate::config::OidcSimpleUser],
) -> Result<String, String> {
    let list = clients
        .get("clients")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let clients_out: Vec<Value> = list
        .into_iter()
        .map(|client| {
            json!({
                "id": client["id"].as_str().unwrap_or("client"),
                "secret": client["secret"].as_str().unwrap_or("secret"),
                "redirectURIs": client["redirectURIs"].clone(),
            })
        })
        .collect();
    let users_out: Vec<Value> = users
        .iter()
        .map(|u| {
            let mut obj = json!({
                "username": u.username,
                "email": u.email,
            });
            if !u.claims.is_empty() {
                obj["claims"] = Value::Object(u.claims.clone().into_iter().collect());
            }
            obj
        })
        .collect();
    let cfg = json!({
        "issuer": issuer,
        "listen": listen,
        "clients": clients_out,
        "users": users_out,
    });
    serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())
}

/// Render a Dex config YAML (static clients + optional OIDC connector uplink).
pub(crate) fn render_dex_config(
    issuer: &str,
    listen: &str,
    clients: &Value,
    password_file: Option<&Path>,
    storage_dir: &Path,
    uplink: Option<&ResolvedUplink>,
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

    let passwords = match (uplink.is_some(), password_file) {
        // Uplink without passwordFile → federation only
        (true, None) => "enablePasswordDB: false\n".to_string(),
        (true, Some(path)) if path.exists() => {
            let content =
                fs::read_to_string(path).map_err(|e| format!("read passwordFile: {e}"))?;
            format!("enablePasswordDB: true\nstaticPasswords:\n{content}")
        }
        (true, Some(path)) => {
            return Err(format!(
                "passwordFile not found: {} (omit passwordFile for uplink-only)",
                path.display()
            ));
        }
        // No uplink → existing behavior
        (false, Some(path)) if path.exists() => {
            let content =
                fs::read_to_string(path).map_err(|e| format!("read passwordFile: {e}"))?;
            format!("enablePasswordDB: true\nstaticPasswords:\n{content}")
        }
        (false, _) => {
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
        }
    };

    let connectors = if let Some(up) = uplink {
        render_connector_block(up, issuer)?
    } else {
        String::new()
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
{connectors}{passwords}"#,
        issuer = issuer,
        listen = listen,
        storage = storage_dir.display(),
        static_clients = static_clients,
        connectors = connectors,
        passwords = passwords,
    ))
}

fn render_connector_block(up: &ResolvedUplink, dex_issuer: &str) -> Result<String, String> {
    use crate::config::OidcUplinkType;

    let redirect = format!("{}/callback", dex_issuer.trim_end_matches('/'));
    let mut block = String::from("connectors:\n");
    match up.uplink_type {
        OidcUplinkType::Oidc => {
            let issuer = up
                .issuer
                .as_deref()
                .ok_or_else(|| "oidc connector missing issuer".to_string())?;
            block.push_str("  - type: oidc\n");
            block.push_str("    id: uplink\n");
            block.push_str("    name: Upstream IdP\n");
            block.push_str("    config:\n");
            block.push_str(&format!("      issuer: {}\n", yaml_string(issuer)));
            block.push_str(&format!("      clientID: {}\n", yaml_string(&up.client_id)));
            block.push_str(&format!(
                "      clientSecret: {}\n",
                yaml_string(&up.client_secret)
            ));
            block.push_str(&format!("      redirectURI: {}\n", yaml_string(&redirect)));
            block.push_str("      scopes:\n");
            for scope in &up.scopes {
                block.push_str(&format!("      - {}\n", yaml_string(scope)));
            }
            block.push_str(&format!("      getUserInfo: {}\n", up.get_user_info));
        }
        OidcUplinkType::Github => {
            block.push_str("  - type: github\n");
            block.push_str("    id: uplink\n");
            block.push_str("    name: GitHub\n");
            block.push_str("    config:\n");
            block.push_str(&format!("      clientID: {}\n", yaml_string(&up.client_id)));
            block.push_str(&format!(
                "      clientSecret: {}\n",
                yaml_string(&up.client_secret)
            ));
            block.push_str(&format!("      redirectURI: {}\n", yaml_string(&redirect)));
            if !up.scopes.is_empty() {
                block.push_str("      scopes:\n");
                for scope in &up.scopes {
                    block.push_str(&format!("      - {}\n", yaml_string(scope)));
                }
            }
        }
        OidcUplinkType::Microsoft => {
            let tenant = up.tenant.as_deref().unwrap_or("common");
            block.push_str("  - type: microsoft\n");
            block.push_str("    id: uplink\n");
            block.push_str("    name: Microsoft Entra ID\n");
            block.push_str("    config:\n");
            block.push_str(&format!("      clientID: {}\n", yaml_string(&up.client_id)));
            block.push_str(&format!(
                "      clientSecret: {}\n",
                yaml_string(&up.client_secret)
            ));
            block.push_str(&format!("      redirectURI: {}\n", yaml_string(&redirect)));
            block.push_str(&format!("      tenant: {}\n", yaml_string(tenant)));
            if !up.scopes.is_empty() {
                block.push_str("      scopes:\n");
                for scope in &up.scopes {
                    block.push_str(&format!("      - {}\n", yaml_string(scope)));
                }
            }
        }
        OidcUplinkType::Discord => {
            // Discord has no native Dex connector — use generic oauth2.
            block.push_str("  - type: oauth2\n");
            block.push_str("    id: uplink\n");
            block.push_str("    name: Discord\n");
            block.push_str("    config:\n");
            block.push_str(&format!("      clientID: {}\n", yaml_string(&up.client_id)));
            block.push_str(&format!(
                "      clientSecret: {}\n",
                yaml_string(&up.client_secret)
            ));
            block.push_str(&format!("      redirectURI: {}\n", yaml_string(&redirect)));
            block.push_str("      authorizationURL: https://discord.com/api/oauth2/authorize\n");
            block.push_str("      tokenURL: https://discord.com/api/oauth2/token\n");
            block.push_str("      userInfoURL: https://discord.com/api/users/@me\n");
            block.push_str("      scopes:\n");
            for scope in &up.scopes {
                block.push_str(&format!("      - {}\n", yaml_string(scope)));
            }
            block.push_str("      userIDKey: id\n");
            block.push_str("      claimMapping:\n");
            block.push_str("        userNameKey: username\n");
            block.push_str("        emailKey: email\n");
        }
    }
    Ok(block)
}

fn yaml_string(value: &str) -> String {
    // Quote when needed so secrets/URLs with special chars stay valid YAML.
    if value.is_empty()
        || value.contains(':')
        || value.contains('#')
        || value.contains(' ')
        || value.contains('"')
        || value.contains('\'')
        || value.contains('\n')
        || value.starts_with('{')
        || value.starts_with('[')
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OidcUplink, OidcUplinkType};

    fn test_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("vzctl-oidc-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_clients() -> Value {
        json!({
            "clients": [{
                "id": "web",
                "secret": "sec",
                "name": "web",
                "redirectURIs": ["https://web.svc.example.vz.test/oauth2/callback"]
            }]
        })
    }

    #[test]
    fn merge_host_only_resolves_secret() {
        let state = test_dir("merge-host");
        write_host_secret(&state, "host-secret").unwrap();
        let host = OidcUplink {
            uplink_type: Some(OidcUplinkType::Oidc),
            issuer: Some("https://login.example.com".into()),
            tenant: None,
            client_id: Some("vzctl-dex".into()),
            client_secret_file: Some("client-secret".into()),
            scopes: None,
            get_user_info: Some(true),
        };
        let resolved = merge_uplink(&state, "proj", Some(&host), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            resolved.issuer.as_deref(),
            Some("https://login.example.com")
        );
        assert_eq!(resolved.client_id, "vzctl-dex");
        assert_eq!(resolved.client_secret, "host-secret");
        assert_eq!(resolved.source, UplinkSource::Host);
        assert_eq!(resolved.scopes, vec!["openid", "profile", "email"]);
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn merge_project_overrides_host_fields() {
        let state = test_dir("merge-override");
        write_host_secret(&state, "host-secret").unwrap();
        let host = OidcUplink {
            uplink_type: Some(OidcUplinkType::Oidc),
            issuer: Some("https://login.example.com".into()),
            tenant: None,
            client_id: Some("vzctl-dex".into()),
            client_secret_file: Some("client-secret".into()),
            scopes: Some(vec!["openid".into()]),
            get_user_info: Some(false),
        };
        let project = OidcUplink {
            uplink_type: Some(OidcUplinkType::Oidc),
            issuer: Some("https://login.corp.example".into()),
            tenant: None,
            client_id: Some("edge-dmz-dex".into()),
            client_secret_file: Some("host".into()),
            scopes: None,
            get_user_info: None,
        };
        let resolved = merge_uplink(&state, "edge-dmz", Some(&host), Some(&project))
            .unwrap()
            .unwrap();
        assert_eq!(
            resolved.issuer.as_deref(),
            Some("https://login.corp.example")
        );
        assert_eq!(resolved.client_id, "edge-dmz-dex");
        assert_eq!(resolved.client_secret, "host-secret");
        assert_eq!(resolved.source, UplinkSource::Merged);
        assert_eq!(resolved.scopes, vec!["openid"]);
        assert!(!resolved.get_user_info);
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn merge_project_secret_file() {
        let state = test_dir("merge-proj-secret");
        let proj_dir = project_oidc_dir(&state, "edge-dmz");
        fs::create_dir_all(&proj_dir).unwrap();
        fs::write(proj_dir.join("uplink-secret"), "project-secret").unwrap();
        let project = OidcUplink {
            uplink_type: Some(OidcUplinkType::Oidc),
            issuer: Some("https://login.corp.example".into()),
            tenant: None,
            client_id: Some("edge".into()),
            client_secret_file: Some("uplink-secret".into()),
            scopes: None,
            get_user_info: None,
        };
        let resolved = merge_uplink(&state, "edge-dmz", None, Some(&project))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.client_secret, "project-secret");
        assert_eq!(resolved.source, UplinkSource::Project);
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn merge_incomplete_errors() {
        let state = test_dir("merge-incomplete");
        let host = OidcUplink {
            uplink_type: Some(OidcUplinkType::Oidc),
            issuer: Some("https://login.example.com".into()),
            tenant: None,
            client_id: None,
            client_secret_file: None,
            scopes: None,
            get_user_info: None,
        };
        let err = merge_uplink(&state, "p", Some(&host), None).unwrap_err();
        assert!(err.contains("clientID"));
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn render_without_uplink_keeps_password_db() {
        let state = test_dir("render-no-uplink");
        let yaml = render_dex_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            None,
            &state,
            None,
        )
        .unwrap();
        assert!(yaml.contains("enablePasswordDB: true"));
        assert!(yaml.contains("admin@vzctl.local"));
        assert!(!yaml.contains("connectors:"));
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn render_with_uplink_disables_password_db() {
        let state = test_dir("render-uplink");
        let uplink = ResolvedUplink {
            uplink_type: OidcUplinkType::Oidc,
            issuer: Some("https://login.example.com".into()),
            tenant: None,
            client_id: "vzctl-dex".into(),
            client_secret: "s3cret".into(),
            scopes: vec!["openid".into(), "email".into()],
            get_user_info: true,
            source: UplinkSource::Host,
        };
        let yaml = render_dex_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            None,
            &state,
            Some(&uplink),
        )
        .unwrap();
        assert!(yaml.contains("enablePasswordDB: false"));
        assert!(yaml.contains("connectors:"));
        assert!(yaml.contains("type: oidc"));
        assert!(yaml.contains("id: uplink"));
        assert!(yaml.contains("https://login.example.com"));
        assert!(yaml.contains("clientSecret: s3cret"));
        assert!(yaml.contains("https://auth.svc.p.vz.test/callback"));
        assert!(!yaml.contains("admin@vzctl.local"));
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn render_github_connector() {
        let state = test_dir("render-github");
        let uplink = ResolvedUplink {
            uplink_type: OidcUplinkType::Github,
            issuer: None,
            tenant: None,
            client_id: "gh-app".into(),
            client_secret: "gh-sec".into(),
            scopes: vec!["read:user".into(), "user:email".into()],
            get_user_info: true,
            source: UplinkSource::Host,
        };
        let yaml = render_dex_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            None,
            &state,
            Some(&uplink),
        )
        .unwrap();
        assert!(yaml.contains("type: github"));
        assert!(yaml.contains("name: GitHub"));
        assert!(yaml.contains("clientID: gh-app"));
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn render_microsoft_and_discord_connectors() {
        let state = test_dir("render-ms-discord");
        let ms = ResolvedUplink {
            uplink_type: OidcUplinkType::Microsoft,
            issuer: None,
            tenant: Some("common".into()),
            client_id: "ms-app".into(),
            client_secret: "ms-sec".into(),
            scopes: vec!["openid".into()],
            get_user_info: true,
            source: UplinkSource::Host,
        };
        let yaml = render_dex_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            None,
            &state,
            Some(&ms),
        )
        .unwrap();
        assert!(yaml.contains("type: microsoft"));
        assert!(yaml.contains("tenant: common"));
        assert!(yaml.contains("Microsoft Entra ID"));

        let discord = ResolvedUplink {
            uplink_type: OidcUplinkType::Discord,
            issuer: None,
            tenant: None,
            client_id: "dc-app".into(),
            client_secret: "dc-sec".into(),
            scopes: vec!["identify".into(), "email".into()],
            get_user_info: true,
            source: UplinkSource::Host,
        };
        let yaml = render_dex_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            None,
            &state,
            Some(&discord),
        )
        .unwrap();
        assert!(yaml.contains("type: oauth2"));
        assert!(yaml.contains("name: Discord"));
        assert!(yaml.contains("discord.com/api/oauth2/authorize"));
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn render_uplink_with_password_file_keeps_both() {
        let state = test_dir("render-both");
        let pw = state.join("passwords");
        fs::write(
            &pw,
            r#"  - email: "break@vzctl.local"
    hash: "$2a$10$x"
    username: "break"
    userID: "1"
"#,
        )
        .unwrap();
        let uplink = ResolvedUplink {
            uplink_type: OidcUplinkType::Oidc,
            issuer: Some("https://login.example.com".into()),
            tenant: None,
            client_id: "vzctl-dex".into(),
            client_secret: "s3cret".into(),
            scopes: vec!["openid".into()],
            get_user_info: false,
            source: UplinkSource::Merged,
        };
        let yaml = render_dex_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            Some(&pw),
            &state,
            Some(&uplink),
        )
        .unwrap();
        assert!(yaml.contains("enablePasswordDB: true"));
        assert!(yaml.contains("break@vzctl.local"));
        assert!(yaml.contains("connectors:"));
        assert!(yaml.contains("getUserInfo: false"));
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn render_simple_config_includes_users_and_clients() {
        use crate::config::OidcSimpleUser;
        use std::collections::BTreeMap;
        let users = vec![OidcSimpleUser {
            username: "alice".into(),
            email: "alice@dev.local".into(),
            claims: BTreeMap::from([("role".into(), json!("admin"))]),
        }];
        let raw = render_simple_config(
            "https://auth.svc.p.vz.test",
            "127.0.0.1:5556",
            &sample_clients(),
            &users,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["issuer"], "https://auth.svc.p.vz.test");
        assert_eq!(parsed["users"][0]["username"], "alice");
        assert_eq!(parsed["users"][0]["claims"]["role"], "admin");
        assert_eq!(parsed["clients"][0]["id"], "web");
    }

    #[test]
    fn write_host_uplink_roundtrip() {
        let state = test_dir("host-write");
        write_host_secret(&state, "sec").unwrap();
        let uplink = OidcUplink {
            uplink_type: Some(OidcUplinkType::Oidc),
            issuer: Some("https://login.example.com".into()),
            tenant: None,
            client_id: Some("vzctl-dex".into()),
            client_secret_file: None,
            scopes: None,
            get_user_info: None,
        };
        write_host_uplink(&state, &uplink).unwrap();
        let loaded = load_host_uplink(&state).unwrap().unwrap();
        assert_eq!(loaded.issuer.as_deref(), Some("https://login.example.com"));
        assert_eq!(loaded.client_id.as_deref(), Some("vzctl-dex"));
        assert_eq!(loaded.client_secret_file.as_deref(), Some("client-secret"));
        let _ = fs::remove_dir_all(&state);
    }
}
