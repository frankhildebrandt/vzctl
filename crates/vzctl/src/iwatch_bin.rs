use crate::guest_utils::GuestUtilsError;
use serde_json::Value;
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

/// True when the host must supply or resolve an iwatch binary (env pin).
pub fn iwatch_fetch_required() -> bool {
    configured_iwatch_version().is_some() || configured_iwatch_bin().is_some()
}

fn configured_iwatch_bin() -> Option<PathBuf> {
    std::env::var("VZCTL_IWATCH_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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
    fetch_latest_iwatch_tag_api().or_else(|api_err| {
        fetch_latest_iwatch_tag_redirect().map_err(|redirect_err| {
            GuestUtilsError::new(format!(
                "no published iwatch release on github.com/{IWATCH_REPO} (api: {api_err}; redirect: {redirect_err}); publish a release or set VZCTL_IWATCH_VERSION / VZCTL_IWATCH_BIN"
            ))
        })
    })
}

fn fetch_latest_iwatch_tag_api() -> Result<String, GuestUtilsError> {
    let url = format!("https://api.github.com/repos/{IWATCH_REPO}/releases/latest");
    let output = Command::new("curl")
        .args(["-fsSL", "-H", "Accept: application/vnd.github+json", &url])
        .output()
        .map_err(|error| GuestUtilsError::new(format!("curl {url}: {error}")))?;
    if !output.status.success() {
        return Err(GuestUtilsError::new(format!(
            "HTTP {} from GitHub API",
            output.status.code().unwrap_or(-1)
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| GuestUtilsError::new(format!("invalid GitHub releases JSON: {error}")))?;
    let tag = value
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && !tag.contains('/'));
    tag.map(|tag| tag.to_string())
        .ok_or_else(|| GuestUtilsError::new("GitHub releases/latest JSON has no tag_name"))
}

fn fetch_latest_iwatch_tag_redirect() -> Result<String, GuestUtilsError> {
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
    let effective = String::from_utf8_lossy(&output.stdout);
    parse_latest_tag(effective.trim())
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
    (format!("{base}/{archive}"), format!("{base}/checksums.txt"))
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
    if let Some(path) = configured_iwatch_bin() {
        if path.is_file() {
            fs::copy(&path, dest)
                .map_err(|error| GuestUtilsError::new(format!("copy VZCTL_IWATCH_BIN: {error}")))?;
            set_executable(dest)?;
            return sha256_path(dest);
        }
        return Err(GuestUtilsError::new(format!(
            "VZCTL_IWATCH_BIN is not a file: {}",
            path.display()
        )));
    }

    stage_release(&iwatch_version_string()?, dest, &mut download_file)
}

fn stage_release(
    version: &str,
    dest: &Path,
    fetch: &mut impl FnMut(&str, &str, &Path) -> Result<(), GuestUtilsError>,
) -> Result<String, GuestUtilsError> {
    let archive_name = iwatch_archive_name(&version);
    let work = dest.parent().unwrap_or(dest).join("iwatch-download");
    fs::create_dir_all(&work)
        .map_err(|error| GuestUtilsError::new(format!("iwatch work dir: {error}")))?;
    let checksums_path = work.join("checksums.txt");
    let archive_path = work.join(&archive_name);
    download_release_asset(version, "checksums.txt", &checksums_path, fetch)?;
    let checksums = fs::read_to_string(&checksums_path)
        .map_err(|error| GuestUtilsError::new(format!("read checksums.txt: {error}")))?;
    let expected = parse_checksums(&checksums, &archive_name)?;
    download_release_asset(version, &archive_name, &archive_path, fetch)?;
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

fn download_release_asset(
    version: &str,
    name: &str,
    dest: &Path,
    fetch: &mut impl FnMut(&str, &str, &Path) -> Result<(), GuestUtilsError>,
) -> Result<(), GuestUtilsError> {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    if tag.len() < 2
        || !tag
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
    {
        return Err(GuestUtilsError::new("invalid iwatch release tag"));
    }
    let (archive_url, checksums_url) = iwatch_download_urls(&tag);
    let browser = if name == "checksums.txt" {
        checksums_url
    } else if name == iwatch_archive_name(&tag) {
        archive_url
    } else {
        return Err(GuestUtilsError::new("unexpected iwatch release asset"));
    };
    if fetch(&browser, "application/octet-stream", dest).is_ok() {
        return Ok(());
    }
    // Resolve this exact release and asset; never use latest or a supplied URL.
    let metadata_url = format!("https://api.github.com/repos/{IWATCH_REPO}/releases/tags/{tag}");
    let metadata_path = dest.with_extension("release.json");
    fetch(&metadata_url, "application/vnd.github+json", &metadata_path)?;
    let bytes = fs::read(&metadata_path)
        .map_err(|error| GuestUtilsError::new(format!("read iwatch release metadata: {error}")))?;
    let _ = fs::remove_file(&metadata_path);
    let release: Value = serde_json::from_slice(&bytes).map_err(|error| {
        GuestUtilsError::new(format!("invalid iwatch release metadata: {error}"))
    })?;
    if release["tag_name"].as_str() != Some(&tag) || release["draft"].as_bool() != Some(false) {
        return Err(GuestUtilsError::new(
            "iwatch release metadata tag/draft mismatch",
        ));
    }
    let assets: Vec<_> = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| asset["name"].as_str() == Some(name))
        .collect();
    if assets.len() != 1 || assets[0]["state"].as_str() != Some("uploaded") {
        return Err(GuestUtilsError::new(format!(
            "iwatch release has no unique uploaded asset {name}"
        )));
    }
    let id = assets[0]["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| GuestUtilsError::new("invalid iwatch release asset id"))?;
    let api = format!("https://api.github.com/repos/{IWATCH_REPO}/releases/assets/{id}");
    fetch(&api, "application/octet-stream", dest)
}

fn download_file(url: &str, accept: &str, dest: &Path) -> Result<(), GuestUtilsError> {
    let status = Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "120",
            "-H",
        ])
        .arg(format!("Accept: {accept}"))
        .arg("-o")
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
    let binary = find_named(&unpacked, "iwatch")
        .ok_or_else(|| GuestUtilsError::new("iwatch archive does not contain an iwatch binary"))?;
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
            parse_latest_tag("https://github.com/frankhildebrandt/iwatch/releases/tag/v0.2.0\n")
                .unwrap(),
            "v0.2.0"
        );
        assert_eq!(
            parse_latest_tag("https://github.com/frankhildebrandt/iwatch/releases/tag/v1.0.0/")
                .unwrap(),
            "v1.0.0"
        );
        assert!(
            parse_latest_tag("https://github.com/frankhildebrandt/iwatch/releases/latest").is_err()
        );
        assert!(parse_latest_tag("https://github.com/frankhildebrandt/iwatch/releases").is_err());
    }

    #[test]
    fn fetch_latest_tag_api_parses_tag_name() {
        let body = r#"{"tag_name":"v0.3.1","name":"v0.3.1"}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        let tag = value["tag_name"].as_str().unwrap();
        assert_eq!(tag, "v0.3.1");
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
        assert_eq!(
            sha,
            "abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1"
        );
    }

    #[test]
    fn parse_checksums_missing_archive() {
        let err = parse_checksums(
            "deadbeef  other.tar.gz\n",
            "iwatch_1.2.3_linux_arm64.tar.gz",
        )
        .unwrap_err();
        assert!(err.message.contains("no entry"));
    }

    fn test_dir(label: &str) -> PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vzctl-iwatch-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn release_metadata(name: &str) -> Value {
        serde_json::json!({"tag_name":"v0.3.0", "draft":false, "assets":[{
            "id":42,"name":name,"state":"uploaded", "url":"https://untrusted.invalid/ignored"
        }]})
    }

    #[test]
    fn browser_success_never_uses_api() {
        let dir = test_dir("direct");
        let mut calls = 0;
        download_release_asset("0.3.0", "checksums.txt", &dir.join("checksums"), &mut |url, accept, path| {
            calls += 1;
            assert_eq!(url, "https://github.com/frankhildebrandt/iwatch/releases/download/v0.3.0/checksums.txt");
            assert_eq!(accept,"application/octet-stream");
            fs::write(path,"ok").unwrap(); Ok(())
        }).unwrap();
        assert_eq!(calls, 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fallback_is_pinned_to_official_asset_id() {
        let dir = test_dir("fallback");
        let mut calls = Vec::new();
        download_release_asset(
            "v0.3.0",
            "checksums.txt",
            &dir.join("checksums"),
            &mut |url, accept, path| {
                calls.push((url.to_owned(), accept.to_owned()));
                match calls.len() {
                    1 => Err(GuestUtilsError::new("HTTP 404")),
                    2 => {
                        fs::write(path, release_metadata("checksums.txt").to_string()).unwrap();
                        Ok(())
                    }
                    3 => {
                        fs::write(path, "checksum body").unwrap();
                        Ok(())
                    }
                    _ => panic!("unexpected request"),
                }
            },
        )
        .unwrap();
        assert_eq!(
            calls[1],
            (
                "https://api.github.com/repos/frankhildebrandt/iwatch/releases/tags/v0.3.0".into(),
                "application/vnd.github+json".into()
            )
        );
        assert_eq!(
            calls[2],
            (
                "https://api.github.com/repos/frankhildebrandt/iwatch/releases/assets/42".into(),
                "application/octet-stream".into()
            )
        );
        assert_eq!(
            fs::read_to_string(dir.join("checksums")).unwrap(),
            "checksum body"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fallback_rejects_wrong_release_missing_duplicate_or_invalid_asset() {
        let dir = test_dir("metadata");
        let good = release_metadata("checksums.txt");
        let mut cases = Vec::new();
        let mut value = good.clone();
        value["tag_name"] = "v9.9.9".into();
        cases.push(value);
        let mut value = good.clone();
        value["draft"] = true.into();
        cases.push(value);
        let mut value = good.clone();
        value["assets"] = serde_json::json!([]);
        cases.push(value);
        let mut value = good.clone();
        value["assets"]
            .as_array_mut()
            .unwrap()
            .push(good["assets"][0].clone());
        cases.push(value);
        let mut value = good.clone();
        value["assets"][0]["id"] = (-1).into();
        cases.push(value);
        let mut value = good.clone();
        value["assets"][0]["state"] = "new".into();
        cases.push(value);
        for value in cases {
            let mut calls = 0;
            assert!(download_release_asset(
                "v0.3.0",
                "checksums.txt",
                &dir.join("checksums"),
                &mut |_, _, path| {
                    calls += 1;
                    if calls == 1 {
                        return Err(GuestUtilsError::new("404"));
                    }
                    assert_eq!(calls, 2);
                    fs::write(path, value.to_string()).unwrap();
                    Ok(())
                }
            )
            .is_err());
        }
        assert!(download_release_asset(
            "../bad",
            "checksums.txt",
            &dir.join("checksums"),
            &mut |_, _, _| panic!("invalid tag must not fetch")
        )
        .is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_or_invalid_checksum_and_api_failure_leave_destination_untouched() {
        let dir = test_dir("failclosed");
        let dest = dir.join("iwatch");
        fs::write(&dest, b"existing binary").unwrap();
        for body in ["", "not-a-sha  iwatch_0.3.0_linux_arm64.tar.gz", "{}"] {
            let mut calls = 0;
            let result = stage_release("v0.3.0", &dest, &mut |url, _, path| {
                calls += 1;
                assert!(url.ends_with("/checksums.txt"));
                fs::write(path, body).unwrap();
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(calls, 1);
            assert_eq!(fs::read(&dest).unwrap(), b"existing binary");
        }
        let mut calls = 0;
        assert!(stage_release("v0.3.0", &dest, &mut |_, _, _| {
            calls += 1;
            Err(GuestUtilsError::new("HTTP unavailable"))
        })
        .is_err());
        assert_eq!(calls, 2);
        assert_eq!(fs::read(&dest).unwrap(), b"existing binary");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fallback_archive_still_requires_matching_sha256_before_extraction() {
        let dir = test_dir("hash");
        let source = dir.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("iwatch"), b"verified binary").unwrap();
        let archive = dir.join("fixture.tar.gz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg("iwatch")
            .status()
            .unwrap()
            .success());
        let archive_bytes = fs::read(&archive).unwrap();
        let digest = sha256_path(&archive).unwrap();
        for valid in [false, true] {
            let dest = dir.join("iwatch");
            fs::write(&dest, b"existing binary").unwrap();
            let result = stage_release("v0.3.0", &dest, &mut |url, accept, path| {
                if url.starts_with("https://github.com/") {
                    return Err(GuestUtilsError::new("404"));
                }
                if accept == "application/vnd.github+json" {
                    let mut value = release_metadata("checksums.txt");
                    value["assets"].as_array_mut().unwrap().push(serde_json::json!({"id":43,"name":"iwatch_0.3.0_linux_arm64.tar.gz","state":"uploaded"}));
                    fs::write(path, value.to_string()).unwrap();
                } else if url.ends_with("/42") {
                    let sha = if valid {
                        digest.clone()
                    } else {
                        "0".repeat(64)
                    };
                    fs::write(path, format!("{sha}  iwatch_0.3.0_linux_arm64.tar.gz\n")).unwrap();
                } else {
                    assert!(url.ends_with("/43"));
                    fs::write(path, &archive_bytes).unwrap();
                }
                Ok(())
            });
            if valid {
                assert!(result.is_ok());
                assert_eq!(fs::read(&dest).unwrap(), b"verified binary");
            } else {
                assert!(result.unwrap_err().message.contains("sha256 mismatch"));
                assert_eq!(fs::read(&dest).unwrap(), b"existing binary");
            }
        }
        fs::remove_dir_all(dir).unwrap();
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
