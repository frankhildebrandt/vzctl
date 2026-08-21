use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const VIRTIOFS_BIND_SCRIPT: &str = include_str!("../../../guest-agent/scripts/virtiofs-bind");
pub const ROUTER_APPLY_SCRIPT: &str = include_str!("../../../guest-agent/scripts/router-apply");
pub const CA_INJECT_SCRIPT: &str = include_str!("../../../guest-agent/scripts/ca-inject");
pub const AGENT_SYSTEMD_UNIT: &str =
    include_str!("../../../guest-agent/systemd/vzctl-agent.service");
pub const GUEST_MANIFEST_PATH: &str = "/var/lib/vzctl/utils.manifest.json";
pub const AGENT_BINARY_GUEST_PATH: &str = "/usr/local/sbin/vzctl-agent";
pub const AGENT_METADATA_GUEST_PATH: &str = "/usr/lib/vzctl-agent/image-metadata.json";

const DEPLOY_HEREDOC: &str = "VZCTL_GUEST_UTILS_DEPLOY_EOF";
const CHUNK_SIZE: usize = 180 * 1024;
const AGENT_RESTART_POLL_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct GuestFile {
    pub path: &'static str,
    pub content: String,
    pub mode: &'static str,
}

#[derive(Debug, Clone)]
pub struct GuestUtilsBundle {
    pub bundle_id: String,
    pub agent_version: String,
    pub content_sha256: String,
    pub binary_sha256: String,
    pub cache_dir: PathBuf,
}

#[derive(Debug)]
pub struct GuestUtilsError {
    pub message: String,
}

impl GuestUtilsError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for GuestUtilsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GuestUtilsError {}

pub fn agent_version_string() -> Result<String, GuestUtilsError> {
    if let Ok(version) = std::env::var("VZCTL_AGENT_VERSION") {
        return Ok(version.trim().to_string());
    }
    let version_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../guest-agent/VERSION");
    fs::read_to_string(&version_path)
        .map(|value| value.trim().to_string())
        .map_err(|error| GuestUtilsError::new(format!("read guest-agent/VERSION: {error}")))
}

pub fn guest_deploy_files() -> Vec<GuestFile> {
    vec![
        GuestFile {
            path: "/usr/local/lib/vzctl/virtiofs-bind",
            content: VIRTIOFS_BIND_SCRIPT.to_string(),
            mode: "0755",
        },
        GuestFile {
            path: "/etc/sudoers.d/vzctl-virtiofs",
            content: "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/virtiofs-bind\n"
                .to_string(),
            mode: "0440",
        },
        GuestFile {
            path: "/usr/local/lib/vzctl/router-apply",
            content: ROUTER_APPLY_SCRIPT.to_string(),
            mode: "0755",
        },
        GuestFile {
            path: "/etc/sudoers.d/vzctl-router",
            content: "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/router-apply\n"
                .to_string(),
            mode: "0440",
        },
        GuestFile {
            path: "/usr/local/lib/vzctl/ca-inject",
            content: CA_INJECT_SCRIPT.to_string(),
            mode: "0755",
        },
        GuestFile {
            path: "/etc/sudoers.d/vzctl-ca",
            content: "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/ca-inject\n"
                .to_string(),
            mode: "0440",
        },
        GuestFile {
            path: "/etc/sudoers.d/vzctl-agent",
            content: "vzctl-agent ALL=(ALL) NOPASSWD:ALL\n".to_string(),
            mode: "0440",
        },
        GuestFile {
            path: "/etc/systemd/system/vzctl-agent.service",
            content: AGENT_SYSTEMD_UNIT.to_string(),
            mode: "0644",
        },
    ]
}

