//! Local CA under Application Support/vzctl/ca/ (v0.2 / #45).

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};

mod profile;
mod upgrade;
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
    let requested = requested_format(&args);
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
                Err(message) => fail(format, "certs.mint", EXIT_FAILED, message),
            },
            Err(message) => fail(requested, "certs.mint", EXIT_USAGE, message),
        },
        "fingerprint" => match parse_format_only(iter) {
            Ok(format) => match read_fingerprint(state_dir) {
                Ok(fp) => {
                    let mut info = host_trust_status(state_dir);
                    if let Some(obj) = info.as_object_mut() {
                        obj.insert("fingerprint".into(), json!(fp));
                        obj.insert("path".into(), json!(fingerprint_path(state_dir)));
                    }
                    emit_ok(format, "certs.fingerprint", info);
                    ExitCode::SUCCESS
                }
                Err(message) => fail(format, "certs.fingerprint", EXIT_INVALID, message),
            },
            Err(message) => fail(requested, "certs.fingerprint", EXIT_USAGE, message),
        },
        "rollout" => {
            fail(requested, "certs.rollout", EXIT_USAGE,
                "certs rollout is applied via `vzctl apply` (ensure_ca_rollout); use agent ca_inject for live VMs".into())
        }
        "verify" => {
            fail(requested, "certs.verify", EXIT_USAGE,
                "certs verify is not implemented; use vm exec with curl --fail against the target URL".into())
        }
        "-h" | "--help" | "help" => {
            usage();
            ExitCode::from(EXIT_USAGE)
        }
        other => {
            fail(requested, "certs", EXIT_USAGE, format!("unknown certs subcommand: {other}"))
        }
    }
}

fn usage() {
    eprintln!(
        "usage: vzctl certs ca init [--force] [--format human|json]
       vzctl certs ca upgrade --output <new-private-directory> [--format human|json]
       vzctl certs ca install [--format human|json]
       vzctl certs mint <san> [--san alias...] [--format human|json]
       vzctl certs fingerprint [--format human|json]
       vzctl certs rollout [--vm NAME] [--format human|json]
       vzctl certs verify --vm NAME --url URL"
    );
}

fn ca_command(args: impl Iterator<Item = String>, state_dir: &Path) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested = requested_format(&args);
    let mut args = args.into_iter();
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
                Err(message) => fail(format, "certs.ca.init", EXIT_FAILED, message),
            },
            Err(message) => fail(requested, "certs.ca.init", EXIT_USAGE, message),
        },
        "upgrade" => {
            let mut output = None;
            let mut remaining = Vec::new();
            let mut invalid = None;
            while let Some(arg) = args.next() {
                if arg == "--output" {
                    if output.is_some() {
                        invalid = Some("--output repeated".to_string());
                        break;
                    }
                    output = args.next().map(PathBuf::from);
                    if output.is_none() {
                        invalid = Some("--output requires a directory".to_string());
                        break;
                    }
                } else {
                    remaining.push(arg);
                }
            }
            let parsed = parse_format_only(remaining.into_iter()).and_then(|format| {
                if let Some(message) = invalid {
                    return Err(message);
                }
                output.map(|output| (format, output)).ok_or_else(|| {
                    "--output is required; upgrade only stages a private candidate".into()
                })
            });
            match parsed {
                Ok((format, output)) => match upgrade::stage(state_dir, &output) {
                    Ok(info) => {
                        emit_ok(format, "certs.ca.upgrade", info);
                        ExitCode::SUCCESS
                    }
                    Err(message) => fail(format, "certs.ca.upgrade", EXIT_FAILED, message),
                },
                Err(message) => fail(requested, "certs.ca.upgrade", EXIT_USAGE, message),
            }
        }
        "install" => match parse_format_only(args) {
            Ok(format) => match install_keychain(state_dir) {
                Ok(info) => {
                    emit_ok(format, "certs.ca.install", info);
                    ExitCode::SUCCESS
                }
                Err(message) => fail(format, "certs.ca.install", EXIT_FAILED, message),
            },
            Err(message) => fail(requested, "certs.ca.install", EXIT_USAGE, message),
        },
        other => fail(
            requested,
            "certs.ca",
            EXIT_USAGE,
            format!("unknown certs ca subcommand: {other}"),
        ),
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

