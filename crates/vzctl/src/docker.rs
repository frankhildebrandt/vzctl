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

/// Resolve the host `docker` CLI. LaunchAgent-spawned apply jobs often inherit a
/// minimal PATH without Homebrew (`/opt/homebrew/bin`).
fn resolve_docker_bin() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join("docker"));
        }
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/docker"),
        PathBuf::from("/usr/local/bin/docker"),
    ]);
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("docker CLI not found (install Docker or ensure /opt/homebrew/bin is on PATH)".into())
}

fn docker_command() -> Result<Command, String> {
    Ok(Command::new(resolve_docker_bin()?))
}

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
    // Quote paths: Application Support contains spaces (OpenSSH otherwise errors).
    let body = format!(
        "Host {host}\n  User {DOCKER_USER}\n  IdentityFile \"{}\"\n  IdentitiesOnly yes\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile \"{}/known_hosts\"\n",
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

fn ensure_user_ssh_include(ssh_config: &Path) -> Result<(), String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let ssh_dir = PathBuf::from(home).join(".ssh");
    fs::create_dir_all(&ssh_dir)
        .map_err(|error| format!("cannot create {}: {error}", ssh_dir.display()))?;
    let user_config = ssh_dir.join("config");
    let include = format!("Include \"{}\"", ssh_config.display());
    let existing = fs::read_to_string(&user_config).unwrap_or_default();
    if existing.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == include || trimmed.contains(&ssh_config.display().to_string())
    }) {
        return Ok(());
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&user_config)
        .map_err(|error| format!("cannot update {}: {error}", user_config.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")
            .map_err(|error| format!("cannot update {}: {error}", user_config.display()))?;
    }
    writeln!(file, "\n# vzctl docker context\n{include}")
        .map_err(|error| format!("cannot update {}: {error}", user_config.display()))?;
    let _ = fs::set_permissions(&user_config, fs::Permissions::from_mode(0o600));
    Ok(())
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

    // `vm exec` runs as vzctl-agent; docker.sock is root:docker mode 660.
    // Restart the agent so the long-lived process picks up supplementary groups.
    let agent_docker_group = YamlValue::Sequence(vec![
        YamlValue::String("sh".into()),
        YamlValue::String("-c".into()),
        YamlValue::String(
            "getent group docker >/dev/null && usermod -aG docker vzctl-agent \
             && (systemctl try-restart vzctl-agent || true) || true"
                .into(),
        ),
    ]);

    if include_engine {
        map.insert(
            YamlValue::String("package_update".into()),
            YamlValue::Bool(true),
        );
        map.insert(
            YamlValue::String("packages".into()),
            YamlValue::Sequence(vec![YamlValue::String("docker.io".into())]),
        );
        let bootcmd = vec![YamlValue::String(
            r#"(
  set -e
  if ! findmnt /var/lib/docker >/dev/null 2>&1; then
    ROOT_DISK=$(findmnt -n -o SOURCE / | sed 's/[0-9]*$//;s#^/dev/##')
    DEV=$(lsblk -ndo NAME,TYPE,RO,SIZE | awk -v root="$ROOT_DISK" '
      $2=="disk" && $3==0 && $1!=root { print $1, $4 }' | sort -k2 -h | tail -n1 | awk '{print $1}')
    if [ -n "$DEV" ] && [ -b "/dev/$DEV" ] && [ ! -b "/dev/${DEV}1" ]; then
      parted -s "/dev/$DEV" mklabel gpt mkpart primary ext4 1MiB 100%
      mkfs.ext4 -F "/dev/${DEV}1"
    fi
    if [ -n "$DEV" ] && [ -b "/dev/${DEV}1" ]; then
      mkdir -p /var/lib/docker
      grep -q ' /var/lib/docker ' /etc/fstab || \
        echo "/dev/${DEV}1 /var/lib/docker ext4 defaults,nofail 0 2" >> /etc/fstab
      mount /var/lib/docker || true
    fi
  fi
  mkdir -p /var/lib/docker/apt-cache /var/lib/docker/apt-lists
  printf 'Dir::Cache::Archives "/var/lib/docker/apt-cache";\n' > /etc/apt/apt.conf.d/00vzctl-cache
  printf 'Dir::State::Lists "/var/lib/docker/apt-lists";\n' > /etc/apt/apt.conf.d/00vzctl-lists
) || true
"#
            .into(),
        )];
        let mut runcmd = vec![
            YamlValue::Sequence(vec![
                YamlValue::String("systemctl".into()),
                YamlValue::String("enable".into()),
                YamlValue::String("--now".into()),
                YamlValue::String("docker".into()),
            ]),
            agent_docker_group.clone(),
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
            YamlValue::String("bootcmd".into()),
            YamlValue::Sequence(bootcmd),
        );
        map.insert(
            YamlValue::String("runcmd".into()),
            YamlValue::Sequence(runcmd),
        );
    } else {
        map.insert(
            YamlValue::String("runcmd".into()),
            YamlValue::Sequence(vec![agent_docker_group]),
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
    let content = format!("{{\n  \"bip\": \"{bip}\",\n  \"iptables\": false\n}}\n");
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
    // Recreated VMs rotate SSH host keys; drop stale pins so accept-new can relearn.
    let project_dir = project_docker_dir(state_dir, project);
    let _ = fs::remove_file(project_dir.join("known_hosts"));
    let _ = Command::new("ssh-keygen")
        .args(["-R", &hostname])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // Docker Desktop often ignores DOCKER_SSH_COMMAND; Include makes IdentityFile visible.
    ensure_user_ssh_include(&ssh_config)?;
    let ssh_command = format!(
        "ssh -F \"{}\" -i \"{}\" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
        ssh_config.display(),
        private_key.display()
    );

    let docker = resolve_docker_bin()?;
    let existing = Command::new(&docker)
        .args(["context", "ls", "--format", "{{.Name}}"])
        .output();
    match existing {
        Ok(output) if output.status.success() => {
            let names = String::from_utf8_lossy(&output.stdout);
            if names.lines().any(|line| line.trim() == name) {
                let status = Command::new(&docker)
                    .args(["context", "update", &name, "--docker"])
                    .arg(format!("host={docker_host}"))
                    .env("DOCKER_SSH_COMMAND", &ssh_command)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(|error| format!("docker context update failed: {error}"))?;
                if !status.success() {
                    // Older docker may lack update; recreate.
                    let _ = Command::new(&docker)
                        .args(["context", "rm", "-f", &name])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                } else {
                    return Ok(name);
                }
            }
        }
        Ok(_) | Err(_) => {}
    }

    let status = Command::new(&docker)
        .args(["context", "create", &name, "--docker"])
        .arg(format!("host={docker_host}"))
        .env("DOCKER_SSH_COMMAND", &ssh_command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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
    let Ok(docker) = resolve_docker_bin() else {
        return Ok(());
    };
    let status = Command::new(&docker)
        .args(["context", "rm", "-f", &name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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
    let output = docker_command()?
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
    let Ok(docker) = resolve_docker_bin() else {
        return false;
    };
    Command::new(docker)
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

const STRUCTURED_VERBS: &[&str] = &["ps", "inspect", "start", "stop", "restart", "run"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DockerOp {
    Passthrough(Vec<String>),
    Ps {
        all: bool,
    },
    Inspect {
        id: String,
    },
    Start {
        id: String,
    },
    Stop {
        id: String,
    },
    Restart {
        id: String,
    },
    Run {
        image: String,
        name: Option<String>,
        env: Vec<String>,
        ports: Vec<String>,
        cmd: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedDockerArgs {
    project: Option<String>,
    format: OutputFormat,
    op: DockerOp,
}

fn parse_docker_args(args: Vec<String>) -> Result<ParsedDockerArgs, String> {
    // Pull global flags from anywhere before an explicit `--` passthrough boundary so
    // UI-appended `--format json` / `--project P` work after the verb.
    let (format, args) = extract_format_flag(args)?;
    let (project, args) = extract_project_flag(args)?;

    let mut rest = Vec::new();
    let mut iter = args.into_iter().peekable();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                return Err("help".into());
            }
            "--" => {
                rest.extend(iter);
                return Ok(ParsedDockerArgs {
                    project,
                    format,
                    op: DockerOp::Passthrough(rest),
                });
            }
            other if other.starts_with('-') && rest.is_empty() => {
                // Global-looking flags already handled; remaining dash args before a
                // verb are passthrough (legacy `vzctl docker -a` / compose flags).
                rest.push(arg);
                rest.extend(iter);
                return Ok(ParsedDockerArgs {
                    project,
                    format,
                    op: DockerOp::Passthrough(rest),
                });
            }
            _ => {
                rest.push(arg);
                rest.extend(iter);
                break;
            }
        }
    }

    if rest.is_empty() {
        return Err("usage".into());
    }

    let verb = rest[0].as_str();
    if !STRUCTURED_VERBS.contains(&verb) {
        return Ok(ParsedDockerArgs {
            project,
            format,
            op: DockerOp::Passthrough(rest),
        });
    }

    let op = parse_structured_op(verb, &rest[1..])?;
    Ok(ParsedDockerArgs {
        project,
        format,
        op,
    })
}

fn extract_format_flag(args: Vec<String>) -> Result<(OutputFormat, Vec<String>), String> {
    let mut format = OutputFormat::Human;
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            out.push(arg);
            out.extend(iter);
            break;
        }
        if arg == "--format" {
            let value = iter.next().unwrap_or_default();
            format = match value.as_str() {
                "json" => OutputFormat::Json,
                "human" => OutputFormat::Human,
                "" => return Err("--format requires human or json".into()),
                other => return Err(format!("unsupported --format: {other}")),
            };
            continue;
        }
        out.push(arg);
    }
    Ok((format, out))
}

fn extract_project_flag(args: Vec<String>) -> Result<(Option<String>, Vec<String>), String> {
    let mut project = None;
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            out.push(arg);
            out.extend(iter);
            break;
        }
        if arg == "--project" {
            let value = iter.next().unwrap_or_default();
            if value.is_empty() {
                return Err("--project requires a value".into());
            }
            project = Some(value);
            continue;
        }
        out.push(arg);
    }
    Ok((project, out))
}

fn parse_structured_op(verb: &str, args: &[String]) -> Result<DockerOp, String> {
    match verb {
        "ps" => {
            let mut all = false;
            let mut iter = args.iter();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "-a" | "--all" => all = true,
                    "--format" => {
                        let _ = iter.next();
                    }
                    other => return Err(format!("unknown docker ps option: {other}")),
                }
            }
            Ok(DockerOp::Ps { all })
        }
        "inspect" => {
            let mut id = None;
            let mut iter = args.iter();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--format" => {
                        let _ = iter.next();
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown docker inspect option: {other}"));
                    }
                    other if id.is_none() => id = Some(other.to_string()),
                    other => return Err(format!("unexpected argument: {other}")),
                }
            }
            let id = id.ok_or_else(|| "docker inspect requires a container id".to_string())?;
            Ok(DockerOp::Inspect { id })
        }
        "start" | "stop" | "restart" => {
            let mut id = None;
            let mut iter = args.iter();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--format" => {
                        let _ = iter.next();
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown docker {verb} option: {other}"));
                    }
                    other if id.is_none() => id = Some(other.to_string()),
                    other => return Err(format!("unexpected argument: {other}")),
                }
            }
            let id = id.ok_or_else(|| format!("docker {verb} requires a container id"))?;
            Ok(match verb {
                "start" => DockerOp::Start { id },
                "stop" => DockerOp::Stop { id },
                _ => DockerOp::Restart { id },
            })
        }
        "run" => {
            let mut image = None;
            let mut name = None;
            let mut env = Vec::new();
            let mut ports = Vec::new();
            let mut cmd = Vec::new();
            let mut iter = args.iter().peekable();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--" => {
                        cmd.extend(iter.cloned());
                        break;
                    }
                    "--format" => {
                        let _ = iter.next();
                    }
                    "--image" => {
                        let value = iter
                            .next()
                            .cloned()
                            .ok_or_else(|| "--image requires a value".to_string())?;
                        image = Some(value);
                    }
                    "--name" => {
                        let value = iter
                            .next()
                            .cloned()
                            .ok_or_else(|| "--name requires a value".to_string())?;
                        name = Some(value);
                    }
                    "-e" | "--env" => {
                        let value = iter
                            .next()
                            .cloned()
                            .ok_or_else(|| format!("{arg} requires KEY=VALUE"))?;
                        env.push(value);
                    }
                    "-p" | "--publish" => {
                        let value = iter
                            .next()
                            .cloned()
                            .ok_or_else(|| format!("{arg} requires host:guest"))?;
                        ports.push(value);
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown docker run option: {other}"));
                    }
                    other if image.is_none() => {
                        // Allow positional image for convenience.
                        image = Some(other.to_string());
                    }
                    other => {
                        cmd.push(other.to_string());
                        cmd.extend(iter.cloned());
                        break;
                    }
                }
            }
            let image = image.ok_or_else(|| "docker run requires --image".to_string())?;
            Ok(DockerOp::Run {
                image,
                name,
                env,
                ports,
                cmd,
            })
        }
        other => Err(format!("unknown docker verb: {other}")),
    }
}

