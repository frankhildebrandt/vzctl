use serde_json::{json, Value as JsonValue};
use serde_yaml::Value as YamlValue;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_RUNTIME: u8 = 24;
const DOCKER_USER: &str = "vzctl";

pub(crate) fn context_name(project: &str) -> String {
    format!("vzctl-{project}")
}

pub(crate) fn ssh_hostname(project: &str) -> String {
    format!("docker.svc.{project}.vz.test")
}

pub(crate) fn project_docker_dir(state_dir: &Path, project: &str) -> PathBuf {
    state_dir.join("projects").join(project).join("docker")
}

pub(crate) fn ensure_ssh_keypair(
    state_dir: &Path,
    project: &str,
) -> Result<(PathBuf, String), String> {
    let directory = project_docker_dir(state_dir, project);
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "cannot create docker SSH directory {}: {error}",
            directory.display()
        )
    })?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
        format!(
            "cannot protect docker SSH directory {}: {error}",
            directory.display()
        )
    })?;

    let private_key = directory.join("id_ed25519");
    let public_key = directory.join("id_ed25519.pub");
    if !private_key.exists() {
        let status = Command::new("ssh-keygen")
            .args([
                "-t",
                "ed25519",
                "-N",
                "",
                "-C",
                &format!("vzctl-docker@{project}"),
                "-f",
            ])
            .arg(&private_key)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .map_err(|error| format!("ssh-keygen failed: {error}"))?;
        if !status.success() {
            return Err(format!("ssh-keygen exited with {status}"));
        }
    }
    fs::set_permissions(&private_key, fs::Permissions::from_mode(0o600)).ok();
    let mut pubkey = String::new();
    File::open(&public_key)
        .and_then(|mut file| file.read_to_string(&mut pubkey))
        .map_err(|error| format!("cannot read {}: {error}", public_key.display()))?;
    let pubkey = pubkey.trim().to_string();
    if pubkey.is_empty() {
        return Err(format!("empty public key at {}", public_key.display()));
    }
    write_ssh_config(state_dir, project, &private_key)?;
    Ok((private_key, pubkey))
}

fn write_ssh_config(state_dir: &Path, project: &str, private_key: &Path) -> Result<(), String> {
    let directory = project_docker_dir(state_dir, project);
    let config_path = directory.join("ssh_config");
    let host = ssh_hostname(project);
    let body = format!(
        "Host {host}\n  User {DOCKER_USER}\n  IdentityFile {}\n  IdentitiesOnly yes\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile {}/known_hosts\n",
        private_key.display(),
        directory.display()
    );
    let mut file = File::create(&config_path)
        .map_err(|error| format!("cannot write {}: {error}", config_path.display()))?;
    file.write_all(body.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", config_path.display()))?;
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).ok();
    Ok(())
}

pub(crate) fn ssh_config_path(state_dir: &Path, project: &str) -> PathBuf {
    project_docker_dir(state_dir, project).join("ssh_config")
}

pub(crate) fn docker_role_cloud_config(
    pubkey: &str,
    include_engine: bool,
    docker_bip: Option<&str>,
) -> YamlValue {
    let mut map = serde_yaml::Mapping::new();
    let mut users = Vec::new();
    let mut user = serde_yaml::Mapping::new();
    user.insert(
        YamlValue::String("name".into()),
        YamlValue::String(DOCKER_USER.into()),
    );
    user.insert(
        YamlValue::String("groups".into()),
        YamlValue::Sequence(vec![
            YamlValue::String("sudo".into()),
            YamlValue::String("docker".into()),
        ]),
    );
    user.insert(
        YamlValue::String("shell".into()),
        YamlValue::String("/bin/bash".into()),
    );
    user.insert(
        YamlValue::String("sudo".into()),
        YamlValue::String("ALL=(ALL) NOPASSWD:ALL".into()),
    );
    user.insert(
        YamlValue::String("ssh_authorized_keys".into()),
        YamlValue::Sequence(vec![YamlValue::String(pubkey.into())]),
    );
    users.push(YamlValue::Mapping(user));
    map.insert(
        YamlValue::String("users".into()),
        YamlValue::Sequence(users),
    );

    if include_engine {
        map.insert(
            YamlValue::String("package_update".into()),
            YamlValue::Bool(true),
        );
        map.insert(
            YamlValue::String("packages".into()),
            YamlValue::Sequence(vec![YamlValue::String("docker.io".into())]),
        );
        let mut runcmd = vec![
            YamlValue::Sequence(vec![
                YamlValue::String("systemctl".into()),
                YamlValue::String("enable".into()),
                YamlValue::String("--now".into()),
                YamlValue::String("docker".into()),
            ]),
            YamlValue::Sequence(vec![
                YamlValue::String("sh".into()),
                YamlValue::String("-c".into()),
                YamlValue::String(
                    "if ! findmnt /var/lib/docker >/dev/null 2>&1; then \
                     DEV=$(lsblk -ndo NAME,TYPE | awk '$2==\"disk\"{print $1}' | tail -n1); \
                     if [ -n \"$DEV\" ] && [ ! -b /dev/${DEV}1 ]; then \
                     parted -s /dev/$DEV mklabel gpt mkpart primary ext4 1MiB 100% && \
                     mkfs.ext4 -F /dev/${DEV}1 && mkdir -p /var/lib/docker && \
                     echo \"/dev/${DEV}1 /var/lib/docker ext4 defaults,nofail 0 2\" >> /etc/fstab && \
                     mount /var/lib/docker; fi; fi"
                        .into(),
                ),
            ]),
        ];
        if docker_bip.is_some() {
            // daemon.json is written via write_files; restart after first boot packages.
            runcmd.push(YamlValue::Sequence(vec![
                YamlValue::String("systemctl".into()),
                YamlValue::String("restart".into()),
                YamlValue::String("docker".into()),
            ]));
        }
        map.insert(
            YamlValue::String("runcmd".into()),
            YamlValue::Sequence(runcmd),
        );
    }

    if let Some(bip) = docker_bip {
        map.insert(
            YamlValue::String("write_files".into()),
            YamlValue::Sequence(vec![docker_daemon_json_write_file(bip)]),
        );
    }

    YamlValue::Mapping(map)
}

