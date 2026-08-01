//! virtiofs mount helpers shared by `vm create`, `vm mount`, and reconciler.

use crate::config::{valid_volume_name, VIRTIOFS_DEVICE_TAG};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// VirtioFS share name for the hypernetwork project dir on docker-role VMs.
pub(crate) const DOCKER_PROJECT_MOUNT_TAG: &str = "project";

/// Resolved host→guest mount persisted in `vm.json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ResolvedMount {
    pub(crate) name: String,
    pub(crate) source: PathBuf,
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) read_only: bool,
}

impl ResolvedMount {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "source": self.source,
            "target": self.target,
            "read_only": self.read_only,
        })
    }
}

pub(crate) fn parse_mount_flag(raw: &str) -> Result<ResolvedMount, String> {
    let mut name = None;
    let mut source = None;
    let mut target = None;
    let mut read_only = false;
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part == "ro" || part == "read_only" || part == "readOnly" {
            read_only = true;
            continue;
        }
        let Some((key, value)) = part.split_once('=') else {
            return Err(format!(
                "invalid --mount fragment {part:?}; expected key=value"
            ));
        };
        match key {
            "tag" | "name" => name = Some(value.to_string()),
            "source" => source = Some(PathBuf::from(value)),
            "target" => target = Some(value.to_string()),
            "ro" | "read_only" | "readOnly" => {
                read_only = matches!(value, "1" | "true" | "yes" | "ro");
            }
            other => {
                return Err(format!("unknown --mount key {other:?}"));
            }
        }
    }
    let source = source.ok_or_else(|| " --mount requires source=…".to_string())?;
    let target = target.ok_or_else(|| "--mount requires target=…".to_string())?;
    let name = match name {
        Some(value) => value,
        None => default_mount_name(&source)?,
    };
    validate_resolved_mount(&name, &source, &target)?;
    let source = if source.is_absolute() {
        source
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot resolve relative mount source: {error}"))?
            .join(source)
    };
    if !source.is_dir() {
        return Err(format!(
            "mount source {:?} is not an existing directory",
            source.display()
        ));
    }
    Ok(ResolvedMount {
        name,
        source,
        target,
        read_only,
    })
}

pub(crate) fn default_mount_name(source: &Path) -> Result<String, String> {
    let base = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "cannot derive mount name from source path".to_string())?;
    let sanitized: String = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if !valid_volume_name(&sanitized) {
        return Err(format!(
            "derived mount name {sanitized:?} is invalid; pass tag=… explicitly"
        ));
    }
    Ok(sanitized)
}

pub(crate) fn validate_resolved_mount(
    name: &str,
    source: &Path,
    target: &str,
) -> Result<(), String> {
    if name == VIRTIOFS_DEVICE_TAG {
        return Err(format!(
            "mount name {VIRTIOFS_DEVICE_TAG:?} is reserved for the virtiofs device tag"
        ));
    }
    if !valid_volume_name(name) {
        return Err(format!(
            "mount name {name:?} must be 1-36 chars [A-Za-z0-9][A-Za-z0-9_-]*"
        ));
    }
    if source.as_os_str().is_empty() {
        return Err("mount source must not be empty".to_string());
    }
    if !target.starts_with('/') || target.len() < 2 {
        return Err("mount target must be an absolute path (not /)".to_string());
    }
    Ok(())
}

/// Absolute project directory (config parent) for 1:1 docker host binds.
pub(crate) fn resolve_project_dir(config_path: &Path) -> Result<PathBuf, String> {
    let config_file = if config_path.is_dir() {
        config_path.join("hypernetwork.config.yaml")
    } else {
        config_path.to_path_buf()
    };
    let dir = config_file
        .parent()
        .ok_or_else(|| format!("cannot resolve project dir for {}", config_file.display()))?;
    fs::canonicalize(dir).map_err(|error| {
        format!(
            "cannot canonicalize project dir {}: {error}",
            dir.display()
        )
    })
}

/// `--mount` flag: share project dir into the guest at the same absolute path.
pub(crate) fn docker_project_mount_flag(project_dir: &Path) -> Result<String, String> {
    let abs = if project_dir.is_absolute()
        && fs::metadata(project_dir)
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
    {
        fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf())
    } else {
        fs::canonicalize(project_dir).map_err(|error| {
            format!(
                "cannot canonicalize project dir {}: {error}",
                project_dir.display()
            )
        })?
    };
    let target = abs.to_string_lossy();
    if !target.starts_with('/') || target.len() < 2 {
        return Err(format!(
            "project dir {} is not a usable absolute mount target",
            abs.display()
        ));
    }
    validate_resolved_mount(DOCKER_PROJECT_MOUNT_TAG, &abs, &target)?;
    Ok(format!(
        "tag={DOCKER_PROJECT_MOUNT_TAG},source={},target={target}",
        abs.display()
    ))
}

pub(crate) fn read_manifest_mounts(bundle: &Path) -> Result<Vec<ResolvedMount>, String> {
    let path = bundle.join("vm.json");
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid VM manifest {}: {error}", path.display()))?;
    let Some(mounts) = value.get("mounts") else {
        return Ok(Vec::new());
    };
    serde_json::from_value(mounts.clone())
        .map_err(|error| format!("invalid mounts in {}: {error}", path.display()))
}

pub(crate) fn write_manifest_mounts(bundle: &Path, mounts: &[ResolvedMount]) -> Result<(), String> {
    let path = bundle.join("vm.json");
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid VM manifest {}: {error}", path.display()))?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| format!("VM manifest {} is not an object", path.display()))?;
    root.insert(
        "mounts".to_string(),
        Value::Array(mounts.iter().map(ResolvedMount::to_json).collect()),
    );
    let pretty = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("cannot serialize mounts: {error}"))?;
    fs::write(&path, format!("{pretty}\n"))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mount_flag_requires_source_and_target() {
        let mount = parse_mount_flag("tag=web-src,source=/tmp,target=/srv/app").unwrap();
        assert_eq!(mount.name, "web-src");
        assert_eq!(mount.target, "/srv/app");
        assert!(!mount.read_only);
    }

    #[test]
    fn parse_mount_flag_accepts_ro() {
        let mount = parse_mount_flag("source=/tmp,target=/srv/app,ro").unwrap();
        assert!(mount.read_only);
        assert_eq!(mount.name, "tmp");
    }

    #[test]
    fn rejects_reserved_device_tag() {
        let error = parse_mount_flag("tag=vzctl,source=/tmp,target=/srv/app").unwrap_err();
        assert!(error.contains("reserved"));
    }

    #[test]
    fn docker_project_mount_flag_is_same_path() {
        let dir = std::env::temp_dir().join(format!(
            "vzctl-docker-project-mount-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let flag = docker_project_mount_flag(&dir).unwrap();
        let abs = fs::canonicalize(&dir).unwrap();
        let expected = format!(
            "tag={DOCKER_PROJECT_MOUNT_TAG},source={},target={}",
            abs.display(),
            abs.display()
        );
        assert_eq!(flag, expected);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_project_dir_from_config_file() {
        let dir = std::env::temp_dir().join(format!(
            "vzctl-docker-project-dir-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("hypernetwork.config.yaml");
        fs::write(&config, "x: 1\n").unwrap();
        let resolved = resolve_project_dir(&config).unwrap();
        assert_eq!(resolved, fs::canonicalize(&dir).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }
}
