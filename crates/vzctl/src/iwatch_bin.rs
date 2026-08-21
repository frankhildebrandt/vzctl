use crate::guest_utils::GuestUtilsError;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IWATCH_REPO: &str = "frankhildebrandt/iwatch";
pub const IWATCH_GUEST_PATH: &str = "/usr/local/bin/iwatch";

/// Release tag to bundle: `VZCTL_IWATCH_VERSION` pin, otherwise GitHub `latest`.
pub fn iwatch_version_string() -> Result<String, GuestUtilsError> {
    if let Some(version) = configured_iwatch_version() {
        if !version.eq_ignore_ascii_case("latest") {
            return Ok(version);
        }
    }
    fetch_latest_iwatch_tag()
}

fn configured_iwatch_version() -> Option<String> {
    std::env::var("VZCTL_IWATCH_VERSION")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parse the tag from a GitHub `/releases/latest` redirect target.
pub fn parse_latest_tag(url: &str) -> Result<String, GuestUtilsError> {
    let url = url.trim().trim_end_matches('/');
    let Some(tag) = url.rsplit_once("/tag/").map(|(_, tag)| tag) else {
        return Err(GuestUtilsError::new(format!(
            "cannot parse iwatch latest tag from {url}"
        )));
    };
    if tag.is_empty() || tag.contains('/') {
        return Err(GuestUtilsError::new(format!(
            "cannot parse iwatch latest tag from {url}"
        )));
    }
    Ok(tag.to_string())
}

fn fetch_latest_iwatch_tag() -> Result<String, GuestUtilsError> {
    let url = format!("https://github.com/{IWATCH_REPO}/releases/latest");
    let output = Command::new("curl")
        .args(["-fsSL", "-o", "/dev/null", "-w", "%{url_effective}", &url])
        .output()
        .map_err(|error| GuestUtilsError::new(format!("curl {url}: {error}")))?;
    if !output.status.success() {
        return Err(GuestUtilsError::new(format!(
            "cannot resolve latest iwatch release from {url}"
        )));
    }
    parse_latest_tag(&String::from_utf8_lossy(&output.stdout))
}

/// GoReleaser archive filename for the linux/arm64 guest binary.
pub fn iwatch_archive_name(version: &str) -> String {
    let semver = version.strip_prefix('v').unwrap_or(version);
    format!("iwatch_{semver}_linux_arm64.tar.gz")
}

/// GitHub release URLs for the archive and checksums.txt.
pub fn iwatch_download_urls(version: &str) -> (String, String) {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let archive = iwatch_archive_name(&tag);
    let base = format!("https://github.com/{IWATCH_REPO}/releases/download/{tag}");
    (
        format!("{base}/{archive}"),
        format!("{base}/checksums.txt"),
    )
}

/// Extract the SHA256 for `archive` from a GoReleaser checksums.txt body.
pub fn parse_checksums(text: &str, archive: &str) -> Result<String, GuestUtilsError> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(sha) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        let name = name.rsplit('/').next().unwrap_or(name);
        if name == archive {
            if sha.len() != 64 || !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(GuestUtilsError::new("invalid iwatch checksum line"));
            }
            return Ok(sha.to_ascii_lowercase());
        }
    }
    Err(GuestUtilsError::new(format!(
        "checksums.txt has no entry for {archive}"
    )))
}

/// Place a linux/arm64 `iwatch` binary at `dest`, returning its SHA256 hex.
pub fn stage_iwatch_binary(dest: &Path) -> Result<String, GuestUtilsError> {
    if let Ok(path) = std::env::var("VZCTL_IWATCH_BIN") {
        let path = PathBuf::from(path.trim());
        if path.is_file() {
            fs::copy(&path, dest).map_err(|error| {
                GuestUtilsError::new(format!("copy VZCTL_IWATCH_BIN: {error}"))
            })?;
            set_executable(dest)?;
            return sha256_path(dest);
        }
        return Err(GuestUtilsError::new(format!(
            "VZCTL_IWATCH_BIN is not a file: {}",
            path.display()
        )));
    }

    let version = iwatch_version_string()?;
    let (archive_url, checksums_url) = iwatch_download_urls(&version);
    let archive_name = iwatch_archive_name(&version);
    let work = dest.parent().unwrap_or(dest).join("iwatch-download");
    fs::create_dir_all(&work)
        .map_err(|error| GuestUtilsError::new(format!("iwatch work dir: {error}")))?;
    let checksums_path = work.join("checksums.txt");
    let archive_path = work.join(&archive_name);
    download_file(&checksums_url, &checksums_path)?;
    let checksums = fs::read_to_string(&checksums_path)
        .map_err(|error| GuestUtilsError::new(format!("read checksums.txt: {error}")))?;
    let expected = parse_checksums(&checksums, &archive_name)?;
    download_file(&archive_url, &archive_path)?;
    let actual = sha256_path(&archive_path)?;
    if actual != expected {
        return Err(GuestUtilsError::new(format!(
            "iwatch archive sha256 mismatch: expected {expected} got {actual}"
        )));
    }
    extract_iwatch(&archive_path, &work, dest)?;
    set_executable(dest)?;
    let sha = sha256_path(dest)?;
    let _ = fs::remove_dir_all(&work);
    Ok(sha)
}