fn docker_daemon_json_write_file(bip: &str) -> YamlValue {
    let content = format!(
        "{{\n  \"bip\": \"{bip}\",\n  \"iptables\": false\n}}\n"
    );
    let mut file = serde_yaml::Mapping::new();
    file.insert(
        YamlValue::String("path".into()),
        YamlValue::String("/etc/docker/daemon.json".into()),
    );
    file.insert(
        YamlValue::String("owner".into()),
        YamlValue::String("root:root".into()),
    );
    file.insert(
        YamlValue::String("permissions".into()),
        YamlValue::String("0644".into()),
    );
    file.insert(
        YamlValue::String("content".into()),
        YamlValue::String(content),
    );
    YamlValue::Mapping(file)
}

/// Ensure `/etc/docker/daemon.json` carries the hypernetwork docker bip.
pub(crate) fn ensure_docker_daemon_bip(mut config: YamlValue, bip: &str) -> YamlValue {
    let YamlValue::Mapping(ref mut map) = config else {
        return config;
    };
    let key = YamlValue::String("write_files".into());
    let mut files = match map.remove(&key) {
        Some(YamlValue::Sequence(items)) => items,
        _ => Vec::new(),
    };
    let path = "/etc/docker/daemon.json";
    files.retain(|item| {
        item.as_mapping()
            .and_then(|mapping| mapping.get(YamlValue::String("path".into())))
            .and_then(YamlValue::as_str)
            != Some(path)
    });
    files.push(docker_daemon_json_write_file(bip));
    map.insert(key, YamlValue::Sequence(files));
    config
}

/// Deep-merge cloud-config YAML. System scalars win; sequences are concatenated
/// (system first); mappings are recursively merged.
pub(crate) fn merge_cloud_config(system: YamlValue, user: Option<YamlValue>) -> YamlValue {
    let Some(user) = user else {
        return system;
    };
    merge_yaml(system, user)
}

fn merge_yaml(system: YamlValue, user: YamlValue) -> YamlValue {
    match (system, user) {
        (YamlValue::Mapping(mut system_map), YamlValue::Mapping(user_map)) => {
            for (key, user_value) in user_map {
                match system_map.remove(&key) {
                    Some(system_value) => {
                        system_map.insert(key, merge_yaml(system_value, user_value));
                    }
                    None => {
                        system_map.insert(key, user_value);
                    }
                }
            }
            YamlValue::Mapping(system_map)
        }
        (YamlValue::Sequence(mut system_seq), YamlValue::Sequence(user_seq)) => {
            system_seq.extend(user_seq);
            YamlValue::Sequence(system_seq)
        }
        (system_value, _) => system_value,
    }
}

pub(crate) fn load_user_cloud_init(path: &Path) -> Result<YamlValue, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read cloud-init {}: {error}", path.display()))?;
    let trimmed = source
        .strip_prefix("#cloud-config")
        .unwrap_or(&source)
        .trim_start_matches(|c: char| c == '\n' || c == '\r');
    serde_yaml::from_str(trimmed)
        .map_err(|error| format!("invalid cloud-init YAML {}: {error}", path.display()))
}

