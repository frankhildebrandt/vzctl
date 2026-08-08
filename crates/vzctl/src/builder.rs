//! Ephemeral Builder-VM backend for `image seal` / `image bake` on macOS.
//!
//! When local `virt-customize` is missing, vzctl boots a pinned Linux appliance
//! (root disk), attaches the target raw image as data disk, runs a cloud-init
//! runbook, and parses `VZCTL_BUILDER_RESULT` from the helper serial log.

use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
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
            format!("VZCTL_BUILDER_IMAGE is not a file: {}", path.display()),
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
            BuilderFailure::new(
                12,
                format!("cannot read {}: {error}", digest_path.display()),
            )
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
    // Serial logs may prepend a getty login prompt on the same line.
    let trimmed = line.trim();
    let payload = match trimmed.strip_prefix(BUILDER_RESULT_PREFIX) {
        Some(payload) => payload,
        None => {
            let idx = trimmed.find(BUILDER_RESULT_PREFIX)?;
            &trimmed[idx + BUILDER_RESULT_PREFIX.len()..]
        }
    };
    let value: Value = serde_json::from_str(payload.trim()).ok()?;
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
        op: value.get("op").and_then(Value::as_str).map(str::to_string),
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
    let mut body = String::from("set -e; ");
    body.push_str(&mount_target_script());
    body.push_str("; ");
    for check in crate::IMAGE_PRESERVATION_CHECKS {
        let msg = format!("preservation failed: {check}");
        body.push_str(&format!(
            "chroot /mnt/target /bin/sh -c {check:?} || {{ printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"precheck\",\"exit\":14,\"message\":{msg:?}}}\\n' > /dev/hvc0; sync; poweroff; exit 1; }}; "
        ));
    }
    for cleanup in crate::IMAGE_CLEANUP_COMMANDS {
        let msg = format!("cleanup failed: {cleanup}");
        body.push_str(&format!(
            "chroot /mnt/target /bin/sh -c {cleanup:?} || {{ printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"customize\",\"exit\":13,\"message\":{msg:?}}}\\n' > /dev/hvc0; sync; poweroff; exit 1; }}; "
        ));
    }
    for check in crate::IMAGE_PRESERVATION_CHECKS {
        let msg = format!("preservation failed: {check}");
        body.push_str(&format!(
            "chroot /mnt/target /bin/sh -c {check:?} || {{ printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"postcheck\",\"exit\":14,\"message\":{msg:?}}}\\n' > /dev/hvc0; sync; poweroff; exit 1; }}; "
        ));
    }
    for check in crate::IMAGE_CLONE_SAFE_CHECKS {
        body.push_str(&format!(
            "chroot /mnt/target /bin/sh -c {check:?} || {{ printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"clone-safe\",\"exit\":14,\"message\":\"clone-safe failed\"}}\\n' > /dev/hvc0; sync; poweroff; exit 1; }}; "
        ));
    }
    body.push_str(&unmount_target_script());
    body.push_str("; ");
    body.push_str(
        "printf '\\nVZCTL_BUILDER_RESULT {\"ok\":true,\"phase\":\"done\",\"exit\":0,\"op\":\"seal\"}\\n' > /dev/hvc0; sync; poweroff",
    );

    // Single runcmd so a failed step cannot be followed by a spurious ok marker.
    SealRunbook {
        op: "seal",
        commands: vec![format!(
            "( {body} ) || {{ printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"seal\",\"exit\":13,\"message\":\"seal failed\"}}\\n' > /dev/hvc0; sync; poweroff; exit 1; }}"
        )],
    }
}

