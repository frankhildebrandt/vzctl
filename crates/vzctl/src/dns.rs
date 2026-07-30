use serde_json::json;
use serde_yaml::Value as YamlValue;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const API_VERSION: &str = "vzctl.dev/v1";
const DEFAULT_CONFIG: &str = "hypernetwork.config.yaml";
const DEFAULT_DNS_PORT: u16 = 15353;
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
pub(crate) const EXIT_RESOLVER: u8 = 19;
const MANAGED_MARKER: &str = "# managed-by: vzctl";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Install,
    Uninstall,
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Install => "dns.install-resolver",
            Self::Uninstall => "dns.uninstall-resolver",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    action: Action,
    project: Option<String>,
    config: PathBuf,
    config_explicit: bool,
    format: Format,
}

#[derive(Debug, Eq, PartialEq)]
struct Scope {
    project: String,
    owner: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Change {
    Installed,
    Updated,
    Unchanged,
    Removed,
    Absent,
}

impl Change {
    fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
            Self::Removed => "removed",
            Self::Absent => "absent",
        }
    }
}

#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) fn command(args: impl Iterator<Item = String>) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let options = match parse(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "dns", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let scope = match resolve_scope(&options) {
        Ok(scope) => scope,
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            return ExitCode::from(failure.code);
        }
    };
    let resolver_dir = std::env::var_os("VZCTL_RESOLVER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/resolver"));
    let port = match dns_port() {
        Ok(port) => port,
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            return ExitCode::from(failure.code);
        }
    };
    let result = match options.action {
        Action::Install => install(&resolver_dir, &scope, port),
        Action::Uninstall => uninstall(&resolver_dir, &scope),
    };
    match result {
        Ok((path, change)) => {
            emit_success(options.format, options.action, &scope, &path, port, change);
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_failure(options.format, options.action.command(), &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, Failure> {
    let action = match args.next().as_deref() {
        Some("install-resolver") => Action::Install,
        Some("uninstall-resolver") => Action::Uninstall,
        _ => return Err(Failure::new(EXIT_USAGE, usage())),
    };
    let mut project = None;
    let mut config = PathBuf::from(DEFAULT_CONFIG);
    let mut config_explicit = false;
    let mut format = Format::Human;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--project" => {
                let value = next_value(&mut args, "--project requires a project")?;
                if project.replace(value).is_some() {
                    return Err(Failure::new(EXIT_USAGE, "--project may only be used once"));
                }
            }
            "--config" => {
                let value = next_value(&mut args, "--config requires a path")?;
                if config_explicit {
                    return Err(Failure::new(EXIT_USAGE, "--config may only be used once"));
                }
                config = PathBuf::from(value);
                config_explicit = true;
            }
            "--format" => {
                let value = next_value(&mut args, "--format requires human or json")?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    _ => {
                        return Err(Failure::new(
                            EXIT_USAGE,
                            format!("unsupported dns format: {value}"),
                        ))
                    }
                };
            }
            "-h" | "--help" => return Err(Failure::new(EXIT_USAGE, usage())),
            _ => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown dns option: {argument}"),
                ))
            }
        }
    }
    Ok(Options {
        action,
        project,
        config,
        config_explicit,
        format,
    })
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    message: &'static str,
) -> Result<String, Failure> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::new(EXIT_USAGE, message))
}

fn resolve_scope(options: &Options) -> Result<Scope, Failure> {
    let config_exists = options.config.is_file();
    if options.config_explicit && !config_exists {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("config does not exist: {}", options.config.display()),
        ));
    }
    let config_project = if config_exists {
        Some(project_from_config(&options.config)?)
    } else {
        None
    };
    if let (Some(explicit), Some(configured)) = (&options.project, &config_project) {
        if explicit != configured {
            return Err(Failure::new(
                EXIT_INVALID,
                format!(
                    "--project {explicit} does not match spec.project {configured} in {}",
                    options.config.display()
                ),
            ));
        }
    }
    let project = options.project.clone().or(config_project).ok_or_else(|| {
        Failure::new(
            EXIT_INVALID,
            format!("no --project and no {DEFAULT_CONFIG}"),
        )
    })?;
    validate_project(&project)?;
    let owner = if config_exists {
        let canonical = fs::canonicalize(&options.config).map_err(|error| {
            Failure::new(
                EXIT_INVALID,
                format!(
                    "cannot resolve config {}: {error}",
                    options.config.display()
                ),
            )
        })?;
        format!(
            "config-{:016x}",
            stable_hash(canonical.as_os_str().as_encoded_bytes())
        )
    } else {
        format!("project-{project}")
    };
    Ok(Scope { project, owner })
}