/// Refuse half pairs and symlink targets; never silently replace an unknown key.
fn read_pair(cert: &Path, key: &Path) -> Result<Option<(String, String)>, String> {
    let mut present = Vec::new();
    for path in [cert, key] {
        match fs::symlink_metadata(path) {
            Ok(meta)
                if meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.len() <= 1024 * 1024 =>
            {
                present.push(true)
            }
            Ok(_) => {
                return Err("certificate/key path must be a regular file, not a symlink".into())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => present.push(false),
            Err(error) => return Err(format!("inspect certificate/key: {error}")),
        }
    }
    if present == [false, false] {
        return Ok(None);
    }
    if present != [true, true] {
        return Err("incomplete certificate/key pair; refusing automatic recreation".into());
    }
    if fs::metadata(key)
        .map_err(|e| e.to_string())?
        .permissions()
        .mode()
        & 0o077
        != 0
    {
        return Err("private certificate key must not be group/world accessible".into());
    }
    Ok(Some((
        fs::read_to_string(cert).map_err(|_| "read certificate failed")?,
        fs::read_to_string(key).map_err(|_| "read private key failed")?,
    )))
}

fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_write(path, bytes, 0o600)
}

fn write_public(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_write(path, bytes, 0o644)
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("certificate destination has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("refusing non-regular certificate destination".into());
        }
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    static NEXT_WRITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT_WRITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".vzctl-cert-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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
    fs::read_to_string(ca_cert_path(state_dir)).map_err(|e| format!("read CA cert: {e}"))
}

/// Ensure CA exists; create if missing or `force`.
pub(crate) fn ensure_ca(state_dir: &Path, force: bool) -> Result<Value, String> {
    ensure_dirs(state_dir)?;
    let cert_path = ca_cert_path(state_dir);
    let key_path = ca_key_path(state_dir);
    let existing = read_pair(&cert_path, &key_path)?;
    if let Some((cert, key)) = existing.as_ref().filter(|_| !force) {
        profile::ca_pair(cert, key, true)?;
        let fp = read_fingerprint(state_dir)?;
        if fp != fingerprint_pem(cert)
            || fs::read_to_string(trust_cert_path(state_dir))
                .ok()
                .as_deref()
                != Some(cert.as_str())
        {
            return Err("CA fingerprint/trust copy is inconsistent; inspect state and complete an explicit reviewed rollout".into());
        }
        return Ok(json!({
            "created": false,
            "fingerprint": fp,
            "cert": cert_path,
            "trust": trust_cert_path(state_dir),
        }));
    }

    let mut params =
        CertificateParams::new(Vec::<String>::new()).map_err(|e| format!("CA params: {e}"))?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.use_authority_key_identifier_extension = true;
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

    profile::ca_pair(&cert_pem, &key_pem, true)?;
    write_secret(&key_path, key_pem.as_bytes())?;
    write_public(&cert_path, cert_pem.as_bytes())?;
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
    let ca_key_pem =
        fs::read_to_string(ca_key_path(state_dir)).map_err(|e| format!("read CA key: {e}"))?;
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

    let mut params =
        CertificateParams::new(names.clone()).map_err(|e| format!("leaf params: {e}"))?;
    params.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
    params.not_after = OffsetDateTime::now_utc() + Duration::days(825);
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, primary);
    params.distinguished_name = dn;

    let (cert_path, key_path, meta_path) = leaf_paths(state_dir, primary);
    let existing = read_pair(&cert_path, &key_path)?;
    let key_pair = if let Some((cert, key)) = &existing {
        let current = profile::leaf_pair(cert, key, &ca_pem, false)?;
        let metadata: Option<Value> = fs::read(&meta_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        if current.subject_alt_names == params.subject_alt_names
            && current.not_after > OffsetDateTime::now_utc() + Duration::days(30)
            && profile::leaf_pair(cert, key, &ca_pem, true).is_ok()
            && metadata
                .as_ref()
                .is_some_and(|value| value["fingerprint"] == fingerprint_pem(cert))
        {
            return Ok(
                json!({"san": primary, "sans": names, "fingerprint": fingerprint_pem(cert),
                "cert": cert_path, "key": key_path, "reused": true}),
            );
        }
        KeyPair::from_pem(key).map_err(|_| "invalid existing leaf private key")?
    } else {
        KeyPair::generate().map_err(|e| format!("leaf keygen: {e}"))?
    };
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key)
        .map_err(|e| format!("leaf sign: {e}"))?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let fp = fingerprint_pem(&cert_pem);
    profile::leaf_pair(&cert_pem, &key_pem, &ca_pem, true)?;
    if existing.is_none() {
        write_secret(&key_path, key_pem.as_bytes())?;
    }
    write_public(&cert_path, cert_pem.as_bytes())?;
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

