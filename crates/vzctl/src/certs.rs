//! Local CA under Application Support/vzctl/ca/ (v0.2 / #45).

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};
use time::{Duration, OffsetDateTime};

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_FAILED: u8 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

pub(crate) fn command(args: impl Iterator<Item = String>, state_dir: &Path) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let mut iter = args.into_iter().peekable();
    let Some(subcommand) = iter.next() else {
        usage();
        return ExitCode::from(EXIT_USAGE);
    };
    match subcommand.as_str() {
        "ca" => ca_command(iter, state_dir),
        "mint" => match parse_mint(iter) {
            Ok((format, san, extras)) => match mint_leaf(state_dir, &san, &extras) {
                Ok(info) => {
                    emit_ok(format, "certs.mint", info);
                    ExitCode::SUCCESS
                }
                Err(message) => fail(format, EXIT_FAILED, message),
            },
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        "fingerprint" => match parse_format_only(iter) {
            Ok(format) => match read_fingerprint(state_dir) {
                Ok(fp) => {
                    emit_ok(
                        format,
                        "certs.fingerprint",
                        json!({ "fingerprint": fp, "path": fingerprint_path(state_dir) }),
                    );
                    ExitCode::SUCCESS
                }
                Err(message) => fail(format, EXIT_INVALID, message),
            },
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        "rollout" => {
            eprintln!(
                "certs rollout is applied via `vzctl apply` (ensure_ca_rollout); \
                 use agent ca_inject for live VMs"
            );
            ExitCode::from(EXIT_USAGE)
        }
        "verify" => {
            eprintln!("usage: vzctl certs verify --vm NAME --url URL (via guest agent exec)");
            ExitCode::from(EXIT_USAGE)
        }
        "-h" | "--help" | "help" => {
            usage();
            ExitCode::from(EXIT_USAGE)
        }
        other => {
            eprintln!("unknown certs subcommand: {other}");
            usage();
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn usage() {
    eprintln!(
        "usage: vzctl certs ca init [--force] [--format human|json]
       vzctl certs ca install [--format human|json]
       vzctl certs mint <san> [--san alias...] [--format human|json]
       vzctl certs fingerprint [--format human|json]
       vzctl certs rollout [--vm NAME] [--format human|json]
       vzctl certs verify --vm NAME --url URL"
    );
}

fn ca_command(mut args: impl Iterator<Item = String>, state_dir: &Path) -> ExitCode {
    let Some(subcommand) = args.next() else {
        usage();
        return ExitCode::from(EXIT_USAGE);
    };
    match subcommand.as_str() {
        "init" => match parse_init(args) {
            Ok((format, force)) => match ensure_ca(state_dir, force) {
                Ok(info) => {
                    emit_ok(format, "certs.ca.init", info);
                    ExitCode::SUCCESS
                }
                Err(message) => fail(format, EXIT_FAILED, message),
            },
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        "install" => match parse_format_only(args) {
            Ok(format) => match install_keychain(state_dir) {
                Ok(info) => {
                    emit_ok(format, "certs.ca.install", info);
                    ExitCode::SUCCESS
                }
                Err(message) => fail(format, EXIT_FAILED, message),
            },
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_USAGE)
            }
        },
        other => {
            eprintln!("unknown certs ca subcommand: {other}");
            usage();
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn parse_format_only(mut args: impl Iterator<Item = String>) -> Result<Format, String> {
    let mut format = Format::Human;
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
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(format)
}

fn parse_init(mut args: impl Iterator<Item = String>) -> Result<(Format, bool), String> {
    let mut format = Format::Human;
    let mut force = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--force" => force = true,
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => return Err(format!("unsupported format: {value}")),
                    None => return Err("--format requires human or json".into()),
                };
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok((format, force))
}

fn parse_mint(
    mut args: impl Iterator<Item = String>,
) -> Result<(Format, String, Vec<String>), String> {
    let mut format = Format::Human;
    let mut san = None;
    let mut extras = Vec::new();
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
            "--san" => {
                let value = args
                    .next()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| "--san requires a value".to_string())?;
                extras.push(value);
            }
            value if !value.starts_with('-') && san.is_none() => san = Some(arg),
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    let san = san.ok_or_else(|| "mint requires a primary SAN/hostname".to_string())?;
    Ok((format, san, extras))
}

pub(crate) fn ca_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("ca")
}

pub(crate) fn root_dir(state_dir: &Path) -> PathBuf {
    ca_dir(state_dir).join("root")
}

pub(crate) fn issued_dir(state_dir: &Path) -> PathBuf {
    ca_dir(state_dir).join("issued")
}

pub(crate) fn trust_dir(state_dir: &Path) -> PathBuf {
    ca_dir(state_dir).join("trust")
}

pub(crate) fn ca_cert_path(state_dir: &Path) -> PathBuf {
    root_dir(state_dir).join("ca.crt")
}

pub(crate) fn ca_key_path(state_dir: &Path) -> PathBuf {
    root_dir(state_dir).join("ca.key")
}

pub(crate) fn fingerprint_path(state_dir: &Path) -> PathBuf {
    root_dir(state_dir).join("fingerprint")
}

pub(crate) fn trust_cert_path(state_dir: &Path) -> PathBuf {
    trust_dir(state_dir).join("vzctl-local.crt")
}

pub(crate) fn leaf_paths(state_dir: &Path, san: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = issued_dir(state_dir).join(san);
    (
        dir.join("cert.pem"),
        dir.join("key.pem"),
        dir.join("meta.json"),
    )
}

fn ensure_dirs(state_dir: &Path) -> Result<(), String> {
    for dir in [
        ca_dir(state_dir),
        root_dir(state_dir),
        issued_dir(state_dir),
        trust_dir(state_dir),
    ] {
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    Ok(())
}

fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    Ok(())
}

fn write_public(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

pub(crate) fn fingerprint_pem(pem: &str) -> String {
    let der = pem::parse(pem.as_bytes())
        .map(|p| p.contents().to_vec())
        .unwrap_or_else(|_| pem.as_bytes().to_vec());
    let digest = Sha256::digest(der);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn read_fingerprint(state_dir: &Path) -> Result<String, String> {
    let path = fingerprint_path(state_dir);
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("CA not initialized ({}); run vzctl certs ca init", e))?;
    Ok(raw.trim().to_string())
}

pub(crate) fn read_ca_pem(state_dir: &Path) -> Result<String, String> {
    fs::read_to_string(ca_cert_path(state_dir))
        .map_err(|e| format!("read CA cert: {e}"))
}

/// Ensure CA exists; create if missing or `force`.
pub(crate) fn ensure_ca(state_dir: &Path, force: bool) -> Result<Value, String> {
    ensure_dirs(state_dir)?;
    let cert_path = ca_cert_path(state_dir);
    let key_path = ca_key_path(state_dir);
    if cert_path.exists() && key_path.exists() && !force {
        let fp = read_fingerprint(state_dir)?;
        return Ok(json!({
            "created": false,
            "fingerprint": fp,
            "cert": cert_path,
            "trust": trust_cert_path(state_dir),
        }));
    }

    let mut params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| format!("CA params: {e}"))?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
    params.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "vzctl Local CA");
    dn.push(DnType::OrganizationName, "vzctl");
    params.distinguished_name = dn;

    let key_pair = KeyPair::generate().map_err(|e| format!("CA keygen: {e}"))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("CA sign: {e}"))?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let fp = fingerprint_pem(&cert_pem);

    write_public(&cert_path, cert_pem.as_bytes())?;
    write_secret(&key_path, key_pem.as_bytes())?;
    write_public(fingerprint_path(state_dir).as_path(), fp.as_bytes())?;
    write_public(trust_cert_path(state_dir).as_path(), cert_pem.as_bytes())?;

    Ok(json!({
        "created": true,
        "fingerprint": fp,
        "cert": cert_path,
        "trust": trust_cert_path(state_dir),
    }))
}

pub(crate) fn mint_leaf(
    state_dir: &Path,
    primary: &str,
    extra_sans: &[String],
) -> Result<Value, String> {
    ensure_ca(state_dir, false)?;
    let ca_pem = read_ca_pem(state_dir)?;
    let ca_key_pem = fs::read_to_string(ca_key_path(state_dir))
        .map_err(|e| format!("read CA key: {e}"))?;
    let ca_key = KeyPair::from_pem(&ca_key_pem).map_err(|e| format!("parse CA key: {e}"))?;
    let ca_params =
        CertificateParams::from_ca_cert_pem(&ca_pem).map_err(|e| format!("parse CA cert: {e}"))?;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| format!("rebuild CA cert: {e}"))?;

    let mut names = vec![primary.to_string()];
    for san in extra_sans {
        if !names.iter().any(|n| n == san) {
            names.push(san.clone());
        }
    }

    let mut params = CertificateParams::new(names.clone())
        .map_err(|e| format!("leaf params: {e}"))?;
    params.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
    params.not_after = OffsetDateTime::now_utc() + Duration::days(825);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, primary);
    params.distinguished_name = dn;

    let key_pair = KeyPair::generate().map_err(|e| format!("leaf keygen: {e}"))?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key)
        .map_err(|e| format!("leaf sign: {e}"))?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let fp = fingerprint_pem(&cert_pem);
    let (cert_path, key_path, meta_path) = leaf_paths(state_dir, primary);
    write_public(&cert_path, cert_pem.as_bytes())?;
    write_secret(&key_path, key_pem.as_bytes())?;
    let meta = json!({
        "san": primary,
        "sans": names,
        "fingerprint": fp,
        "not_after_unix": (OffsetDateTime::now_utc() + Duration::days(825)).unix_timestamp(),
        "issued_unix": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    write_public(
        &meta_path,
        serde_json::to_string_pretty(&meta)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;

    Ok(json!({
        "san": primary,
        "sans": names,
        "fingerprint": fp,
        "cert": cert_path,
        "key": key_path,
    }))
}

fn install_keychain(state_dir: &Path) -> Result<Value, String> {
    let cert = ca_cert_path(state_dir);
    if !cert.exists() {
        return Err("CA not initialized; run vzctl certs ca init".into());
    }
    let status = Command::new("security")
        .args([
            "add-trusted-cert",
            "-d",
            "-r",
            "trustRoot",
            "-k",
            &format!(
                "{}/Library/Keychains/login.keychain-db",
                std::env::var("HOME").unwrap_or_else(|_| "/".into())
            ),
            cert.to_str().unwrap_or(""),
        ])
        .status()
        .map_err(|e| format!("security add-trusted-cert: {e}"))?;
    if !status.success() {
        return Err(format!(
            "security add-trusted-cert failed with status {status}"
        ));
    }
    Ok(json!({
        "installed": true,
        "cert": cert,
        "fingerprint": read_fingerprint(state_dir)?,
    }))
}

/// NoCloud write_files entries that seed the CA into a guest at first boot.
pub(crate) fn nocloud_ca_write_files(state_dir: &Path) -> Result<Vec<Value>, String> {
    let pem = read_ca_pem(state_dir)?;
    Ok(vec![
        json!({
            "path": "/usr/local/share/ca-certificates/vzctl-local.crt",
            "permissions": "0644",
            "content": pem,
        }),
        json!({
            "path": "/var/lib/vzctl/ca.fingerprint",
            "permissions": "0644",
            "content": read_fingerprint(state_dir)?,
        }),
    ])
}

fn emit_ok(format: Format, command: &str, data: Value) {
    match format {
        Format::Json => {
            let envelope = json!({
                "apiVersion": API_VERSION,
                "kind": "Result",
                "ok": true,
                "command": command,
                "data": data,
            });
            println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
        }
        Format::Human => {
            if let Some(obj) = data.as_object() {
                for (key, value) in obj {
                    println!("{key}: {}", value_to_display(value));
                }
            } else {
                println!("{}", value_to_display(&data));
            }
        }
    }
}

fn fail(format: Format, code: u8, message: String) -> ExitCode {
    match format {
        Format::Json => {
            let envelope = json!({
                "apiVersion": API_VERSION,
                "kind": "Error",
                "ok": false,
                "error": { "message": message, "code": code },
            });
            eprintln!("{}", serde_json::to_string_pretty(&envelope).unwrap());
        }
        Format::Human => eprintln!("{message}"),
    }
    ExitCode::from(code)
}

fn value_to_display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn init_and_mint_roundtrip() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vzctl-ca-test-{stamp}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let info = ensure_ca(&dir, false).unwrap();
        assert_eq!(info["created"], true);
        let fp = read_fingerprint(&dir).unwrap();
        assert_eq!(info["fingerprint"], fp);
        let again = ensure_ca(&dir, false).unwrap();
        assert_eq!(again["created"], false);
        let leaf = mint_leaf(
            &dir,
            "auth.svc.edge-dmz.vz.test",
            &["auth.localhost".into()],
        )
        .unwrap();
        assert!(leaf["cert"].as_str().unwrap().contains("auth.svc"));
        assert!(PathBuf::from(leaf["cert"].as_str().unwrap()).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
