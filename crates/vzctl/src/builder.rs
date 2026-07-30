//! Ephemeral Builder-VM backend for `image seal` / `image bake` on macOS.
//!
//! When local `virt-customize` is missing, vzctl boots a pinned Linux appliance
//! (root disk), attaches the target raw image as data disk, runs a cloud-init
//! runbook, and parses `VZCTL_BUILDER_RESULT` from the helper serial log.

use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[allow(dead_code)]
pub const BUILDER_ALIAS: &str = "vzctl-builder";
pub const BUILDER_RESULT_PREFIX: &str = "VZCTL_BUILDER_RESULT ";
const DEFAULT_TIMEOUT_SECS: u64 = 900;
const BUILDER_RELEASE_TAG: &str = "builder-v1";

#[derive(Debug)]
pub struct BuilderFailure {
    pub code: u8,
    pub message: String,
}

impl BuilderFailure {
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageBackendKind {
    Local,
    Builder,
}

impl ImageBackendKind {
    pub fn from_env() -> Option<Self> {
        match std::env::var("VZCTL_IMAGE_BACKEND")
            .ok()
            .as_deref()
            .map(str::trim)
        {
            Some("local") => Some(Self::Local),
            Some("builder") => Some(Self::Builder),
            _ => None,
        }
    }
}

pub fn virt_customize_available() -> bool {
    Command::new("virt-customize")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn qemu_img_available() -> bool {
    Command::new("qemu-img")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn select_backend_kind() -> Result<ImageBackendKind, BuilderFailure> {
    if let Some(kind) = ImageBackendKind::from_env() {
        return match kind {
            ImageBackendKind::Local if !virt_customize_available() || !qemu_img_available() => {
                Err(BuilderFailure::new(
                    12,
                    "VZCTL_IMAGE_BACKEND=local requires virt-customize and qemu-img",
                ))
            }
            other => Ok(other),
        };
    }
    if virt_customize_available() && qemu_img_available() {
        return Ok(ImageBackendKind::Local);
    }
    Ok(ImageBackendKind::Builder)
}

pub fn builder_cache_dir(images_dir: &Path) -> PathBuf {
    images_dir.join("builder")
}

pub fn builder_image_path(images_dir: &Path) -> PathBuf {
    builder_cache_dir(images_dir).join("vzctl-builder.raw")
}

pub fn resolve_builder_image(images_dir: &Path) -> Result<PathBuf, BuilderFailure> {
    if let Some(path) = std::env::var_os("VZCTL_BUILDER_IMAGE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(BuilderFailure::new(
            12,
            format!(
                "VZCTL_BUILDER_IMAGE is not a file: {}",
                path.display()
            ),
        ));
    }
    let cached = builder_image_path(images_dir);
    if cached.is_file() {
        verify_cached_sha256(&cached)?;
        return Ok(cached);
    }
    Err(BuilderFailure::new(
        12,
        format!(
            "builder appliance missing at {} (and virt-customize unavailable). \
             Build with scripts/build-builder-appliance.sh on ARM64 Linux, then \
             copy to that path or set VZCTL_BUILDER_IMAGE. Release tag: {BUILDER_RELEASE_TAG}",
            cached.display()
        ),
    ))
}

fn verify_cached_sha256(path: &Path) -> Result<(), BuilderFailure> {
    let digest_path = path.with_extension("sha256");
    if !digest_path.is_file() {
        return Ok(());
    }
    let expected = fs::read_to_string(&digest_path)
        .map_err(|error| {
            BuilderFailure::new(12, format!("cannot read {}: {error}", digest_path.display()))
        })?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(());
    }
    let actual = sha256_file(path)?;
    if actual != expected {
        return Err(BuilderFailure::new(
            12,
            format!(
                "builder appliance checksum mismatch for {}: expected {expected}, got {actual}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, BuilderFailure> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|error| {
        BuilderFailure::new(12, format!("cannot open {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 64];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            BuilderFailure::new(12, format!("cannot read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hex encode without adding a hex crate — small helper.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct BuilderResult {
    pub ok: bool,
    pub exit: u8,
    pub phase: Option<String>,
    pub message: Option<String>,
    #[allow(dead_code)]
    pub op: Option<String>,
}

pub fn parse_builder_result_line(line: &str) -> Option<BuilderResult> {
    let payload = line.trim().strip_prefix(BUILDER_RESULT_PREFIX)?;
    let value: Value = serde_json::from_str(payload).ok()?;
    Some(BuilderResult {
        ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        exit: value
            .get("exit")
            .and_then(Value::as_u64)
            .unwrap_or(13)
            .min(255) as u8,
        phase: value
            .get("phase")
            .and_then(Value::as_str)
            .map(str::to_string),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        op: value
            .get("op")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub fn find_builder_result(serial_log: &Path) -> Option<BuilderResult> {
    let file = File::open(serial_log).ok()?;
    let mut last = None;
    for line in BufReader::new(file).lines().flatten() {
        if let Some(result) = parse_builder_result_line(&line) {
            last = Some(result);
        }
    }
    last
}

pub struct SealRunbook {
    pub op: &'static str,
    pub commands: Vec<String>,
}

pub fn seal_runbook() -> SealRunbook {
    let mut commands = Vec::new();
    for check in crate::IMAGE_PRESERVATION_CHECKS {
        commands.push(format!(
            "virt-customize -a /dev/vdb --format raw --run-command {check:?} || {{ printf 'VZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"precheck\",\"exit\":14,\"message\":\"preservation failed\"}}\\n'; sync; poweroff; exit 1; }}"
        ));
    }
    for cleanup in crate::IMAGE_CLEANUP_COMMANDS {
        commands.push(format!(
            "virt-customize -a /dev/vdb --format raw --run-command {cleanup:?} || {{ printf 'VZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"customize\",\"exit\":13,\"message\":\"cleanup failed\"}}\\n'; sync; poweroff; exit 1; }}"
        ));
    }
    for check in crate::IMAGE_PRESERVATION_CHECKS {
        commands.push(format!(
            "virt-customize -a /dev/vdb --format raw --run-command {check:?} || {{ printf 'VZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"postcheck\",\"exit\":14,\"message\":\"preservation failed\"}}\\n'; sync; poweroff; exit 1; }}"
        ));
    }
    for check in crate::IMAGE_CLONE_SAFE_CHECKS {
        commands.push(format!(
            "virt-customize -a /dev/vdb --format raw --run-command {check:?} || {{ printf 'VZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"clone-safe\",\"exit\":14,\"message\":\"clone-safe failed\"}}\\n'; sync; poweroff; exit 1; }}"
        ));
    }
    commands.push(
        "printf 'VZCTL_BUILDER_RESULT {\"ok\":true,\"phase\":\"done\",\"exit\":0,\"op\":\"seal\"}\\n'"
            .to_string(),
    );
    commands.push("sync".to_string());
    commands.push("poweroff".to_string());
    SealRunbook {
        op: "seal",
        commands,
    }
}

pub fn bake_runbook(staging_mount_hint: &str) -> SealRunbook {
    // Staging files are placed on cidata and copied into a host-visible path via
    // a second approach: we inject via virt-customize --copy-in from a directory
    // on the appliance root that cloud-init writes from user-data write_files.
    let _ = staging_mount_hint;
    let commands = vec![
        "test -f /var/lib/vzctl-builder/staging/vzctl-agent".to_string(),
        "virt-customize -a /dev/vdb --format raw \
            --mkdir /usr/lib/vzctl-agent \
            --copy-in /var/lib/vzctl-builder/staging/vzctl-agent:/usr/local/sbin \
            --copy-in /var/lib/vzctl-builder/staging/vzctl-agent.service:/etc/systemd/system \
            --copy-in /var/lib/vzctl-builder/staging/vzctl-agent-tmpfiles.conf:/usr/lib/tmpfiles.d \
            --copy-in /var/lib/vzctl-builder/staging/image-metadata.json:/usr/lib/vzctl-agent \
            --run-command 'id -u vzctl-agent >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin vzctl-agent' \
            --run-command 'chmod 0755 /usr/local/sbin/vzctl-agent && chmod 0644 /etc/systemd/system/vzctl-agent.service /usr/lib/tmpfiles.d/vzctl-agent-tmpfiles.conf /usr/lib/vzctl-agent/image-metadata.json' \
            --run-command 'systemctl enable vzctl-agent.service' \
            || { printf 'VZCTL_BUILDER_RESULT {\"ok\":false,\"phase\":\"bake\",\"exit\":13,\"message\":\"bake failed\"}\\n'; sync; poweroff; exit 1; }"
            .replace('\n', " "),
        "printf 'VZCTL_BUILDER_RESULT {\"ok\":true,\"phase\":\"done\",\"exit\":0,\"op\":\"bake\"}\\n'"
            .to_string(),
        "sync".to_string(),
        "poweroff".to_string(),
    ];
    SealRunbook {
        op: "bake",
        commands,
    }
}

pub struct BuilderRunOptions<'a> {
    pub appliance: &'a Path,
    pub target_raw: &'a Path,
    pub runbook: &'a SealRunbook,
    pub staging_dir: Option<&'a Path>,
    pub timeout: Duration,
    pub progress: bool,
}

pub fn run_builder_vm(options: BuilderRunOptions<'_>) -> Result<BuilderResult, BuilderFailure> {
    if options.progress {
        eprintln!("Starting builder VM…");
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let work = std::env::temp_dir().join(format!(
        "vzctl-builder-{}-{}",
        std::process::id(),
        nonce
    ));
    let bundle = work.join("bundle");
    let state = work.join("state");
    let seed = work.join("seed");
    fs::create_dir_all(&bundle).map_err(io_err)?;
    fs::create_dir_all(&state).map_err(io_err)?;
    fs::create_dir_all(&seed).map_err(io_err)?;

    let disk = bundle.join("disk.raw");
    let data_disk = bundle.join("dataDisk.raw");
    copy_or_clone(options.appliance, &disk)?;
    // Target must be raw; work on a sibling temp then rename is handled by caller.
    // Here we operate in-place on the provided target via hardlink/clone when possible.
    link_or_copy(options.target_raw, &data_disk)?;

    write_seed(&seed, options.runbook, options.staging_dir)?;

    let cidata = bundle.join("cidata.iso");
    create_cidata_iso(&seed, &cidata)?;

    let helper = helper_path()?;
    let vm_id = format!("vzctl-builder-{nonce}");
    let mut child = Command::new(&helper)
        .args([
            "run",
            "--vm-id",
            &vm_id,
            "--bundle",
            bundle.to_str().ok_or_else(|| {
                BuilderFailure::new(12, "builder bundle path is not UTF-8")
            })?,
            "--disk",
            disk.to_str().ok_or_else(|| {
                BuilderFailure::new(12, "builder disk path is not UTF-8")
            })?,
            "--data-disk",
            data_disk.to_str().ok_or_else(|| {
                BuilderFailure::new(12, "builder data-disk path is not UTF-8")
            })?,
            "--cidata",
            cidata.to_str().ok_or_else(|| {
                BuilderFailure::new(12, "builder cidata path is not UTF-8")
            })?,
            "--supervisor-sock",
            state.join("missing.sock").to_str().unwrap_or("/tmp/vzctl-missing.sock"),
        ])
        .env("VZCTL_STATE_DIR", &state)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            BuilderFailure::new(12, format!("cannot start vz-helper: {error}"))
        })?;

    let result = match wait_for_result(&mut child, &vm_id, options.timeout) {
        Ok(result) => {
            let _ = child.kill();
            let _ = child.wait();
            // Sync target bytes back if we used a copy (link_or_copy may have copied).
            if data_disk.exists() && data_disk != options.target_raw {
                // If hardlinked/cloned, mutations already visible; if copied, copy back.
                if !same_file(&data_disk, options.target_raw) {
                    fs::copy(&data_disk, options.target_raw).map_err(io_err)?;
                }
            }
            result
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_dir_all(&work);
            return Err(error);
        }
    };

    let _ = fs::remove_dir_all(&work);
    if !result.ok {
        return Err(BuilderFailure::new(
            result.exit,
            result
                .message
                .clone()
                .unwrap_or_else(|| {
                    format!(
                        "builder {} failed in phase {}",
                        options.runbook.op,
                        result.phase.as_deref().unwrap_or("unknown")
                    )
                }),
        ));
    }
    Ok(result)
}

fn wait_for_result(
    child: &mut Child,
    vm_id: &str,
    timeout: Duration,
) -> Result<BuilderResult, BuilderFailure> {
    let started = Instant::now();
    let mut serial_path: Option<PathBuf> = None;
    let mut stdout_buf = String::new();

    if let Some(stdout) = child.stdout.as_mut() {
        // Non-blocking-ish: read available after short waits via try_wait loop.
        let _ = stdout;
    }

    loop {
        if started.elapsed() > timeout {
            return Err(BuilderFailure::new(
                13,
                format!("builder timed out after {}s waiting for result marker", timeout.as_secs()),
            ));
        }

        if let Some(stdout) = child.stdout.as_mut() {
            let mut chunk = [0_u8; 4096];
            // Best-effort non-blocking: set nonblock on fd.
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                let fd = stdout.as_raw_fd();
                unsafe {
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    if flags >= 0 {
                        let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                }
                if let Ok(n) = stdout.read(&mut chunk) {
                    if n > 0 {
                        stdout_buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
                        if let Some(path) = parse_serial_path(&stdout_buf) {
                            serial_path = Some(path);
                        }
                    }
                }
            }
        }

        if serial_path.is_none() {
            if let Some(path) = find_serial_log(vm_id) {
                serial_path = Some(path);
            }
        }

        if let Some(path) = &serial_path {
            if let Some(result) = find_builder_result(path) {
                return Ok(result);
            }
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(path) = &serial_path {
                    if let Some(result) = find_builder_result(path) {
                        return Ok(result);
                    }
                }
                return Err(BuilderFailure::new(
                    13,
                    format!(
                        "builder helper exited ({status}) without VZCTL_BUILDER_RESULT marker"
                    ),
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(BuilderFailure::new(
                    13,
                    format!("cannot wait for builder helper: {error}"),
                ));
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn parse_serial_path(stdout: &str) -> Option<PathBuf> {
    for token in stdout.split_whitespace() {
        if let Some(path) = token.strip_prefix("serial=") {
            let path = PathBuf::from(path);
            if path.as_os_str().len() > 0 {
                return Some(path);
            }
        }
    }
    None
}

fn find_serial_log(vm_id: &str) -> Option<PathBuf> {
    let logs = dirs_logs();
    let entries = fs::read_dir(logs).ok()?;
    let mut best: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains("vzctl-builder") && name.ends_with(".serial.log") {
            // Prefer files mentioning a sanitized form of vm_id prefix.
            let path = entry.path();
            if name.contains(&sanitize_component(vm_id)) || best.is_none() {
                best = Some(path);
            }
        }
    }
    best
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn dirs_logs() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Logs/vzctl")
}

fn helper_path() -> Result<PathBuf, BuilderFailure> {
    if let Some(path) = std::env::var_os("VZCTL_HELPER_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Ok(path) = which("vz-helper") {
        return Ok(path);
    }
    let local = dirs_home()
        .join(".local/bin/vz-helper");
    if local.is_file() {
        return Ok(local);
    }
    Err(BuilderFailure::new(
        12,
        "vz-helper not found; set VZCTL_HELPER_PATH or install vz-helper",
    ))
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let output = Command::new("/usr/bin/which")
        .arg(name)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Err(())
    } else {
        Ok(PathBuf::from(path))
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn write_seed(
    seed: &Path,
    runbook: &SealRunbook,
    staging: Option<&Path>,
) -> Result<(), BuilderFailure> {
    let instance_id = format!(
        "vzctl-builder-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    fs::write(
        seed.join("meta-data"),
        format!("instance-id: {instance_id}\nlocal-hostname: vzctl-builder\n"),
    )
    .map_err(io_err)?;

    let mut write_files = String::new();
    if let Some(staging) = staging {
        for name in [
            "vzctl-agent",
            "vzctl-agent.service",
            "vzctl-agent-tmpfiles.conf",
            "image-metadata.json",
        ] {
            let path = staging.join(name);
            let bytes = fs::read(&path).map_err(|error| {
                BuilderFailure::new(
                    12,
                    format!("missing bake staging file {}: {error}", path.display()),
                )
            })?;
            let b64 = base64_encode(&bytes);
            let mode = if name == "vzctl-agent" { "0755" } else { "0644" };
            write_files.push_str(&format!(
                "  - path: /var/lib/vzctl-builder/staging/{name}\n    permissions: '{mode}'\n    encoding: b64\n    content: {b64}\n"
            ));
        }
    }

    let runcmd = runbook
        .commands
        .iter()
        .map(|command| format!("  - |\n    {command}"))
        .collect::<Vec<_>>()
        .join("\n");

    let user_data = format!(
        "#cloud-config\nwrite_files:\n{write_files}runcmd:\n{runcmd}\n"
    );
    fs::write(seed.join("user-data"), user_data).map_err(io_err)?;
    // Empty network-config keeps cloud-init happy without DHCP waits when NAT is present.
    fs::write(seed.join("network-config"), "version: 2\n").map_err(io_err)?;
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() {
            bytes[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < bytes.len() {
            bytes[i + 2] as u32
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn create_cidata_iso(seed: &Path, destination: &Path) -> Result<(), BuilderFailure> {
    let output = Command::new("hdiutil")
        .args([
            "makehybrid",
            "-iso",
            "-joliet",
            "-default-volume-name",
            "cidata",
            "-o",
        ])
        .arg(destination)
        .arg(seed)
        .output()
        .map_err(|error| {
            BuilderFailure::new(12, format!("cannot start hdiutil: {error}"))
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BuilderFailure::new(
            12,
            format!(
                "hdiutil failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

fn copy_or_clone(source: &Path, destination: &Path) -> Result<(), BuilderFailure> {
    #[cfg(target_os = "macos")]
    {
        if clonefile(source, destination).is_ok() {
            return Ok(());
        }
    }
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(io_err)
}

fn link_or_copy(source: &Path, destination: &Path) -> Result<(), BuilderFailure> {
    #[cfg(target_os = "macos")]
    {
        if clonefile(source, destination).is_ok() {
            return Ok(());
        }
    }
    if fs::hard_link(source, destination).is_ok() {
        return Ok(());
    }
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(io_err)
}

#[cfg(target_os = "macos")]
fn clonefile(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn clonefile(source: *const c_char, destination: *const c_char, flags: c_int) -> c_int;
    }
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
    if unsafe { clonefile(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => {
            use std::os::unix::fs::MetadataExt;
            ma.dev() == mb.dev() && ma.ino() == mb.ino()
        }
        _ => false,
    }
}

fn io_err(error: io::Error) -> BuilderFailure {
    BuilderFailure::new(12, error.to_string())
}

pub fn doctor_builder_check(images_dir: &Path) -> (String, String, Value) {
    let local_ok = virt_customize_available() && qemu_img_available();
    let cached = builder_image_path(images_dir);
    let cache_ok = cached.is_file() && verify_cached_sha256(&cached).is_ok();
    let env_set = std::env::var_os("VZCTL_BUILDER_IMAGE").is_some();
    let details = json!({
        "local_virt_customize": local_ok,
        "appliance_cached": cache_ok,
        "appliance_path": cached,
        "VZCTL_BUILDER_IMAGE": env_set,
        "backend": select_backend_kind().ok().map(|k| match k {
            ImageBackendKind::Local => "local",
            ImageBackendKind::Builder => "builder",
        }),
    });
    if local_ok || cache_ok || env_set {
        (
            "ok".into(),
            if local_ok {
                "local virt-customize/qemu-img available for image seal/bake".into()
            } else {
                "builder appliance available for image seal/bake".into()
            },
            details,
        )
    } else {
        (
            "warn".into(),
            format!(
                "no image customization backend: install virt-customize or cache appliance at {}",
                cached.display()
            ),
            details,
        )
    }
}

pub fn default_timeout() -> Duration {
    std::env::var("VZCTL_BUILDER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_builder_result_json() {
        let result = parse_builder_result_line(
            "VZCTL_BUILDER_RESULT {\"ok\":true,\"phase\":\"done\",\"exit\":0,\"op\":\"seal\"}",
        )
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.exit, 0);
        assert_eq!(result.phase.as_deref(), Some("done"));
    }

    #[test]
    fn rejects_malformed_builder_result() {
        assert!(parse_builder_result_line("VZCTL_BUILDER_RESULT not-json").is_none());
        assert!(parse_builder_result_line("other line").is_none());
    }
}