fn project_from_config(path: &Path) -> Result<String, Failure> {
    let text = fs::read_to_string(path).map_err(|error| {
        Failure::new(
            EXIT_INVALID,
            format!("cannot read config {}: {error}", path.display()),
        )
    })?;
    let document: YamlValue = serde_yaml::from_str(&text).map_err(|error| {
        Failure::new(
            EXIT_INVALID,
            format!("cannot parse config {}: {error}", path.display()),
        )
    })?;
    document
        .get("spec")
        .and_then(|spec| spec.get("project"))
        .and_then(YamlValue::as_str)
        .filter(|project| !project.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            Failure::new(
                EXIT_INVALID,
                format!("config {} has no spec.project", path.display()),
            )
        })
}

fn validate_project(project: &str) -> Result<(), Failure> {
    let valid = project.len() <= 63
        && project
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && project
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && project
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);
    if valid {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_INVALID,
            "project must be a lowercase DNS label (a-z, 0-9, hyphen; max 63 characters)",
        ))
    }
}

fn dns_port() -> Result<u16, Failure> {
    match std::env::var("VZCTL_DNS_PORT") {
        Ok(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| Failure::new(EXIT_INVALID, format!("invalid VZCTL_DNS_PORT: {value}"))),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_DNS_PORT),
        Err(error) => Err(Failure::new(
            EXIT_INVALID,
            format!("invalid VZCTL_DNS_PORT: {error}"),
        )),
    }
}

fn install(resolver_dir: &Path, scope: &Scope, port: u16) -> Result<(PathBuf, Change), Failure> {
    ensure_resolver_dir(resolver_dir)?;
    let path = resolver_path(resolver_dir, &scope.project);
    let desired = resolver_content(scope, port);
    let change = match read_existing(&path)? {
        None => Change::Installed,
        Some(existing) => {
            ensure_owned(&path, &existing, scope)?;
            if existing == desired {
                return Ok((path, Change::Unchanged));
            }
            Change::Updated
        }
    };
    atomic_write(&path, desired.as_bytes()).map_err(|error| io_failure("write", &path, error))?;
    Ok((path, change))
}

fn uninstall(resolver_dir: &Path, scope: &Scope) -> Result<(PathBuf, Change), Failure> {
    let path = resolver_path(resolver_dir, &scope.project);
    let Some(existing) = read_existing(&path)? else {
        return Ok((path, Change::Absent));
    };
    ensure_owned(&path, &existing, scope)?;
    let before =
        fs::symlink_metadata(&path).map_err(|error| io_failure("inspect", &path, error))?;
    let after = fs::symlink_metadata(&path).map_err(|error| io_failure("inspect", &path, error))?;
    if before.dev() != after.dev() || before.ino() != after.ino() || !after.file_type().is_file() {
        return Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver changed during cleanup; refusing to remove {}",
                path.display()
            ),
        ));
    }
    fs::remove_file(&path).map_err(|error| io_failure("remove", &path, error))?;
    Ok((path, Change::Removed))
}

fn ensure_resolver_dir(path: &Path) -> Result<(), Failure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver directory must not be a symlink: {}",
                path.display()
            ),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(Failure::new(
            EXIT_RESOLVER,
            format!("resolver path is not a directory: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| io_failure("create directory", path, error))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|error| io_failure("set permissions on", path, error))
        }
        Err(error) => Err(io_failure("inspect", path, error)),
    }
}

fn read_existing(path: &Path) -> Result<Option<String>, Failure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_failure("inspect", path, error)),
    };
    if !metadata.file_type().is_file() {
        return Err(Failure::new(
            EXIT_RESOLVER,
            format!("resolver target is not a regular file: {}", path.display()),
        ));
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|error| io_failure("read", path, error))
}

fn ensure_owned(path: &Path, content: &str, scope: &Scope) -> Result<(), Failure> {
    let project_marker = format!("# project: {}", scope.project);
    let owner_marker = format!("# owner: {}", scope.owner);
    if content.lines().any(|line| line == MANAGED_MARKER)
        && content.lines().any(|line| line == project_marker)
        && content.lines().any(|line| line == owner_marker)
    {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_RESOLVER,
            format!(
                "resolver collision at {}; file is not owned by this project/config",
                path.display()
            ),
        ))
    }
}

fn resolver_path(resolver_dir: &Path, project: &str) -> PathBuf {
    resolver_dir.join(format!("{project}.vz.test"))
}

fn resolver_content(scope: &Scope, port: u16) -> String {
    format!(
        "{MANAGED_MARKER}\n# project: {}\n# owner: {}\nnameserver 127.0.0.1\nport {port}\n",
        scope.project, scope.owner
    )
}

fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "resolver has no parent"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid resolver filename"))?;
    let mut last_error = None;
    for attempt in 0..100 {
        let temporary = parent.join(format!(".{file_name}.tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(mut file) => {
                let result = publish_temporary(&mut file, &temporary, path, content);
                if result.is_err() {
                    let _ = fs::remove_file(&temporary);
                }
                return result;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no temp name")))
}

fn publish_temporary(
    file: &mut File,
    temporary: &Path,
    destination: &Path,
    content: &[u8],
) -> io::Result<()> {
    file.write_all(content)?;
    file.sync_all()?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    fs::rename(temporary, destination)?;
    File::open(
        destination
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent"))?,
    )?
    .sync_all()
}

fn io_failure(operation: &str, path: &Path, error: io::Error) -> Failure {
    let sudo = if error.kind() == io::ErrorKind::PermissionDenied {
        "; run this command with sudo"
    } else {
        ""
    };
    Failure::new(
        EXIT_RESOLVER,
        format!("{operation} {}: {error}{sudo}", path.display()),
    )
}

fn emit_success(
    format: Format,
    action: Action,
    scope: &Scope,
    path: &Path,
    port: u16,
    change: Change,
) {
    let message = format!("resolver {}: {}", change.as_str(), path.display());
    match format {
        Format::Human => println!("{message}"),
        Format::Json => println!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": action.command(),
                "status": "ok",
                "exit_code": 0,
                "summary": {
                    "message": message,
                    "change": change.as_str(),
                },
                "resolver": {
                    "project": scope.project,
                    "domain": format!("{}.vz.test", scope.project),
                    "path": path,
                    "nameserver": "127.0.0.1",
                    "port": port,
                    "managed": true,
                }
            })
        ),
    }
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    match format {
        Format::Human => eprintln!("{}", failure.message),
        Format::Json => println!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": command,
                "status": "fail",
                "exit_code": failure.code,
                "summary": { "message": failure.message },
            })
        ),
    }
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| Format::Json)
        .unwrap_or(Format::Human)
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn usage() -> &'static str {
    "usage: vzctl dns install-resolver|uninstall-resolver [--project <name>] [--config <path>] [--format human|json]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vzctl-dns-{name}-{}-{unique}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    fn scope(owner: &str) -> Scope {
        Scope {
            project: "edge-dmz".to_string(),
            owner: owner.to_string(),
        }
    }

    #[test]
    fn install_and_uninstall_are_idempotent() {
        let dir = temp_dir("idempotent");
        let scope = scope("config-a");
        let (path, first) = install(&dir, &scope, 15353).unwrap();
        assert_eq!(first, Change::Installed);
        assert_eq!(install(&dir, &scope, 15353).unwrap().1, Change::Unchanged);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(uninstall(&dir, &scope).unwrap().1, Change::Removed);
        assert_eq!(uninstall(&dir, &scope).unwrap().1, Change::Absent);
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn install_updates_port_for_same_owner() {
        let dir = temp_dir("update");
        let scope = scope("config-a");
        let (path, _) = install(&dir, &scope, 15353).unwrap();
        assert_eq!(install(&dir, &scope, 15354).unwrap().1, Change::Updated);
        assert!(fs::read_to_string(path).unwrap().contains("port 15354"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn collision_is_never_overwritten_or_removed() {
        let dir = temp_dir("collision");
        let first = scope("config-a");
        let second = scope("config-b");
        let (path, _) = install(&dir, &first, 15353).unwrap();
        let original = fs::read_to_string(&path).unwrap();
        assert_eq!(
            install(&dir, &second, 15353).unwrap_err().code,
            EXIT_RESOLVER
        );
        assert_eq!(uninstall(&dir, &second).unwrap_err().code, EXIT_RESOLVER);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn symlink_target_is_rejected() {
        let dir = temp_dir("symlink");
        let target = dir.join("foreign");
        fs::write(&target, "keep\n").unwrap();
        let path = resolver_path(&dir, "edge-dmz");
        symlink(&target, &path).unwrap();
        assert_eq!(
            install(&dir, &scope("config-a"), 15353).unwrap_err().code,
            EXIT_RESOLVER
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "keep\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scope_uses_spec_project_and_config_identity() {
        let dir = temp_dir("config");
        let config = dir.join(DEFAULT_CONFIG);
        fs::write(
            &config,
            "apiVersion: hypernetwork/v1\nspec:\n  project: edge-dmz\n",
        )
        .unwrap();
        let options = Options {
            action: Action::Install,
            project: None,
            config: config.clone(),
            config_explicit: false,
            format: Format::Human,
        };
        let resolved = resolve_scope(&options).unwrap();
        assert_eq!(resolved.project, "edge-dmz");
        assert!(resolved.owner.starts_with("config-"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_noncanonical_project_labels() {
        assert!(validate_project("edge-dmz").is_ok());
        for project in ["Edge", "../edge", "edge.dmz", "-edge", "edge-", ""] {
            assert_eq!(validate_project(project).unwrap_err().code, EXIT_INVALID);
        }
    }

    #[test]
    fn json_success_uses_cli_v1_command_name() {
        let value: Value = json!({
            "apiVersion": API_VERSION,
            "command": Action::Install.command(),
            "status": "ok",
            "exit_code": 0
        });
        assert_eq!(value["command"], "dns.install-resolver");
        assert_eq!(value["apiVersion"], API_VERSION);
    }
}
