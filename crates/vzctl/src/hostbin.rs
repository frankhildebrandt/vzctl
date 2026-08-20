//! Resolve host helper binaries when LaunchAgent PATH is minimal.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Locate `name` on PATH plus known macOS tool prefixes (Homebrew, Multipass).
///
/// `qemu-img` prefers the vendored bundle (`libexec/qemu-img`) over PATH.
pub fn resolve(name: &str) -> Option<PathBuf> {
    if name == "qemu-img" {
        return resolve_qemu_img();
    }
    search_dirs()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

pub fn resolve_qemu_img() -> Option<PathBuf> {
    if let Some(path) = env::var_os("VZCTL_QEMU_IMG").map(PathBuf::from) {
        if is_executable(&path) {
            return Some(path);
        }
    }
    qemu_img_candidates()
        .into_iter()
        .find(|path| is_executable(path))
}

pub fn qemu_img_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let state = state_dir();
    paths.push(state.join("libexec/qemu-img/qemu-img"));
    paths.push(state.join("bin/qemu-img"));
    paths.push(PathBuf::from("/usr/local/libexec/vzctl/qemu-img/qemu-img"));
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../daemon/Vendor/qemu-img/qemu-img"),
    );
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("qemu-img"));
            paths.push(dir.join("../libexec/qemu-img/qemu-img"));
        }
    }
    for dir in search_dirs() {
        paths.push(dir.join("qemu-img"));
    }
    paths
}

pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
    }
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/Library/Application Support/com.canonical.multipass/bin"),
    ]);
    dirs
}

fn state_dir() -> PathBuf {
    if let Some(directory) = env::var_os("VZCTL_STATE_DIR") {
        return PathBuf::from(directory);
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Application Support/vzctl")
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_dirs_include_homebrew_and_multipass() {
        let dirs = search_dirs();
        assert!(dirs.iter().any(|dir| dir == Path::new("/opt/homebrew/bin")));
        assert!(dirs.iter().any(|dir| {
            dir == Path::new("/Library/Application Support/com.canonical.multipass/bin")
        }));
    }

    #[test]
    fn qemu_img_candidates_prefer_vendored_libexec() {
        let paths = qemu_img_candidates();
        assert!(paths
            .iter()
            .any(|path| path.ends_with("libexec/qemu-img/qemu-img")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("daemon/Vendor/qemu-img/qemu-img")));
        let libexec = paths
            .iter()
            .position(|path| path.ends_with("libexec/qemu-img/qemu-img"))
            .unwrap();
        let path_fallback = paths
            .iter()
            .position(|path| path.ends_with("com.canonical.multipass/bin/qemu-img"));
        if let Some(fallback) = path_fallback {
            assert!(libexec < fallback);
        }
    }

    #[test]
    fn resolve_finds_a_system_binary() {
        let path = resolve("true").or_else(|| resolve("echo"));
        assert!(path.is_some(), "expected /usr/bin/true or echo");
        assert!(path.unwrap().is_file());
    }
}