pub(crate) fn command(
    args: impl Iterator<Item = String>,
    state_dir: &Path,
    socket_path: &Path,
) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let parsed = match parse_docker_args(args) {
        Ok(parsed) => parsed,
        Err(error) if error == "help" || error == "usage" => {
            eprintln!(
                "usage: vzctl docker [--project P] [--format human|json] <ps|inspect|start|stop|restart|run> ...\n\
                 \x20      vzctl docker [--project P] [--] <docker-args...>"
            );
            return ExitCode::from(EXIT_USAGE);
        }
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let project = match parsed
        .project
        .clone()
        .or_else(|| infer_project(socket_path))
    {
        Some(project) => project,
        None => {
            eprintln!("cannot determine project; pass --project");
            return ExitCode::from(EXIT_INVALID);
        }
    };

    let context = match ensure_context(&project, state_dir, None) {
        Ok(name) => name,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };

    let private_key = project_docker_dir(state_dir, &project).join("id_ed25519");
    let ssh_config = ssh_config_path(state_dir, &project);
    let ssh_command = format!(
        "ssh -F \"{}\" -i \"{}\" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
        ssh_config.display(),
        private_key.display()
    );

    match parsed.op {
        DockerOp::Passthrough(passthrough) => {
            if passthrough.is_empty() {
                eprintln!("usage: vzctl docker [--project P] [--] <docker-args...>");
                return ExitCode::from(EXIT_USAGE);
            }
            let status = match docker_command() {
                Ok(mut cmd) => cmd
                    .arg("--context")
                    .arg(&context)
                    .args(&passthrough)
                    .env("DOCKER_SSH_COMMAND", ssh_command)
                    .status(),
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            };
            match status {
                Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
                Err(error) => {
                    eprintln!("docker failed: {error}");
                    ExitCode::from(EXIT_RUNTIME)
                }
            }
        }
        op => run_structured_op(&op, &project, &context, &ssh_command, parsed.format),
    }
}