pub fn bake_runbook(staging_mount_hint: &str) -> SealRunbook {
    // Staging files land on cidata via cloud-init write_files, then are copied
    // into the mounted target root. Nested virt-customize/qemu is avoided
    // because Apple Virtualization does not support nested KVM reliably.
    let _ = staging_mount_hint;
    let bake = format!(
        "set -e; \
lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,MOUNTPOINTS >/dev/hvc0 || true; \
test -f /var/lib/vzctl-builder/staging/vzctl-agent; \
{mount}; \
mkdir -p /mnt/target/usr/lib/vzctl-agent /mnt/target/usr/local/sbin /mnt/target/etc/systemd/system /mnt/target/usr/lib/tmpfiles.d /mnt/target/etc/init.d /mnt/target/var/lib/vzctl; \
cp /var/lib/vzctl-builder/staging/vzctl-agent /mnt/target/usr/local/sbin/vzctl-agent; \
cp /var/lib/vzctl-builder/staging/image-metadata.json /mnt/target/usr/lib/vzctl-agent/image-metadata.json; \
chmod 0755 /mnt/target/usr/local/sbin/vzctl-agent; \
chmod 0644 /mnt/target/usr/lib/vzctl-agent/image-metadata.json; \
if ! chroot /mnt/target /bin/sh -c 'id -u vzctl-agent >/dev/null 2>&1'; then \
  if chroot /mnt/target /bin/sh -c 'command -v useradd >/dev/null 2>&1'; then \
    chroot /mnt/target /bin/sh -c 'useradd --system --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin vzctl-agent'; \
  elif chroot /mnt/target /bin/sh -c 'command -v adduser >/dev/null 2>&1'; then \
    chroot /mnt/target /bin/sh -c 'adduser -S -D -H -s /sbin/nologin vzctl-agent'; \
  else \
    printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"bake\",\"exit\":13,\"message\":\"cannot create vzctl-agent user\"}}\\n' > /dev/hvc0; sync; poweroff; exit 1; \
  fi; \
fi; \
if [ -d /mnt/target/etc/init.d ] && [ -d /mnt/target/etc/runlevels/default ]; then \
  cp /var/lib/vzctl-builder/staging/vzctl-agent.openrc /mnt/target/etc/init.d/vzctl-agent; \
  chmod 0755 /mnt/target/etc/init.d/vzctl-agent; \
  ln -sfn /etc/init.d/vzctl-agent /mnt/target/etc/runlevels/default/vzctl-agent; \
  chroot /mnt/target /bin/sh -c 'command -v rc-update >/dev/null 2>&1 && rc-update add vzctl-agent default' || true; \
elif [ -d /mnt/target/etc/systemd/system ] && chroot /mnt/target /bin/sh -c 'command -v systemctl >/dev/null 2>&1'; then \
  cp /var/lib/vzctl-builder/staging/vzctl-agent.service /mnt/target/etc/systemd/system/vzctl-agent.service; \
  cp /var/lib/vzctl-builder/staging/vzctl-agent.path /mnt/target/etc/systemd/system/vzctl-agent.path; \
  cp /var/lib/vzctl-builder/staging/vzctl-agent-tmpfiles.conf /mnt/target/usr/lib/tmpfiles.d/vzctl-agent-tmpfiles.conf; \
  chmod 0644 /mnt/target/etc/systemd/system/vzctl-agent.service /mnt/target/etc/systemd/system/vzctl-agent.path /mnt/target/usr/lib/tmpfiles.d/vzctl-agent-tmpfiles.conf; \
  systemctl enable --root=/mnt/target vzctl-agent.service vzctl-agent.path; \
else \
  printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"bake\",\"exit\":13,\"message\":\"target has neither systemd nor OpenRC\"}}\\n' > /dev/hvc0; sync; poweroff; exit 1; \
fi; \
{unmount}; \
printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":true,\"phase\":\"done\",\"exit\":0,\"op\":\"bake\"}}\\n' > /dev/hvc0; \
sync; \
poweroff",
        mount = mount_target_script(),
        unmount = unmount_target_script(),
    );
    // Flatten Rust string continuations to one shell line without leaving `\` tokens.
    let bake = bake
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    SealRunbook {
        op: "bake",
        commands: vec![format!(
            "( {bake} ) || {{ printf '\\nVZCTL_BUILDER_RESULT {{\"ok\":false,\"phase\":\"bake\",\"exit\":13,\"message\":\"bake failed\"}}\\n' > /dev/hvc0; sync; poweroff; exit 1; }}"
        )],
    }
}