pub(crate) fn render_user_data(config: &YamlValue) -> Result<String, String> {
    let body =
        serde_yaml::to_string(config).map_err(|error| format!("cloud-init render: {error}"))?;
    let body = body.strip_prefix("---\n").unwrap_or(&body).to_string();
    Ok(format!("#cloud-config\n{body}"))
}

pub(crate) fn ensure_context(
    project: &str,
    state_dir: &Path,
    host: Option<&str>,
) -> Result<String, String> {
    let name = context_name(project);
    let (private_key, _) = ensure_ssh_keypair(state_dir, project)?;
    let hostname = host.unwrap_or(&ssh_hostname(project)).to_string();
    let docker_host = format!("ssh://{DOCKER_USER}@{hostname}");
    let ssh_config = ssh_config_path(state_dir, project);
    let ssh_command = format!(
        "ssh -F {} -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
        ssh_config.display(),
        private_key.display()
    );

    let existing = Command::new("docker")
        .args(["context", "ls", "--format", "{{.Name}}"])
        .output();
    match existing {
        Ok(output) if output.status.success() => {
            let names = String::from_utf8_lossy(&output.stdout);
            if names.lines().any(|line| line.trim() == name) {
                let status = Command::new("docker")
                    .args(["context", "update", &name, "--docker"])
                    .arg(format!("host={docker_host}"))
                    .env("DOCKER_SSH_COMMAND", &ssh_command)
                    .status()
                    .map_err(|error| format!("docker context update failed: {error}"))?;
                if !status.success() {
                    // Older docker may lack update; recreate.
                    let _ = Command::new("docker")
                        .args(["context", "rm", "-f", &name])
                        .status();
                } else {
                    return Ok(name);
                }
            }
        }
        Ok(_) | Err(_) => {}
    }

    let status = Command::new("docker")
        .args(["context", "create", &name, "--docker"])
        .arg(format!("host={docker_host}"))
        .env("DOCKER_SSH_COMMAND", &ssh_command)
        .status()
        .map_err(|error| format!("docker context create failed: {error}"))?;
    if !status.success() {
        return Err(format!("docker context create exited with {status}"));
    }
    let _ = private_key;
    Ok(name)
}

pub(crate) fn remove_context(project: &str) -> Result<(), String> {
    let name = context_name(project);
    let status = Command::new("docker")
        .args(["context", "rm", "-f", &name])
        .status()
        .map_err(|error| format!("docker context rm failed: {error}"))?;
    if !status.success() {
        // Missing context is fine on purge.
        return Ok(());
    }
    Ok(())
}