fn download_file(url: &str, dest: &Path) -> Result<(), GuestUtilsError> {
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|error| GuestUtilsError::new(format!("curl {url}: {error}")))?;
    if !status.success() {
        return Err(GuestUtilsError::new(format!(
            "cannot download iwatch from {url}"
        )));
    }
    Ok(())
}

fn extract_iwatch(archive: &Path, work: &Path, dest: &Path) -> Result<(), GuestUtilsError> {
    let unpacked = work.join("unpacked");
    fs::create_dir_all(&unpacked)
        .map_err(|error| GuestUtilsError::new(format!("iwatch unpack dir: {error}")))?;
    let status = Command::new("tar")
        .args(["-xzf"])
        .arg(archive)
        .arg("-C")
        .arg(&unpacked)
        .status()
        .map_err(|error| GuestUtilsError::new(format!("tar: {error}")))?;
    if !status.success() {
        return Err(GuestUtilsError::new("failed to extract iwatch archive"));
    }
    let binary = find_named(&unpacked, "iwatch").ok_or_else(|| {
        GuestUtilsError::new("iwatch archive does not contain an iwatch binary")
    })?;
    fs::copy(&binary, dest)
        .map_err(|error| GuestUtilsError::new(format!("install iwatch: {error}")))?;
    Ok(())
}

fn find_named(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|value| value.to_str()) == Some(name) {
                return Some(path);
            }
        }
    }
    None
}

fn set_executable(path: &Path) -> Result<(), GuestUtilsError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|error| GuestUtilsError::new(error.to_string()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|error| GuestUtilsError::new(error.to_string()))?;
    }
    Ok(())
}

fn sha256_path(path: &Path) -> Result<String, GuestUtilsError> {
    let bytes = fs::read(path)
        .map_err(|error| GuestUtilsError::new(format!("read {}: {error}", path.display())))?;
    Ok(encode_hex(Sha256::digest(bytes)))
}

fn encode_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_name_strips_v_prefix() {
        assert_eq!(
            iwatch_archive_name("v1.2.3"),
            "iwatch_1.2.3_linux_arm64.tar.gz"
        );
        assert_eq!(
            iwatch_archive_name("1.2.3"),
            "iwatch_1.2.3_linux_arm64.tar.gz"
        );
    }

    #[test]
    fn parse_latest_tag_from_github_redirect() {
        assert_eq!(
            parse_latest_tag(
                "https://github.com/frankhildebrandt/iwatch/releases/tag/v0.2.0\n"
            )
            .unwrap(),
            "v0.2.0"
        );
        assert_eq!(
            parse_latest_tag("https://github.com/frankhildebrandt/iwatch/releases/tag/v1.0.0/")
                .unwrap(),
            "v1.0.0"
        );
        assert!(parse_latest_tag(
            "https://github.com/frankhildebrandt/iwatch/releases/latest"
        )
        .is_err());
    }

    #[test]
    fn version_string_uses_env_pin() {
        std::env::set_var("VZCTL_IWATCH_VERSION", "v9.9.9");
        let version = iwatch_version_string().unwrap();
        std::env::remove_var("VZCTL_IWATCH_VERSION");
        assert_eq!(version, "v9.9.9");
    }

    #[test]
    fn download_urls_pin_linux_arm64() {
        let (archive, checksums) = iwatch_download_urls("v1.2.3");
        assert_eq!(
            archive,
            "https://github.com/frankhildebrandt/iwatch/releases/download/v1.2.3/iwatch_1.2.3_linux_arm64.tar.gz"
        );
        assert_eq!(
            checksums,
            "https://github.com/frankhildebrandt/iwatch/releases/download/v1.2.3/checksums.txt"
        );
    }

    #[test]
    fn parse_checksums_reads_goreleaser_fixture() {
        let text = "\
abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1  iwatch_1.2.3_linux_arm64.tar.gz
dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  iwatch_1.2.3_darwin_arm64.tar.gz
";
        let sha = parse_checksums(text, "iwatch_1.2.3_linux_arm64.tar.gz").unwrap();
        assert_eq!(sha, "abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1");
    }

    #[test]
    fn parse_checksums_missing_archive() {
        let err = parse_checksums("deadbeef  other.tar.gz\n", "iwatch_1.2.3_linux_arm64.tar.gz")
            .unwrap_err();
        assert!(err.message.contains("no entry"));
    }

    #[test]
    fn stage_copies_override_binary() {
        let dir = std::env::temp_dir().join(format!("vzctl-iwatch-stage-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src-iwatch");
        let dest = dir.join("iwatch");
        fs::write(&src, b"iwatch-bin").unwrap();
        std::env::set_var("VZCTL_IWATCH_BIN", &src);
        let sha = stage_iwatch_binary(&dest).unwrap();
        std::env::remove_var("VZCTL_IWATCH_BIN");
        assert_eq!(fs::read(&dest).unwrap(), b"iwatch-bin");
        assert_eq!(sha.len(), 64);
        let _ = fs::remove_dir_all(&dir);
    }
}