fn resolve_target_root_script() -> &'static str {
    // Target image is always attached as the data disk (/dev/vdb). Never use
    // findfs across all disks — the builder appliance may share common labels
    // (or alpine's first partition is a tiny EFI FAT, not the root).
    // Keep this a single shell line (no \ continuations): mount scripts are
    // flattened into one cloud-init runcmd.
    // Alpine cloud images label the root as "/" ; Ubuntu uses cloudimg-rootfs.
    r#"ROOT=""; for part in /dev/vdb[0-9]* /dev/vdbp[0-9]*; do [ -b "$part" ] || continue; label=$(blkid -o value -s LABEL "$part" 2>/dev/null || true); case "$label" in cloudimg-rootfs|ROOT|rootfs|/) ROOT=$part; break ;; esac; done; if [ -z "$ROOT" ]; then ROOT=$(lsblk -nrbo NAME,SIZE,FSTYPE,TYPE -p /dev/vdb 2>/dev/null | awk '$4=="part" && $3!="" && $3!="vfat" {print $2" "$1}' | sort -n | tail -1 | awk '{print $2}'); fi; if [ -z "$ROOT" ]; then ROOT=$(lsblk -nrpo NAME,FSTYPE,TYPE /dev/vdb 2>/dev/null | awk '$3=="part" && $2!="" && $2!="vfat" {print $1}' | tail -1); fi; if [ -z "$ROOT" ] || [ ! -b "$ROOT" ]; then printf '\nVZCTL_BUILDER_RESULT {"ok":false,"phase":"mount","exit":13,"message":"cannot resolve target root on /dev/vdb"}\n' > /dev/hvc0; sync; poweroff; exit 1; fi; printf 'target root %s\n' "$ROOT" > /dev/hvc0"#
}

fn mount_target_script() -> String {
    format!(
        "mkdir -p /mnt/target; {resolve}; mount \"$ROOT\" /mnt/target; mkdir -p /mnt/target/dev /mnt/target/proc /mnt/target/sys; mount --bind /dev /mnt/target/dev; mount -t proc proc /mnt/target/proc; mount -t sysfs sysfs /mnt/target/sys",
        resolve = resolve_target_root_script()
    )
}