fn docker_output(
    context: &str,
    ssh_command: &str,
    args: &[String],
) -> Result<(u8, String, String), String> {
    let output = docker_command()?
        .arg("--context")
        .arg(context)
        .args(args)
        .env("DOCKER_SSH_COMMAND", ssh_command)
        .output()
        .map_err(|error| format!("docker failed: {error}"))?;
    let code = output.status.code().unwrap_or(1) as u8;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((code, stdout, stderr))
}

fn parse_ndjson_containers(stdout: &str) -> Vec<JsonValue> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<JsonValue>(line).ok())
        .map(normalize_container_row)
        .collect()
}

fn normalize_container_row(raw: JsonValue) -> JsonValue {
    let id = raw
        .get("ID")
        .or_else(|| raw.get("Id"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let names = raw
        .get("Names")
        .or_else(|| raw.get("Name"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let image = raw.get("Image").cloned().unwrap_or(JsonValue::Null);
    let status = raw.get("Status").cloned().unwrap_or(JsonValue::Null);
    let state = raw.get("State").cloned().unwrap_or(JsonValue::Null);
    let ports = raw.get("Ports").cloned().unwrap_or(JsonValue::Null);
    let command = raw.get("Command").cloned().unwrap_or(JsonValue::Null);
    let ip = container_ip_from_raw(&raw);
    json!({
        "id": id,
        "names": names,
        "image": image,
        "status": status,
        "state": state,
        "ports": ports,
        "command": command,
        "ip": ip,
        "raw": raw,
    })
}

fn container_ip_from_raw(raw: &JsonValue) -> JsonValue {
    if let Some(ip) = raw.get("IPAddress").and_then(|v| v.as_str()) {
        if !ip.is_empty() {
            return JsonValue::String(ip.to_string());
        }
    }
    if let Some(networks) = raw
        .pointer("/NetworkSettings/Networks")
        .and_then(|v| v.as_object())
    {
        let ips: Vec<String> = networks
            .values()
            .filter_map(|net| net.get("IPAddress").and_then(|v| v.as_str()))
            .filter(|ip| !ip.is_empty())
            .map(str::to_string)
            .collect();
        if !ips.is_empty() {
            return JsonValue::String(ips.join(", "));
        }
    }
    JsonValue::String(String::new())
}

fn enrich_containers_with_ips(
    mut containers: Vec<JsonValue>,
    context: &str,
    ssh_command: &str,
) -> Vec<JsonValue> {
    let ids: Vec<String> = containers
        .iter()
        .filter_map(|c| c.get("id").and_then(|v| v.as_str()))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        return containers;
    }
    let mut args = vec!["inspect".to_string()];
    args.extend(ids.iter().cloned());
    let Ok((0, stdout, _)) = docker_output(context, ssh_command, &args) else {
        return containers;
    };
    let Ok(inspected) = serde_json::from_str::<JsonValue>(&stdout) else {
        return containers;
    };
    let Some(list) = inspected.as_array() else {
        return containers;
    };
    let mut by_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for item in list {
        let Some(id) = item.get("Id").and_then(|v| v.as_str()) else {
            continue;
        };
        let ip = container_ip_from_raw(item);
        if let Some(ip) = ip.as_str() {
            if !ip.is_empty() {
                by_id.insert(id.to_string(), ip.to_string());
                if id.len() > 12 {
                    by_id.insert(id[..12].to_string(), ip.to_string());
                }
            }
        }
    }
    for container in &mut containers {
        let Some(id) = container.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let ip = by_id
            .get(id)
            .or_else(|| {
                if id.len() >= 12 {
                    by_id.get(&id[..12])
                } else {
                    None
                }
            })
            .cloned()
            .unwrap_or_default();
        if let Some(obj) = container.as_object_mut() {
            obj.insert("ip".into(), JsonValue::String(ip));
        }
    }
    containers
}

fn print_envelope_or_human(
    format: OutputFormat,
    command: &str,
    exit_code: u8,
    summary: JsonValue,
    human: impl FnOnce(),
) -> ExitCode {
    let status = if exit_code == 0 { "ok" } else { "fail" };
    let body = envelope(command, status, exit_code, summary);
    match format {
        OutputFormat::Json => {
            println!("{body}");
        }
        OutputFormat::Human => human(),
    }
    ExitCode::from(exit_code)
}

fn run_structured_op(
    op: &DockerOp,
    project: &str,
    context: &str,
    ssh_command: &str,
    format: OutputFormat,
) -> ExitCode {
    match op {
        DockerOp::Passthrough(_) => unreachable!("passthrough handled by caller"),
        DockerOp::Ps { all } => {
            let mut args = vec!["ps".to_string()];
            if *all {
                args.push("--all".into());
            }
            args.push("--format".into());
            args.push("{{json .}}".into());
            let (code, stdout, stderr) = match docker_output(context, ssh_command, &args) {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            };
            if code != 0 {
                let summary = json!({
                    "message": stderr.trim(),
                    "project": project,
                });
                return print_envelope_or_human(format, "docker.ps", code, summary, || {
                    eprint!("{stderr}");
                });
            }
            let containers =
                enrich_containers_with_ips(parse_ndjson_containers(&stdout), context, ssh_command);
            let summary = json!({
                "message": format!("{} container(s)", containers.len()),
                "project": project,
                "containers": containers,
            });
            print_envelope_or_human(format, "docker.ps", 0, summary, || {
                if containers.is_empty() {
                    println!("no containers");
                    return;
                }
                println!(
                    "{:<14} {:<20} {:<24} {:<16} {}",
                    "ID", "NAMES", "IMAGE", "IP", "STATUS"
                );
                for c in &containers {
                    let id = c["id"].as_str().unwrap_or("?");
                    let short = if id.len() > 12 { &id[..12] } else { id };
                    let ip = c["ip"].as_str().unwrap_or("-");
                    let ip = if ip.is_empty() { "-" } else { ip };
                    println!(
                        "{:<14} {:<20} {:<24} {:<16} {}",
                        short,
                        c["names"].as_str().unwrap_or("-"),
                        c["image"].as_str().unwrap_or("-"),
                        ip,
                        c["status"].as_str().unwrap_or("-"),
                    );
                }
            })
        }
        DockerOp::Inspect { id } => {
            let args = vec!["inspect".to_string(), id.clone()];
            let (code, stdout, stderr) = match docker_output(context, ssh_command, &args) {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            };
            if code != 0 {
                let summary = json!({
                    "message": stderr.trim(),
                    "project": project,
                    "id": id,
                });
                return print_envelope_or_human(format, "docker.inspect", code, summary, || {
                    eprint!("{stderr}");
                });
            }
            let parsed: JsonValue = match serde_json::from_str(stdout.trim()) {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("docker inspect decode failed: {error}");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            };
            let inspect = match parsed {
                JsonValue::Array(mut items) if !items.is_empty() => items.remove(0),
                other => other,
            };
            let summary = json!({
                "message": format!("inspected {id}"),
                "project": project,
                "id": id,
                "inspect": inspect,
            });
            print_envelope_or_human(format, "docker.inspect", 0, summary, || {
                println!("{stdout}");
            })
        }
        DockerOp::Start { id } | DockerOp::Stop { id } | DockerOp::Restart { id } => {
            let action = match op {
                DockerOp::Start { .. } => "start",
                DockerOp::Stop { .. } => "stop",
                DockerOp::Restart { .. } => "restart",
                _ => unreachable!(),
            };
            let command_name = format!("docker.{action}");
            let args = vec![action.to_string(), id.clone()];
            let (code, stdout, stderr) = match docker_output(context, ssh_command, &args) {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            };
            let message = if code == 0 {
                format!("{action}ed {id}")
            } else {
                stderr.trim().to_string()
            };
            let summary = json!({
                "message": message,
                "project": project,
                "id": id,
                "stdout": stdout.trim(),
            });
            print_envelope_or_human(format, &command_name, code, summary, || {
                if code == 0 {
                    println!("{}", stdout.trim());
                } else {
                    eprint!("{stderr}");
                }
            })
        }
        DockerOp::Run {
            image,
            name,
            env,
            ports,
            cmd,
        } => {
            let mut args = vec!["run".to_string(), "-d".into()];
            if let Some(name) = name {
                args.push("--name".into());
                args.push(name.clone());
            }
            for value in env {
                args.push("-e".into());
                args.push(value.clone());
            }
            for value in ports {
                args.push("-p".into());
                args.push(value.clone());
            }
            args.push(image.clone());
            args.extend(cmd.iter().cloned());
            let (code, stdout, stderr) = match docker_output(context, ssh_command, &args) {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            };
            let container_id = stdout.trim().to_string();
            let summary = if code == 0 {
                json!({
                    "message": format!("started {container_id}"),
                    "project": project,
                    "container_id": container_id,
                    "image": image,
                })
            } else {
                json!({
                    "message": stderr.trim(),
                    "project": project,
                    "image": image,
                })
            };
            print_envelope_or_human(format, "docker.run", code, summary, || {
                if code == 0 {
                    println!("{container_id}");
                } else {
                    eprint!("{stderr}");
                }
            })
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

fn envelope(command: &str, status: &str, exit_code: u8, summary: JsonValue) -> String {
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

    #[test]
    fn parse_ps_all_and_format_json() {
        let parsed = parse_docker_args(
            ["--project", "edge-dmz", "ps", "--all", "--format", "json"]
                .into_iter()
                .map(String::from)
                .collect(),
        )
        .unwrap();
        assert_eq!(parsed.project.as_deref(), Some("edge-dmz"));
        assert_eq!(parsed.format, OutputFormat::Json);
        assert_eq!(parsed.op, DockerOp::Ps { all: true });
    }

    #[test]
    fn parse_inspect_and_lifecycle() {
        let inspect = parse_docker_args(
            ["inspect", "abc123", "--format", "json"]
                .into_iter()
                .map(String::from)
                .collect(),
        )
        .unwrap();
        assert_eq!(
            inspect.op,
            DockerOp::Inspect {
                id: "abc123".into()
            }
        );

        let stop = parse_docker_args(
            ["--project", "p", "stop", "cid"]
                .into_iter()
                .map(String::from)
                .collect(),
        )
        .unwrap();
        assert_eq!(stop.op, DockerOp::Stop { id: "cid".into() });
    }

    #[test]
    fn parse_run_accepts_project_after_verb() {
        let parsed = parse_docker_args(
            [
                "run",
                "--project",
                "edge-dmz",
                "--image",
                "nginx:alpine",
                "--format",
                "json",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        )
        .unwrap();
        assert_eq!(parsed.project.as_deref(), Some("edge-dmz"));
        assert_eq!(parsed.format, OutputFormat::Json);
        assert_eq!(
            parsed.op,
            DockerOp::Run {
                image: "nginx:alpine".into(),
                name: None,
                env: vec![],
                ports: vec![],
                cmd: vec![],
            }
        );
    }

    #[test]
    fn parse_run_with_flags_and_cmd() {
        let parsed = parse_docker_args(
            [
                "run",
                "--image",
                "nginx:alpine",
                "--name",
                "web",
                "-e",
                "FOO=bar",
                "-p",
                "8080:80",
                "--",
                "nginx",
                "-g",
                "daemon off;",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        )
        .unwrap();
        assert_eq!(
            parsed.op,
            DockerOp::Run {
                image: "nginx:alpine".into(),
                name: Some("web".into()),
                env: vec!["FOO=bar".into()],
                ports: vec!["8080:80".into()],
                cmd: vec!["nginx".into(), "-g".into(), "daemon off;".into()],
            }
        );
    }

    #[test]
    fn parse_passthrough_with_double_dash() {
        let parsed = parse_docker_args(
            ["--project", "p", "--", "compose", "version"]
                .into_iter()
                .map(String::from)
                .collect(),
        )
        .unwrap();
        assert_eq!(
            parsed.op,
            DockerOp::Passthrough(vec!["compose".into(), "version".into()])
        );
    }

    #[test]
    fn parse_unknown_verb_is_passthrough() {
        let parsed =
            parse_docker_args(["compose", "ps"].into_iter().map(String::from).collect()).unwrap();
        assert_eq!(
            parsed.op,
            DockerOp::Passthrough(vec!["compose".into(), "ps".into()])
        );
    }

    #[test]
    fn write_ssh_config_quotes_paths_with_spaces() {
        let root = std::env::temp_dir().join(format!("vzctl-ssh-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let docker_dir = root.join("projects/edge-dmz/docker");
        fs::create_dir_all(&docker_dir).unwrap();
        let key = docker_dir.join("id_ed25519");
        fs::write(&key, "dummy").unwrap();
        write_ssh_config(&root, "edge-dmz", &key).unwrap();
        let body = fs::read_to_string(ssh_config_path(&root, "edge-dmz")).unwrap();
        assert!(
            body.contains(&format!("IdentityFile \"{}\"", key.display())),
            "expected quoted IdentityFile, got:\n{body}"
        );
        assert!(
            body.contains(&format!(
                "UserKnownHostsFile \"{}/known_hosts\"",
                docker_dir.display()
            )),
            "expected quoted UserKnownHostsFile, got:\n{body}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ndjson_containers_normalize_fields() {
        let stdout = r#"{"ID":"abcd1234","Names":"web","Image":"nginx","Status":"Up 1s","State":"running","Ports":"80/tcp","Command":"\"nginx\"","NetworkSettings":{"Networks":{"bridge":{"IPAddress":"172.17.0.2"}}}}
{"ID":"efgh5678","Names":"db","Image":"postgres","Status":"Exited","State":"exited","Ports":"","Command":"\"postgres\""}
"#;
        let containers = parse_ndjson_containers(stdout);
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["id"], "abcd1234");
        assert_eq!(containers[0]["names"], "web");
        assert_eq!(containers[0]["ip"], "172.17.0.2");
        assert_eq!(containers[1]["state"], "exited");
        assert_eq!(containers[1]["ip"], "");
    }

    #[test]
    fn envelope_shape_for_ps() {
        let body = envelope(
            "docker.ps",
            "ok",
            0,
            json!({
                "message": "1 container(s)",
                "project": "edge-dmz",
                "containers": [{"id": "abc", "names": "web"}],
            }),
        );
        let value: JsonValue = serde_json::from_str(&body).unwrap();
        assert_eq!(value["apiVersion"], API_VERSION);
        assert_eq!(value["command"], "docker.ps");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["summary"]["containers"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn resolve_docker_bin_reports_missing_clearly_or_finds_binary() {
        match resolve_docker_bin() {
            Ok(path) => assert!(
                path.file_name().and_then(|n| n.to_str()) == Some("docker"),
                "unexpected docker path {}",
                path.display()
            ),
            Err(message) => assert!(
                message.contains("docker CLI not found"),
                "unexpected error: {message}"
            ),
        }
    }
}