pub(crate) fn context_ping(project: &str) -> Result<(), String> {
    let name = context_name(project);
    let output = Command::new("docker")
        .args(["--context", &name, "info"])
        .output()
        .map_err(|error| format!("docker info failed: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "docker context {name} ping failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub(crate) fn docker_binary_available() -> bool {
    Command::new("docker")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn command(
    args: impl Iterator<Item = String>,
    state_dir: &Path,
    socket_path: &Path,
) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("-h")
        || args.first().map(String::as_str) == Some("--help")
    {
        eprintln!("usage: vzctl docker [--project P] [--] <docker-args...>");
        return ExitCode::from(EXIT_USAGE);
    }

    let mut project = None;
    let mut passthrough = Vec::new();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => {
                project = Some(args.next().unwrap_or_default());
                if project.as_deref().unwrap_or("").is_empty() {
                    eprintln!("--project requires a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            }
            "--" => {
                passthrough.extend(args);
                break;
            }
            other if other.starts_with('-') && passthrough.is_empty() && project.is_none() => {
                // Allow docker flags after optional project; treat as passthrough.
                passthrough.push(arg);
                passthrough.extend(args);
                break;
            }
            _ => {
                passthrough.push(arg);
                passthrough.extend(args);
                break;
            }
        }
    }

    if passthrough.is_empty() {
        eprintln!("usage: vzctl docker [--project P] [--] <docker-args...>");
        return ExitCode::from(EXIT_USAGE);
    }

    let project = match project.or_else(|| infer_project(socket_path)) {
        Some(project) => project,
        None => {
            eprintln!("cannot determine project; pass --project");
            return ExitCode::from(EXIT_INVALID);
        }
    };

    let name = match ensure_context(&project, state_dir, None) {
        Ok(name) => name,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };

    let private_key = project_docker_dir(state_dir, &project).join("id_ed25519");
    let ssh_config = ssh_config_path(state_dir, &project);
    let ssh_command = format!(
        "ssh -F {} -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
        ssh_config.display(),
        private_key.display()
    );

    let status = Command::new("docker")
        .arg("--context")
        .arg(&name)
        .args(&passthrough)
        .env("DOCKER_SSH_COMMAND", ssh_command)
        .status();
    match status {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!("docker failed: {error}");
            ExitCode::from(EXIT_RUNTIME)
        }
    }
}

fn infer_project(socket_path: &Path) -> Option<String> {
    let response = rpc(socket_path, "net.list", json!({})).ok()?;
    response["attachments"]
        .as_array()?
        .iter()
        .find_map(|attachment| {
            let labels = attachment.get("labels")?.as_object()?;
            if labels.get("managed-by")?.as_str()? == "vzctl" {
                attachment.get("project")?.as_str().map(str::to_string)
            } else {
                None
            }
        })
}

fn rpc(socket_path: &Path, method: &str, params: JsonValue) -> Result<JsonValue, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let mut stream =
        UnixStream::connect(socket_path).map_err(|error| format!("supervisor connect: {error}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    writeln!(stream, "{request}").map_err(|error| format!("supervisor write: {error}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| format!("supervisor read: {error}"))?;
    let response: JsonValue =
        serde_json::from_str(&line).map_err(|error| format!("supervisor decode: {error}"))?;
    if let Some(error) = response.get("error") {
        return Err(error.to_string());
    }
    Ok(response.get("result").cloned().unwrap_or(JsonValue::Null))
}

#[derive(Debug)]
pub(crate) struct DoctorDockerCheck {
    pub(crate) id: &'static str,
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) details: JsonValue,
}

pub(crate) fn doctor_check(state_dir: &Path) -> DoctorDockerCheck {
    if !docker_binary_available() {
        return DoctorDockerCheck {
            id: "docker.cli",
            ok: false,
            message: "docker CLI not found on PATH".into(),
            details: json!({ "available": false }),
        };
    }
    let projects = state_dir.join("projects");
    let Ok(entries) = fs::read_dir(&projects) else {
        return DoctorDockerCheck {
            id: "docker.context",
            ok: true,
            message: "no managed docker contexts".into(),
            details: json!({ "contexts": [] }),
        };
    };
    let mut contexts = Vec::new();
    let mut failed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let project = name.to_string_lossy();
        let docker_dir = entry.path().join("docker");
        if !docker_dir.join("id_ed25519").exists() {
            continue;
        }
        let context = context_name(&project);
        contexts.push(context.clone());
        if let Err(error) = context_ping(&project) {
            failed.push(json!({ "context": context, "error": error }));
        }
    }
    if failed.is_empty() {
        DoctorDockerCheck {
            id: "docker.context",
            ok: true,
            message: if contexts.is_empty() {
                "no managed docker contexts".into()
            } else {
                format!("{} docker context(s) reachable", contexts.len())
            },
            details: json!({ "contexts": contexts }),
        }
    } else {
        DoctorDockerCheck {
            id: "docker.context",
            ok: false,
            message: format!("{} docker context(s) unreachable", failed.len()),
            details: json!({ "contexts": contexts, "failures": failed }),
        }
    }
}

#[allow(dead_code)]
pub(crate) fn envelope(command: &str, status: &str, exit_code: u8, summary: JsonValue) -> String {
    serde_json::to_string_pretty(&json!({
        "apiVersion": API_VERSION,
        "command": command,
        "status": status,
        "exit_code": exit_code,
        "summary": summary,
    }))
    .expect("envelope serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_and_host_names() {
        assert_eq!(context_name("edge-dmz"), "vzctl-edge-dmz");
        assert_eq!(ssh_hostname("edge-dmz"), "docker.svc.edge-dmz.vz.test");
    }

    #[test]
    fn merge_appends_packages_and_keeps_system_hostname() {
        let system = serde_yaml::from_str::<YamlValue>(
            r#"
hostname: docker
users: [{name: vzctl}]
packages: [ca-certificates]
"#,
        )
        .unwrap();
        let user = serde_yaml::from_str::<YamlValue>(
            r#"
hostname: override
packages: [docker.io]
runcmd: [[systemctl, enable, --now, docker]]
"#,
        )
        .unwrap();
        let merged = merge_cloud_config(system, Some(user));
        let map = merged.as_mapping().unwrap();
        assert_eq!(
            map.get(YamlValue::String("hostname".into()))
                .and_then(YamlValue::as_str),
            Some("docker")
        );
        let packages = map
            .get(YamlValue::String("packages".into()))
            .and_then(YamlValue::as_sequence)
            .unwrap();
        assert_eq!(packages.len(), 2);
    }
}