fn unmount_target_script() -> String {
    "umount /mnt/target/sys 2>/dev/null || true; umount /mnt/target/proc 2>/dev/null || true; umount /mnt/target/dev 2>/dev/null || true; umount /mnt/target || { printf '\\nVZCTL_BUILDER_RESULT {\"ok\":false,\"phase\":\"umount\",\"exit\":13,\"message\":\"umount failed\"}\\n' > /dev/hvc0; sync; poweroff; exit 1; }; sync"
        .to_string()
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
    // Keep the work root short: macOS AF_UNIX sun_path is ~104 bytes, and the
    // helper nests `{state}/helpers/{FNV64(vm_id)}.console.sock`.
    let token = builder_run_token();
    let work = PathBuf::from(format!("/tmp/vzb-{token}"));
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
    let vm_id = format!("vzb-{token}");
    let mut child = Command::new(&helper)
        .args([
            "run",
            "--vm-id",
            &vm_id,
            "--bundle",
            bundle
                .to_str()
                .ok_or_else(|| BuilderFailure::new(12, "builder bundle path is not UTF-8"))?,
            "--disk",
            disk.to_str()
                .ok_or_else(|| BuilderFailure::new(12, "builder disk path is not UTF-8"))?,
            "--data-disk",
            data_disk
                .to_str()
                .ok_or_else(|| BuilderFailure::new(12, "builder data-disk path is not UTF-8"))?,
            "--cidata",
            cidata
                .to_str()
                .ok_or_else(|| BuilderFailure::new(12, "builder cidata path is not UTF-8"))?,
            "--supervisor-sock",
            state
                .join("missing.sock")
                .to_str()
                .unwrap_or("/tmp/vzb-missing.sock"),
        ])
        .env("VZCTL_STATE_DIR", &state)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| BuilderFailure::new(12, format!("cannot start vz-helper: {error}")))?;

    let result = match wait_for_result(&mut child, &vm_id, options.timeout, options.progress) {
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
            result.message.clone().unwrap_or_else(|| {
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
    progress: bool,
) -> Result<BuilderResult, BuilderFailure> {
    let started = Instant::now();
    let mut serial_path: Option<PathBuf> = None;
    let mut serial_offset: u64 = 0;
    let mut stdout_buf = String::new();

    if let Some(stdout) = child.stdout.as_mut() {
        // Non-blocking-ish: read available after short waits via try_wait loop.
        let _ = stdout;
    }

    loop {
        if started.elapsed() > timeout {
            return Err(BuilderFailure::new(
                13,
                format!(
                    "builder timed out after {}s waiting for result marker",
                    timeout.as_secs()
                ),
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
            if progress {
                emit_builder_serial_progress(path, &mut serial_offset);
            }
            if let Some(result) = find_builder_result(path) {
                return Ok(result);
            }
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(path) = &serial_path {
                    if progress {
                        emit_builder_serial_progress(path, &mut serial_offset);
                    }
                    if let Some(result) = find_builder_result(path) {
                        return Ok(result);
                    }
                }
                let mut stderr_buf = String::new();
                if let Some(stderr) = child.stderr.as_mut() {
                    let _ = stderr.read_to_string(&mut stderr_buf);
                }
                let stderr_tail = stderr_buf.trim();
                let serial_note = serial_path
                    .as_ref()
                    .map(|path| format!(" serial={}", path.display()))
                    .unwrap_or_default();
                return Err(BuilderFailure::new(
                    13,
                    if stderr_tail.is_empty() {
                        format!(
                            "builder helper exited ({status}) without VZCTL_BUILDER_RESULT marker{serial_note}"
                        )
                    } else {
                        format!(
                            "builder helper exited ({status}) without VZCTL_BUILDER_RESULT marker{serial_note}: {stderr_tail}"
                        )
                    },
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

/// Whether a serial line should be mirrored to stderr when progress is on.
fn should_emit_builder_serial_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Result marker becomes the envelope/error; skip noisy duplicates.
    if trimmed.contains(BUILDER_RESULT_PREFIX.trim_end()) {
        return false;
    }
    true
}

/// Read newly completed lines from `path` starting at `offset` (byte position).
/// Incomplete trailing fragments are left unread for the next poll.
fn read_new_serial_lines(path: &Path, offset: &mut u64) -> Vec<String> {
    let Ok(meta) = fs::metadata(path) else {
        return Vec::new();
    };
    if *offset > meta.len() {
        // Truncated / rotated log — restart from the beginning.
        *offset = 0;
    }
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    if file.seek(SeekFrom::Start(*offset)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() || buf.is_empty() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = Vec::new();
    let mut consumed = 0usize;
    for chunk in text.split_inclusive('\n') {
        if !chunk.ends_with('\n') {
            break;
        }
        let clean = chunk.trim_end_matches(['\r', '\n']);
        lines.push(clean.to_string());
        consumed += chunk.len();
    }
    *offset += consumed as u64;
    lines
}

fn emit_builder_serial_progress(path: &Path, offset: &mut u64) {
    for line in read_new_serial_lines(path, offset) {
        if should_emit_builder_serial_line(&line) {
            eprintln!("builder: {line}");
        }
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
    let needle = sanitize_component(vm_id);
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".serial.log") || !name.contains(&needle) {
            continue;
        }
        let path = entry.path();
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map_or(true, |(stamp, _)| modified >= *stamp) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

fn builder_run_token() -> String {
    let mixed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ ((std::process::id() as u64) << 17);
    format!("{:08x}", mixed as u32)
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
    let local = dirs_home().join(".local/bin/vz-helper");
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
            "vzctl-agent.path",
            "vzctl-agent-tmpfiles.conf",
            "vzctl-agent.openrc",
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
            let mode = if name == "vzctl-agent" {
                "0755"
            } else {
                "0644"
            };
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

    let user_data = format!("#cloud-config\nwrite_files:\n{write_files}runcmd:\n{runcmd}\n");
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
        .map_err(|error| BuilderFailure::new(12, format!("cannot start hdiutil: {error}")))?;
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
    fs::copy(source, destination).map(|_| ()).map_err(io_err)
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
    fs::copy(source, destination).map(|_| ()).map_err(io_err)
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
    fn parses_builder_result_after_login_prompt() {
        let result = parse_builder_result_line(
            "vzctl-builder login: VZCTL_BUILDER_RESULT {\"ok\":false,\"phase\":\"bake\",\"exit\":13,\"message\":\"bake failed\"}",
        )
        .unwrap();
        assert!(!result.ok);
        assert_eq!(result.exit, 13);
        assert_eq!(result.phase.as_deref(), Some("bake"));
    }

    #[test]
    fn rejects_malformed_builder_result() {
        assert!(parse_builder_result_line("VZCTL_BUILDER_RESULT not-json").is_none());
        assert!(parse_builder_result_line("other line").is_none());
    }

    #[test]
    fn filters_empty_and_result_marker_serial_lines() {
        assert!(!should_emit_builder_serial_line(""));
        assert!(!should_emit_builder_serial_line("   "));
        assert!(!should_emit_builder_serial_line(
            "VZCTL_BUILDER_RESULT {\"ok\":true,\"phase\":\"done\",\"exit\":0,\"op\":\"bake\"}"
        ));
        assert!(!should_emit_builder_serial_line(
            "vzctl-builder login: VZCTL_BUILDER_RESULT {\"ok\":false,\"phase\":\"bake\",\"exit\":13}"
        ));
        assert!(should_emit_builder_serial_line("target root /dev/vdb1"));
        assert!(should_emit_builder_serial_line(
            "cloud-init: running modules"
        ));
    }

    #[test]
    fn serial_offset_reader_returns_only_new_complete_lines() {
        let dir = std::env::temp_dir().join(format!(
            "vzctl-builder-serial-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        struct Guard(PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _guard = Guard(dir.clone());
        let path = dir.join("serial.log");

        fs::write(&path, "line-one\nline-two\nincompl").unwrap();
        let mut offset = 0u64;
        let first = read_new_serial_lines(&path, &mut offset);
        assert_eq!(first, vec!["line-one", "line-two"]);
        assert_eq!(offset, b"line-one\nline-two\n".len() as u64);

        // Incomplete fragment stays until newline arrives.
        let second = read_new_serial_lines(&path, &mut offset);
        assert!(second.is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        write!(file, "ete\nline-three\n").unwrap();
        drop(file);

        let third = read_new_serial_lines(&path, &mut offset);
        assert_eq!(third, vec!["incomplete", "line-three"]);
    }

    #[test]
    fn runbooks_write_results_to_serial_console() {
        for runbook in [bake_runbook("/mnt/staging"), seal_runbook()] {
            for command in runbook
                .commands
                .iter()
                .filter(|command| command.contains(BUILDER_RESULT_PREFIX))
            {
                assert!(
                    command.contains("> /dev/hvc0"),
                    "result marker is not written to the serial console: {command}"
                );
            }
        }
    }

    #[test]
    fn runbooks_are_valid_shell() {
        for (name, runbook) in [
            ("bake", bake_runbook("/mnt/staging")),
            ("seal", seal_runbook()),
        ] {
            assert_eq!(runbook.commands.len(), 1, "{name}");
            let script = &runbook.commands[0];
            assert!(
                !script.contains("\\ for"),
                "{name} must not contain broken line continuations"
            );
            let status = std::process::Command::new("sh")
                .args(["-n", "-c", script])
                .status()
                .expect("spawn sh");
            assert!(status.success(), "{name} runbook failed sh -n: {script}");
        }
    }

    #[test]
    fn runbooks_mount_target_disk_directly() {
        let bake = bake_runbook("/mnt/staging");
        assert!(
            bake.commands
                .iter()
                .any(|c| c.contains("mount") && c.contains("/mnt/target")),
            "bake runbook should mount the target root directly"
        );
        assert!(
            bake.commands.iter().any(|c| c.contains("/dev/vdb")),
            "bake must resolve root on the data disk /dev/vdb only"
        );
        assert!(
            !bake.commands.iter().any(|c| c.contains("findfs")),
            "bake must not use findfs across all disks"
        );
        assert!(
            !bake.commands.iter().any(|c| c.contains("virt-customize")),
            "bake runbook must not nest virt-customize under Apple VZ"
        );
        assert_eq!(
            bake.commands.len(),
            1,
            "bake must be a single runcmd to avoid false-success markers"
        );
        let seal = seal_runbook();
        assert!(
            seal.commands
                .iter()
                .any(|c| c.contains("mount") && c.contains("/mnt/target")),
            "seal runbook should mount the target root directly"
        );
        assert!(
            seal.commands.iter().any(|c| c.contains("/dev/vdb")),
            "seal must resolve root on the data disk /dev/vdb only"
        );
        assert!(
            !seal.commands.iter().any(|c| c.contains("virt-customize")),
            "seal runbook must not nest virt-customize under Apple VZ"
        );
        assert_eq!(
            seal.commands.len(),
            1,
            "seal must be a single runcmd to avoid false-success markers"
        );
        assert!(
            bake.commands
                .iter()
                .any(|c| c.contains("vzctl-agent.openrc")),
            "bake must install OpenRC unit for Alpine"
        );
    }
}