/// Host Keychain trust for the Local CA (Safari/Chrome/curl). Firefox/Zen need
/// enterprise roots or a manual import — not covered here.
pub(crate) fn host_trust_status(state_dir: &Path) -> Value {
    let cert = ca_cert_path(state_dir);
    if !cert.exists() {
        return json!({
            "present": false,
            "trusted": false,
            "fingerprint": null,
            "cert": cert,
            "fix": "vzctl certs ca init",
        });
    }
    let fingerprint = read_fingerprint(state_dir).ok();
    let trusted = mac_keychain_trusts(&cert);
    json!({
        "present": true,
        "trusted": trusted,
        "fingerprint": fingerprint,
        "cert": cert,
        "fix": if trusted {
            Value::Null
        } else {
            Value::String("vzctl certs ca install".into())
        },
    })
}

fn mac_keychain_trusts(cert: &Path) -> bool {
    let Some(path) = cert.to_str() else {
        return false;
    };
    Command::new("security")
        .args(["verify-cert", "-c", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
                "status": "ok",
                "exit_code": 0,
                "summary": { "message": format!("{command} completed") },
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

fn requested_format(args: &[String]) -> Format {
    if args
        .windows(2)
        .any(|pair| pair[0] == "--format" && pair[1] == "json")
    {
        Format::Json
    } else {
        Format::Human
    }
}

fn fail(format: Format, command: &str, code: u8, message: String) -> ExitCode {
    match format {
        Format::Json => {
            let envelope = json!({
                "apiVersion": API_VERSION,
                "kind": "Error",
                "ok": false,
                "command": command,
                "status": "fail",
                "exit_code": code,
                "summary": { "message": message },
                "error": { "message": message, "code": code },
            });
            eprintln!("{message}");
            println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
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

    fn strict_openssl() -> PathBuf {
        let binary = std::env::var_os("VZCTL_TEST_OPENSSL")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Command::new("brew")
                    .args(["--prefix", "openssl@3"])
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| {
                        PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
                            .join("bin/openssl")
                    })
                    .unwrap_or_else(|| PathBuf::from("openssl"))
            });
        let version = Command::new(&binary)
            .arg("version")
            .output()
            .expect("OpenSSL 3+ required; set VZCTL_TEST_OPENSSL");
        let version = String::from_utf8_lossy(&version.stdout);
        assert!(
            version.starts_with("OpenSSL 3.") || version.starts_with("OpenSSL 4."),
            "strict TLS regression needs OpenSSL 3+; set VZCTL_TEST_OPENSSL (found {version})"
        );
        binary
    }

    #[test]
    fn new_certificates_pass_strict_full_chain_and_hostname_verification() {
        let dir = std::env::temp_dir().join(format!(
            "vzctl-strict-ca-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        ensure_ca(&dir, false).unwrap();
        let host = "auth.svc.strict-test.vz.test";
        mint_leaf(&dir, host, &[]).unwrap();
        let cert = leaf_paths(&dir, host).0;
        let verify = |hostname: &str| {
            Command::new(strict_openssl())
                .args([
                    "verify",
                    "-x509_strict",
                    "-purpose",
                    "sslserver",
                    "-verify_hostname",
                    hostname,
                    "-CAfile",
                ])
                .arg(ca_cert_path(&dir))
                .arg(&cert)
                .output()
                .unwrap()
        };
        let valid = verify(host);
        assert!(
            valid.status.success(),
            "strict full-chain verify failed: {}{}",
            String::from_utf8_lossy(&valid.stdout),
            String::from_utf8_lossy(&valid.stderr)
        );
        assert!(!verify("wrong.strict-test.vz.test").status.success());
        fs::remove_dir_all(dir).unwrap();
    }

    struct TempState(PathBuf);
    impl TempState {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "vzctl-profile-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn legacy_fixture(state: &Path) {
        ensure_dirs(state).unwrap();
        let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.distinguished_name
            .push(DnType::OrganizationName, "legacy-vzctl");
        let key = KeyPair::generate().unwrap();
        let root = ca.self_signed(&key).unwrap();
        let pem = root.pem();
        write_public(&ca_cert_path(state), pem.as_bytes()).unwrap();
        write_secret(&ca_key_path(state), key.serialize_pem().as_bytes()).unwrap();
        write_public(&fingerprint_path(state), fingerprint_pem(&pem).as_bytes()).unwrap();
        write_public(&trust_cert_path(state), pem.as_bytes()).unwrap();
        let leaf = CertificateParams::new(vec![
            "auth.svc.legacy.vz.test".into(),
            "alias.legacy.vz.test".into(),
        ])
        .unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let cert = leaf.signed_by(&leaf_key, &root, &key).unwrap().pem();
        let (cert_path, key_path, meta_path) = leaf_paths(state, "auth.svc.legacy.vz.test");
        write_public(&cert_path, cert.as_bytes()).unwrap();
        write_secret(&key_path, leaf_key.serialize_pem().as_bytes()).unwrap();
        write_public(
            &meta_path,
            serde_json::to_vec(&json!({"fingerprint":fingerprint_pem(&cert),
            "sans":["auth.svc.legacy.vz.test","alias.legacy.vz.test"]}))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
    }

    fn snapshot(path: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn visit(
            root: &Path,
            path: &Path,
            files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
        ) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(root, &entry.path(), files);
                } else {
                    files.insert(
                        entry.path().strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let mut files = std::collections::BTreeMap::new();
        visit(path, path, &mut files);
        files
    }

    #[test]
    fn legacy_profile_requires_explicit_upgrade_without_mutating_state() {
        let temp = TempState::new();
        legacy_fixture(&temp.0);
        let before = snapshot(&temp.0);
        assert!(ensure_ca(&temp.0, false)
            .unwrap_err()
            .contains("certs ca upgrade --output"));
        assert!(mint_leaf(&temp.0, "auth.svc.legacy.vz.test", &[]).is_err());
        assert_eq!(snapshot(&temp.0), before);
    }

    #[test]
    fn upgrade_stages_complete_strict_chains_with_identical_keys_and_root_identity() {
        let temp = TempState::new();
        let state = temp.0.join("active");
        fs::create_dir(&state).unwrap();
        legacy_fixture(&state);
        let before = snapshot(&state);
        let candidate = temp.0.join("candidate");
        let result = upgrade::stage(&state, &candidate).unwrap();
        assert_eq!(result["promoted"], false);
        assert_eq!(result["certificates"], 2);
        assert_eq!(snapshot(&state), before);
        assert_eq!(
            fs::metadata(&candidate).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let manifest: Value =
            serde_json::from_slice(&fs::read(candidate.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["private_keys_unchanged"], true);
        assert_eq!(manifest["root_subject_and_ski_preserved"], true);
        for item in manifest["files"].as_array().unwrap() {
            let relative = item["path"].as_str().unwrap();
            let bytes = fs::read(candidate.join(relative)).unwrap();
            assert_eq!(
                fs::metadata(candidate.join(relative))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            if item["private"] == true {
                assert_eq!(bytes, before[&PathBuf::from(relative)]);
                assert!(item.get("candidate_sha256").is_none());
            } else {
                let digest: String = Sha256::digest(&bytes)
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect();
                assert_eq!(item["candidate_sha256"], digest);
            }
        }
        let old_root = fs::read_to_string(ca_cert_path(&state)).unwrap();
        let new_root = fs::read_to_string(ca_cert_path(&candidate)).unwrap();
        profile::same_root_identity(&old_root, &new_root).unwrap();
        assert_ne!(fingerprint_pem(&old_root), fingerprint_pem(&new_root));
        assert_eq!(ensure_ca(&candidate, false).unwrap()["created"], false);
        for hostname in ["auth.svc.legacy.vz.test", "alias.legacy.vz.test"] {
            let output = Command::new(strict_openssl())
                .args([
                    "verify",
                    "-x509_strict",
                    "-purpose",
                    "sslserver",
                    "-verify_hostname",
                    hostname,
                    "-CAfile",
                ])
                .arg(ca_cert_path(&candidate))
                .arg(leaf_paths(&candidate, "auth.svc.legacy.vz.test").0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for (root_state, leaf_state, expected_error) in [
            (&state, &candidate, "error 92"),
            (&candidate, &state, "error 85"),
        ] {
            let output = Command::new(strict_openssl())
                .args(["verify", "-x509_strict", "-purpose", "sslserver", "-CAfile"])
                .arg(ca_cert_path(root_state))
                .arg(leaf_paths(leaf_state, "auth.svc.legacy.vz.test").0)
                .output()
                .unwrap();
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "mixed chain unexpectedly trusted");
            assert!(diagnostic.contains(expected_error), "{diagnostic}");
        }
        let staged = snapshot(&candidate);
        assert_eq!(
            mint_leaf(
                &candidate,
                "auth.svc.legacy.vz.test",
                &["alias.legacy.vz.test".into()]
            )
            .unwrap()["reused"],
            true
        );
        assert_eq!(snapshot(&candidate), staged);
        assert!(upgrade::stage(&state, &candidate)
            .unwrap_err()
            .contains("already exists"));
        assert_eq!(snapshot(&candidate), staged);
        assert_eq!(snapshot(&state), before);
        assert!(upgrade::stage(&state, &state.join("candidate")).is_err());
    }

    #[test]
    fn incomplete_pairs_and_mismatched_keys_are_never_recreated() {
        let temp = TempState::new();
        ensure_ca(&temp.0, false).unwrap();
        let key = fs::read(ca_key_path(&temp.0)).unwrap();
        fs::remove_file(ca_cert_path(&temp.0)).unwrap();
        assert!(ensure_ca(&temp.0, false)
            .unwrap_err()
            .contains("incomplete"));
        assert_eq!(fs::read(ca_key_path(&temp.0)).unwrap(), key);
        assert!(!ca_cert_path(&temp.0).exists());
        let other = TempState::new();
        legacy_fixture(&other.0);
        write_secret(
            &ca_key_path(&other.0),
            KeyPair::generate().unwrap().serialize_pem().as_bytes(),
        )
        .unwrap();
        let before = snapshot(&other.0);
        let candidate = temp.0.join("bad-candidate");
        assert!(upgrade::stage(&other.0, &candidate)
            .unwrap_err()
            .contains("mismatch"));
        assert!(!candidate.exists());
        assert_eq!(snapshot(&other.0), before);
    }

    #[test]
    fn mint_reuses_the_existing_private_key_and_rejects_half_leaf_pairs() {
        let temp = TempState::new();
        let host = "auth.svc.renew.vz.test";
        mint_leaf(&temp.0, host, &[]).unwrap();
        let (cert, key, _) = leaf_paths(&temp.0, host);
        let key_before = fs::read(&key).unwrap();
        mint_leaf(&temp.0, host, &["alias.renew.vz.test".into()]).unwrap();
        assert_eq!(fs::read(&key).unwrap(), key_before);
        fs::remove_file(&cert).unwrap();
        assert!(mint_leaf(&temp.0, host, &[])
            .unwrap_err()
            .contains("incomplete"));
        assert_eq!(fs::read(&key).unwrap(), key_before);
    }

    #[test]
    fn upgrade_rejects_symlink_source_and_destination_without_changing_targets() {
        use std::os::unix::fs::symlink;
        let temp = TempState::new();
        let state = temp.0.join("active");
        fs::create_dir(&state).unwrap();
        legacy_fixture(&state);
        let candidate = temp.0.join("candidate");
        symlink(&state, &candidate).unwrap();
        assert!(upgrade::stage(&state, &candidate).is_err());
        fs::remove_file(&candidate).unwrap();
        let (cert, _, _) = leaf_paths(&state, "auth.svc.legacy.vz.test");
        let original = fs::read(&cert).unwrap();
        let outside = temp.0.join("outside.pem");
        fs::write(&outside, &original).unwrap();
        fs::remove_file(&cert).unwrap();
        symlink(&outside, &cert).unwrap();
        assert!(upgrade::stage(&state, &candidate).is_err());
        assert!(!candidate.exists());
        assert_eq!(fs::read(&outside).unwrap(), original);
    }

    #[test]
    fn changed_source_snapshot_never_publishes_completion_manifest() {
        let temp = TempState::new();
        let state = temp.0.join("active");
        fs::create_dir(&state).unwrap();
        legacy_fixture(&state);
        let candidate = temp.0.join("candidate");
        let key = ca_key_path(&state);
        let changed = KeyPair::generate().unwrap().serialize_pem();
        let result = upgrade::stage_with_snapshot_hook(&state, &candidate, || {
            fs::write(&key, &changed).unwrap();
        });
        assert!(result.unwrap_err().contains("source changed"));
        assert!(!candidate.join("manifest.json").exists());
        assert_eq!(fs::read_to_string(key).unwrap(), changed);
    }

    #[test]
    fn unsupported_critical_extension_is_not_silently_dropped() {
        let temp = TempState::new();
        let state = temp.0.join("active");
        fs::create_dir(&state).unwrap();
        legacy_fixture(&state);
        let pem = read_ca_pem(&state).unwrap();
        let key = KeyPair::from_pem(&fs::read_to_string(ca_key_path(&state)).unwrap()).unwrap();
        let mut params = CertificateParams::from_ca_cert_pem(&pem).unwrap();
        let mut extension = rcgen::CustomExtension::from_oid_content(&[1, 2, 3, 4], vec![5, 0]);
        extension.set_criticality(true);
        params.custom_extensions.push(extension);
        let cert = params.self_signed(&key).unwrap().pem();
        write_public(&ca_cert_path(&state), cert.as_bytes()).unwrap();
        write_public(&trust_cert_path(&state), cert.as_bytes()).unwrap();
        write_public(&fingerprint_path(&state), fingerprint_pem(&cert).as_bytes()).unwrap();
        let before = snapshot(&state);
        assert!(upgrade::stage(&state, &temp.0.join("candidate"))
            .unwrap_err()
            .contains("unsupported"));
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn expired_leaf_can_be_renewed_without_private_key_rotation() {
        let temp = TempState::new();
        let host = "auth.svc.renew.vz.test";
        mint_leaf(&temp.0, host, &[]).unwrap();
        let (cert, key, _) = leaf_paths(&temp.0, host);
        let key_before = fs::read(&key).unwrap();
        let root_key =
            KeyPair::from_pem(&fs::read_to_string(ca_key_path(&temp.0)).unwrap()).unwrap();
        let root = CertificateParams::from_ca_cert_pem(&read_ca_pem(&temp.0).unwrap())
            .unwrap()
            .self_signed(&root_key)
            .unwrap();
        let mut params =
            CertificateParams::from_ca_cert_pem(&fs::read_to_string(&cert).unwrap()).unwrap();
        params.not_before = OffsetDateTime::now_utc() - Duration::days(3);
        params.not_after = OffsetDateTime::now_utc() - Duration::days(1);
        let leaf_key = KeyPair::from_pem(std::str::from_utf8(&key_before).unwrap()).unwrap();
        let expired = params.signed_by(&leaf_key, &root, &root_key).unwrap().pem();
        write_public(&cert, expired.as_bytes()).unwrap();
        mint_leaf(&temp.0, host, &[]).unwrap();
        assert_eq!(fs::read(key).unwrap(), key_before);
        let verify = Command::new(strict_openssl())
            .args(["verify", "-x509_strict", "-purpose", "sslserver", "-CAfile"])
            .arg(ca_cert_path(&temp.0))
            .arg(cert)
            .output()
            .unwrap();
        assert!(
            verify.status.success(),
            "{}",
            String::from_utf8_lossy(&verify.stderr)
        );
        let before = snapshot(&temp.0);
        assert_eq!(mint_leaf(&temp.0, host, &[]).unwrap()["reused"], true);
        assert_eq!(snapshot(&temp.0), before);
    }

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