pub fn content_fingerprint(agent_version: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(agent_version.as_bytes());
    if let Ok(iwatch_version) = crate::iwatch_bin::iwatch_version_string() {
        hasher.update(iwatch_version.as_bytes());
    }
    for file in guest_deploy_files() {
        hasher.update(file.path.as_bytes());
        hasher.update(file.content.as_bytes());
        hasher.update(file.mode.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn bundle_id(agent_version: &str, content_sha256: &str) -> String {
    format!(
        "{}-{}",
        agent_version,
        &content_sha256[..content_sha256.len().min(12)]
    )
}

pub fn build_agent_staging(agent_version: &str) -> Result<PathBuf, GuestUtilsError> {
    let staging = std::env::temp_dir().join(format!(
        "vzctl-bake-staging-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&staging)
        .map_err(|error| GuestUtilsError::new(format!("cannot create bake staging: {error}")))?;

    let agent_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../guest-agent");
    let binary = staging.join("vzctl-agent");
    let status = Command::new("go")
        .current_dir(&agent_root)
        .env("CGO_ENABLED", "0")
        .env("GOOS", "linux")
        .env("GOARCH", "arm64")
        .args([
            "build",
            "-trimpath",
            "-ldflags",
            &format!("-s -w -buildid= -X main.version={agent_version}"),
            "-o",
        ])
        .arg(&binary)
        .arg("./cmd/vzctl-agent")
        .status()
        .map_err(|error| {
            GuestUtilsError::new(format!(
                "go is required to cross-build vzctl-agent: {error}"
            ))
        })?;
    if !status.success() {
        return Err(GuestUtilsError::new("go build of vzctl-agent failed"));
    }
    for (src, dst) in [
        ("systemd/vzctl-agent.service", "vzctl-agent.service"),
        ("systemd/vzctl-agent.path", "vzctl-agent.path"),
        (
            "systemd/vzctl-agent-tmpfiles.conf",
            "vzctl-agent-tmpfiles.conf",
        ),
        ("openrc/vzctl-agent", "vzctl-agent.openrc"),
    ] {
        fs::copy(agent_root.join(src), staging.join(dst))
            .map_err(|error| GuestUtilsError::new(error.to_string()))?;
    }
    fs::write(
        staging.join("image-metadata.json"),
        format!("{{\"agent_version\":\"{agent_version}\",\"protocol\":1,\"vsock_port\":21950}}\n"),
    )
    .map_err(|error| GuestUtilsError::new(error.to_string()))?;
    crate::iwatch_bin::stage_iwatch_binary(&staging.join("iwatch"))?;
    Ok(staging)
}

fn sha256_file(path: &Path) -> Result<String, GuestUtilsError> {
    let bytes = fs::read(path)
        .map_err(|error| GuestUtilsError::new(format!("read {}: {error}", path.display())))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub fn ensure_cached_bundle(state_dir: &Path) -> Result<GuestUtilsBundle, GuestUtilsError> {
    let agent_version = agent_version_string()?;
    let content_sha256 = content_fingerprint(&agent_version);
    let id = bundle_id(&agent_version, &content_sha256);
    let cache_dir = state_dir.join("guest-utils").join(&id);
    let binary_path = cache_dir.join("vzctl-agent");
    if !binary_path.is_file() {
        fs::create_dir_all(&cache_dir).map_err(|error| {
            GuestUtilsError::new(format!(
                "cannot create guest-utils cache {}: {error}",
                cache_dir.display()
            ))
        })?;
        let staging = build_agent_staging(&agent_version)?;
        for name in [
            "vzctl-agent",
            "vzctl-agent.service",
            "vzctl-agent.path",
            "vzctl-agent-tmpfiles.conf",
            "vzctl-agent.openrc",
            "image-metadata.json",
            "iwatch",
        ] {
            fs::copy(staging.join(name), cache_dir.join(name)).map_err(|error| {
                GuestUtilsError::new(format!("populate guest-utils cache: {error}"))
            })?;
        }
        let _ = fs::remove_dir_all(&staging);
    }
    let binary_sha256 = sha256_file(&binary_path)?;
    Ok(GuestUtilsBundle {
        bundle_id: id,
        agent_version,
        content_sha256,
        binary_sha256,
        cache_dir,
    })
}

pub fn needs_update(guest_bundle_id: Option<&str>, host_bundle_id: &str) -> bool {
    guest_bundle_id != Some(host_bundle_id)
}

pub fn split_chunks(data: &[u8]) -> Vec<&[u8]> {
    if data.is_empty() {
        return vec![];
    }
    data.chunks(CHUNK_SIZE).collect()
}

pub fn deploy_file_shell(path: &str, mode: &str, content: &str) -> String {
    if content.contains(DEPLOY_HEREDOC) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        return format!(
            "set -eu\ntarget={path}\ncat >\"$target.new\" <<'{DEPLOY_HEREDOC}'\n{encoded}\n{DEPLOY_HEREDOC}\nbase64 -d \"$target.new\" >\"$target.new.dec\"\nmv -f \"$target.new.dec\" \"$target.new\"\nchmod {mode} \"$target.new\"\nmv -f \"$target.new\" \"$target\"\n",
            path = sh_escape(path),
            mode = mode,
            encoded = encoded,
        );
    }
    format!(
        "set -eu\ntarget={path}\ncat >\"$target.new\" <<'{DEPLOY_HEREDOC}'\n{content}\n{DEPLOY_HEREDOC}\nchmod {mode} \"$target.new\"\nmv -f \"$target.new\" \"$target\"\n",
        path = sh_escape(path),
        content = content,
        mode = mode,
    )
}

pub fn rollout_to_vm<F>(
    vm_id: &str,
    bundle: &GuestUtilsBundle,
    call: &mut F,
) -> Result<Value, GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let guest_bundle_id = read_guest_bundle_id(vm_id, call)?;
    if !needs_update(guest_bundle_id.as_deref(), &bundle.bundle_id) {
        return Ok(json!({
            "vm_id": vm_id,
            "status": "unchanged",
            "bundle_id": bundle.bundle_id,
        }));
    }

    for file in guest_deploy_files() {
        deploy_guest_file(vm_id, &file, call)?;
    }

    let metadata = fs::read_to_string(bundle.cache_dir.join("image-metadata.json"))
        .map_err(|error| GuestUtilsError::new(error.to_string()))?;
    deploy_guest_file(
        vm_id,
        &GuestFile {
            path: AGENT_METADATA_GUEST_PATH,
            content: metadata,
            mode: "0644",
        },
        call,
    )?;

    let openrc = fs::read_to_string(bundle.cache_dir.join("vzctl-agent.openrc"))
        .map_err(|error| GuestUtilsError::new(error.to_string()))?;
    deploy_guest_file(
        vm_id,
        &GuestFile {
            path: "/etc/init.d/vzctl-agent",
            content: openrc,
            mode: "0755",
        },
        call,
    )?;

    deploy_agent_binary(vm_id, bundle, call)?;
    deploy_iwatch_binary(vm_id, bundle, call)?;
    restart_agent(vm_id, call)?;
    wait_for_agent(vm_id, &bundle.agent_version, call)?;

    let manifest = json!({
        "bundle_id": bundle.bundle_id,
        "agent_version": bundle.agent_version,
        "iwatch_version": crate::iwatch_bin::iwatch_version_string().ok(),
        "content_sha256": bundle.content_sha256,
        "updated_at": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".to_string()),
    });
    deploy_guest_file(
        vm_id,
        &GuestFile {
            path: GUEST_MANIFEST_PATH,
            content: serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n",
            mode: "0644",
        },
        call,
    )?;

    Ok(json!({
        "vm_id": vm_id,
        "status": "upgraded",
        "bundle_id": bundle.bundle_id,
        "agent_version": bundle.agent_version,
    }))
}

pub fn rollout_targets<F>(
    targets: &[String],
    bundle: &GuestUtilsBundle,
    call: &mut F,
) -> Result<Vec<Value>, GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for vm_id in targets {
        match rollout_to_vm(vm_id, bundle, call) {
            Ok(result) => results.push(result),
            Err(error) => failures.push(format!("{vm_id}: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(results)
    } else {
        Err(GuestUtilsError::new(format!(
            "guest utils rollout failed: {}",
            failures.join("; ")
        )))
    }
}

fn read_guest_bundle_id<F>(vm_id: &str, call: &mut F) -> Result<Option<String>, GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let result = guest_exec(
        call,
        vm_id,
        vec![
            "sudo".into(),
            "-n".into(),
            "cat".into(),
            GUEST_MANIFEST_PATH.into(),
        ],
        None,
        10_000,
    );
    match result {
        Ok((0, stdout, _, _)) => {
            let manifest: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
            Ok(manifest["bundle_id"].as_str().map(str::to_string))
        }
        Ok((_, _, _, _)) => Ok(None),
        Err(_) => Ok(None),
    }
}

fn deploy_guest_file<F>(vm_id: &str, file: &GuestFile, call: &mut F) -> Result<(), GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let script = deploy_file_shell(file.path, file.mode, &file.content);
    let (exit, _, stderr, truncated) = guest_exec(
        call,
        vm_id,
        vec!["sudo".into(), "-n".into(), "sh".into(), "-c".into(), script],
        None,
        60_000,
    )?;
    if exit != 0 || truncated {
        return Err(GuestUtilsError::new(format!(
            "deploy {} failed (exit {exit}): {}",
            file.path,
            stderr.trim()
        )));
    }
    Ok(())
}

fn deploy_agent_binary<F>(
    vm_id: &str,
    bundle: &GuestUtilsBundle,
    call: &mut F,
) -> Result<(), GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let binary_path = bundle.cache_dir.join("vzctl-agent");
    let bytes = fs::read(&binary_path)
        .map_err(|error| GuestUtilsError::new(format!("read host agent binary: {error}")))?;
    let staging = "/tmp/vzctl-agent.staging";
    guest_exec(
        call,
        vm_id,
        vec![
            "sudo".into(),
            "-n".into(),
            "sh".into(),
            "-c".into(),
            format!("rm -f {staging}"),
        ],
        None,
        10_000,
    )?;

    for chunk in split_chunks(&bytes) {
        // stdin_b64 is already decoded to raw bytes by the agent; write them
        // with cat. A second `base64 -d` treats ELF as text and fails with
        // "invalid input".
        let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
        let script = format!("cat >> {staging}");
        let (exit, _, stderr, truncated) = guest_exec(
            call,
            vm_id,
            vec!["sudo".into(), "-n".into(), "sh".into(), "-c".into(), script],
            Some(encoded),
            60_000,
        )?;
        if exit != 0 || truncated {
            return Err(GuestUtilsError::new(format!(
                "agent binary chunk upload failed (exit {exit}): {}",
                stderr.trim()
            )));
        }
    }

    let install_script = format!(
        "set -eu\n\
staging={staging}\n\
target={target}\n\
backup={target}.bak\n\
expected={expected}\n\
actual=$(sha256sum \"$staging\" | awk '{{print $1}}')\n\
[ \"$actual\" = \"$expected\" ] || {{ echo \"sha256 mismatch: expected $expected got $actual\" >&2; exit 1; }}\n\
cp -f \"$target\" \"$backup\" 2>/dev/null || true\n\
chmod 0755 \"$staging\"\n\
mv -f \"$staging\" \"$target.new\"\n\
mv -f \"$target.new\" \"$target\"\n",
        staging = sh_escape(staging),
        target = sh_escape(AGENT_BINARY_GUEST_PATH),
        expected = bundle.binary_sha256,
    );
    let (exit, _, stderr, truncated) = guest_exec(
        call,
        vm_id,
        vec![
            "sudo".into(),
            "-n".into(),
            "sh".into(),
            "-c".into(),
            install_script,
        ],
        None,
        60_000,
    )?;
    if exit != 0 || truncated {
        return Err(GuestUtilsError::new(format!(
            "agent binary install failed (exit {exit}): {}",
            stderr.trim()
        )));
    }
    Ok(())
}

fn deploy_iwatch_binary<F>(
    vm_id: &str,
    bundle: &GuestUtilsBundle,
    call: &mut F,
) -> Result<(), GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let binary_path = bundle.cache_dir.join("iwatch");
    let bytes = fs::read(&binary_path)
        .map_err(|error| GuestUtilsError::new(format!("read host iwatch binary: {error}")))?;
    let expected = hex::encode(Sha256::digest(&bytes));
    let staging = "/tmp/iwatch.staging";
    guest_exec(
        call,
        vm_id,
        vec![
            "sudo".into(),
            "-n".into(),
            "sh".into(),
            "-c".into(),
            format!("rm -f {staging}"),
        ],
        None,
        10_000,
    )?;

    for chunk in split_chunks(&bytes) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
        let script = format!("cat >> {staging}");
        let (exit, _, stderr, truncated) = guest_exec(
            call,
            vm_id,
            vec!["sudo".into(), "-n".into(), "sh".into(), "-c".into(), script],
            Some(encoded),
            60_000,
        )?;
        if exit != 0 || truncated {
            return Err(GuestUtilsError::new(format!(
                "iwatch binary chunk upload failed (exit {exit}): {}",
                stderr.trim()
            )));
        }
    }

    let install_script = format!(
        "set -eu\n\
staging={staging}\n\
target={target}\n\
expected={expected}\n\
actual=$(sha256sum \"$staging\" | awk '{{print $1}}')\n\
[ \"$actual\" = \"$expected\" ] || {{ echo \"sha256 mismatch: expected $expected got $actual\" >&2; exit 1; }}\n\
chmod 0755 \"$staging\"\n\
mv -f \"$staging\" \"$target.new\"\n\
mv -f \"$target.new\" \"$target\"\n",
        staging = sh_escape(staging),
        target = sh_escape(crate::iwatch_bin::IWATCH_GUEST_PATH),
        expected = expected,
    );
    let (exit, _, stderr, truncated) = guest_exec(
        call,
        vm_id,
        vec![
            "sudo".into(),
            "-n".into(),
            "sh".into(),
            "-c".into(),
            install_script,
        ],
        None,
        60_000,
    )?;
    if exit != 0 || truncated {
        return Err(GuestUtilsError::new(format!(
            "iwatch binary install failed (exit {exit}): {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Restart the guest agent without waiting for the current vsock session to
/// survive the service stop. Foreground `systemctl restart` kills the agent
/// before the exec response can be written, which surfaces as "connection closed".
fn agent_restart_script() -> &'static str {
    r"set -eu
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  ( systemctl restart vzctl-agent.service >/dev/null 2>&1 & )
elif [ -x /sbin/openrc-run ] && [ -x /etc/init.d/vzctl-agent ]; then
  ( /etc/init.d/vzctl-agent restart >/dev/null 2>&1 & )
else
  echo 'no supported init for vzctl-agent restart' >&2
  exit 1
fi
"
}

fn restart_agent<F>(vm_id: &str, call: &mut F) -> Result<(), GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let script = agent_restart_script();
    match guest_exec(
        call,
        vm_id,
        vec![
            "sudo".into(),
            "-n".into(),
            "sh".into(),
            "-c".into(),
            script.to_string(),
        ],
        None,
        60_000,
    ) {
        Ok((exit, _, stderr, truncated)) => {
            if exit != 0 || truncated {
                return Err(GuestUtilsError::new(format!(
                    "agent restart failed (exit {exit}): {}",
                    stderr.trim()
                )));
            }
            Ok(())
        }
        Err(error) if error.message.contains("connection closed") => Ok(()),
        Err(error) => Err(error),
    }
}

fn wait_for_agent<F>(
    vm_id: &str,
    expected_version: &str,
    call: &mut F,
) -> Result<(), GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let deadline = Instant::now() + Duration::from_secs(AGENT_RESTART_POLL_SECS);
    loop {
        match call(
            "vm.agent.health",
            json!({ "vm_id": vm_id, "timeout_ms": 5_000 }),
        ) {
            Ok(_) => {
                if let Ok(version) = call(
                    "vm.agent.version",
                    json!({ "vm_id": vm_id, "timeout_ms": 5_000 }),
                ) {
                    if version["agent_version"].as_str() == Some(expected_version) {
                        return Ok(());
                    }
                }
            }
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(GuestUtilsError::new(format!(
                "agent did not become ready with version {expected_version} within {AGENT_RESTART_POLL_SECS}s"
            )));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn guest_exec<F>(
    call: &mut F,
    vm_id: &str,
    cmd: Vec<String>,
    stdin_b64: Option<String>,
    timeout_ms: u64,
) -> Result<(u64, String, String, bool), GuestUtilsError>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let mut params = json!({
        "vm_id": vm_id,
        "cmd": cmd,
        "timeout_ms": timeout_ms,
    });
    if let Some(stdin_b64) = stdin_b64 {
        params["stdin_b64"] = json!(stdin_b64);
    }
    let result = call("vm.exec", params).map_err(GuestUtilsError::new)?;
    Ok((
        result["exit"].as_u64().unwrap_or(1),
        result["stdout"].as_str().unwrap_or("").to_string(),
        result["stderr"].as_str().unwrap_or("").to_string(),
        result["truncated"].as_bool().unwrap_or(false),
    ))
}

fn sh_escape(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "/._-:+".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        &LOCK
    }

    #[test]
    fn bundle_id_uses_version_and_hash_prefix() {
        let hash = "abcdef0123456789";
        assert_eq!(bundle_id("0.1.3", hash), "0.1.3-abcdef012345");
    }

    #[test]
    fn rollout_skips_when_bundle_matches() {
        let bundle = GuestUtilsBundle {
            bundle_id: "0.1.3-deadbeef".to_string(),
            agent_version: "0.1.3".to_string(),
            content_sha256: "abc".to_string(),
            binary_sha256: "def".to_string(),
            cache_dir: PathBuf::from("/tmp"),
        };
        let mut calls = Vec::new();
        let result = rollout_to_vm("demo/web", &bundle, &mut |method, params| {
            calls.push((method.to_string(), params));
            if method == "vm.exec" {
                return Ok(json!({
                    "exit": 0,
                    "stdout": "{\"bundle_id\":\"0.1.3-deadbeef\"}",
                    "stderr": "",
                    "truncated": false,
                }));
            }
            Ok(json!({}))
        })
        .unwrap();
        assert_eq!(result["status"], "unchanged");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "vm.exec");
    }

    #[test]
    fn needs_update_compares_bundle_ids() {
        assert!(!needs_update(Some("0.1.3-deadbeef"), "0.1.3-deadbeef"));
        assert!(needs_update(Some("0.1.2-old"), "0.1.3-deadbeef"));
        assert!(needs_update(None, "0.1.3-deadbeef"));
    }

    #[test]
    fn split_chunks_respects_limit() {
        let data = vec![0_u8; CHUNK_SIZE * 2 + 10];
        let chunks = split_chunks(&data);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), CHUNK_SIZE);
        assert_eq!(chunks[1].len(), CHUNK_SIZE);
        assert_eq!(chunks[2].len(), 10);
    }

    #[test]
    fn agent_restart_prefers_systemd_over_openrc_initd() {
        let script = agent_restart_script();
        let systemd_at = script.find("/run/systemd/system").expect("systemd probe");
        let openrc_at = script
            .find("/etc/init.d/vzctl-agent")
            .expect("openrc fallback");
        assert!(systemd_at < openrc_at);
        assert!(script.contains("( systemctl restart vzctl-agent.service >/dev/null 2>&1 & )"));
        assert!(script.contains("( /etc/init.d/vzctl-agent restart >/dev/null 2>&1 & )"));
    }

    #[test]
    fn agent_binary_upload_pipes_raw_stdin_through_cat() {
        let dir = std::env::temp_dir().join(format!(
            "vzctl-agent-upload-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("vzctl-agent"), b"\x7fELFpayload").unwrap();
        let bundle = GuestUtilsBundle {
            bundle_id: "0.1.4-test".into(),
            agent_version: "0.1.4".into(),
            content_sha256: "abc".into(),
            binary_sha256: "def".into(),
            cache_dir: dir.clone(),
        };
        let mut scripts = Vec::new();
        let _ = deploy_agent_binary("web", &bundle, &mut |_method, params| {
            if let Some(cmd) = params["cmd"].as_array() {
                if let Some(script) = cmd.last().and_then(Value::as_str) {
                    scripts.push(script.to_string());
                }
            }
            Ok(json!({
                "exit": 0,
                "stdout": "",
                "stderr": "",
                "truncated": false,
            }))
        });
        let _ = fs::remove_dir_all(&dir);
        assert!(
            scripts.iter().any(|script| script.contains("cat >>")),
            "expected cat append, got {scripts:?}"
        );
        assert!(
            scripts.iter().all(|script| !script.contains("base64 -d")),
            "stdin_b64 is already decoded; base64 -d would fail, got {scripts:?}"
        );
    }

    #[test]
    fn deploy_file_shell_uses_heredoc() {
        let script = deploy_file_shell("/tmp/test", "0755", "#!/bin/sh\necho hi\n");
        assert!(script.contains(DEPLOY_HEREDOC));
        assert!(script.contains("echo hi"));
    }

    #[test]
    fn content_fingerprint_is_stable() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("VZCTL_IWATCH_VERSION");
        std::env::remove_var("VZCTL_IWATCH_BIN");
        let first = content_fingerprint("0.1.3");
        let second = content_fingerprint("0.1.3");
        assert_eq!(first, second);
        assert_ne!(first, content_fingerprint("0.1.4"));
    }

    #[test]
    fn content_fingerprint_changes_with_iwatch_pin() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("VZCTL_IWATCH_BIN");
        std::env::set_var("VZCTL_IWATCH_VERSION", "v1.0.0");
        let first = content_fingerprint("0.1.7");
        std::env::set_var("VZCTL_IWATCH_VERSION", "v1.0.1");
        let second = content_fingerprint("0.1.7");
        std::env::remove_var("VZCTL_IWATCH_VERSION");
        assert_ne!(first, second);
    }
}
