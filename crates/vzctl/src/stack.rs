use crate::config::{
    add_mount, add_network, add_vm, add_volume, config_path, remove_mount, remove_network,
    remove_vm, remove_volume, scaffold_environment, validate_path, write_environment_atomic,
    AddNetworkOptions, AddVmOptions, Environment, NetworkBackend, NetworkMode, ValidationIssue,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const API_VERSION: &str = "vzctl.dev/v1";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
}

pub(crate) fn command(args: impl Iterator<Item = String>) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let subcommand = args.first().map(String::as_str);
    match subcommand {
        Some("init") => init_command(&args[1..], requested_format),
        Some("vm") => vm_command(&args[1..], requested_format),
        Some("net") => net_command(&args[1..], requested_format),
        Some("volume") => volume_command(&args[1..], requested_format),
        Some("mount") => mount_command(&args[1..], requested_format),
        Some(other) => {
            emit_failure(
                requested_format,
                "stack",
                &Failure::new(EXIT_USAGE, format!("unknown stack subcommand: {other}")),
            );
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            emit_failure(
                requested_format,
                "stack",
                &Failure::new(EXIT_USAGE, usage()),
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn init_command(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_init(args) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.init", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.format;
    let target_dir = options.directory;
    if let Err(error) = fs::create_dir_all(&target_dir) {
        let failure = Failure::new(
            EXIT_INVALID,
            format!("cannot create {}: {error}", target_dir.display()),
        );
        emit_failure(format, "stack.init", &failure);
        return ExitCode::from(EXIT_INVALID);
    }
    let config_file = config_path(&target_dir);
    if config_file.exists() && !options.force {
        let failure = Failure::new(
            EXIT_INVALID,
            format!(
                "config already exists: {}; use --force to overwrite",
                config_file.display()
            ),
        );
        emit_failure(format, "stack.init", &failure);
        return ExitCode::from(EXIT_INVALID);
    }
    let environment = match scaffold_environment(&options.name, &options.cidr) {
        Ok(environment) => environment,
        Err(message) => {
            let failure = Failure::new(EXIT_INVALID, message);
            emit_failure(format, "stack.init", &failure);
            return ExitCode::from(EXIT_INVALID);
        }
    };
    match write_environment_atomic(&config_file, &environment) {
        Ok(validated) => {
            emit_success(
                format,
                "stack.init",
                json!({
                    "path": config_file,
                    "name": validated.metadata.name,
                    "project": validated.spec.project,
                }),
                format!(
                    "initialized stack at {} (project {})",
                    config_file.display(),
                    validated.spec.project
                ),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.init", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

struct InitOptions {
    directory: PathBuf,
    name: String,
    cidr: String,
    force: bool,
    format: Format,
}

fn parse_init(args: &[String]) -> Result<InitOptions, Failure> {
    let mut directory = PathBuf::from(".");
    let mut name = None;
    let mut cidr = "10.80.0.0/24".to_string();
    let mut force = false;
    let mut format = Format::Human;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                directory = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--name" => {
                index += 1;
                name = Some(next_value(args, index, "--name requires a value")?);
                index += 1;
            }
            "--cidr" => {
                index += 1;
                cidr = next_value(args, index, "--cidr requires a value")?;
                index += 1;
            }
            "--force" => {
                force = true;
                index += 1;
            }
            "--format" => {
                index += 1;
                format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack init option: {argument}"),
                ));
            }
            argument => {
                directory = PathBuf::from(argument);
                index += 1;
            }
        }
    }
    let name = name.ok_or_else(|| Failure::new(EXIT_USAGE, "--name is required"))?;
    Ok(InitOptions {
        directory,
        name,
        cidr,
        force,
        format,
    })
}

fn vm_command(args: &[String], requested_format: Format) -> ExitCode {
    let action = args.first().map(String::as_str);
    match action {
        Some("add") => vm_add(&args[1..], requested_format),
        Some("remove") => vm_remove(&args[1..], requested_format),
        Some(other) => {
            emit_failure(
                requested_format,
                "stack.vm",
                &Failure::new(EXIT_USAGE, format!("unknown stack vm subcommand: {other}")),
            );
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            emit_failure(
                requested_format,
                "stack.vm",
                &Failure::new(EXIT_USAGE, vm_usage()),
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn vm_add(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_vm_add(args) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.vm.add", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.common.format;
    let config_file = config_path(&options.common.config);
    let mut environment = match load_environment(&config_file, format, "stack.vm.add") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = add_vm(&mut environment, &options.name, &options.vm) {
        emit_validation_failure(format, "stack.vm.add", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(validated) => {
            let ip = validated
                .spec
                .vms
                .get(&options.name)
                .and_then(|vm| vm.networks.first())
                .map(|network| network.ip.clone())
                .unwrap_or_default();
            emit_success(
                format,
                "stack.vm.add",
                json!({
                    "path": config_file,
                    "vm": options.name,
                    "ip": ip,
                }),
                format!("added VM {} to {}", options.name, config_file.display()),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.vm.add", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

fn vm_remove(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_named_remove(args, "stack vm remove") {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.vm.remove", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.format;
    let config_file = config_path(&options.config);
    let mut environment = match load_environment(&config_file, format, "stack.vm.remove") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = remove_vm(&mut environment, &options.name) {
        emit_validation_failure(format, "stack.vm.remove", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.vm.remove",
                json!({
                    "path": config_file,
                    "vm": options.name,
                }),
                format!("removed VM {} from {}", options.name, config_file.display()),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.vm.remove", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

struct VmAddOptions {
    common: CommonOptions,
    name: String,
    vm: AddVmOptions,
}

fn parse_vm_add(args: &[String]) -> Result<VmAddOptions, Failure> {
    let mut common = CommonOptions::default();
    let mut name = None;
    let mut from_image = "ubuntu-base".to_string();
    let mut network = None;
    let mut ip = None;
    let mut disk = "4G".to_string();
    let mut cpus = None;
    let mut memory = None;
    let mut roles = Vec::new();
    let mut cloud_init = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                common.config = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--format" => {
                index += 1;
                common.format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            "--from" => {
                index += 1;
                from_image = next_value(args, index, "--from requires a value")?;
                index += 1;
            }
            "--network" => {
                index += 1;
                network = Some(next_value(args, index, "--network requires a value")?);
                index += 1;
            }
            "--ip" => {
                index += 1;
                ip = Some(next_value(args, index, "--ip requires a value")?);
                index += 1;
            }
            "--disk" | "--data-disk" => {
                index += 1;
                disk = next_value(args, index, "--disk requires a value")?;
                index += 1;
            }
            "--cpus" => {
                index += 1;
                let raw = next_value(args, index, "--cpus requires a value")?;
                cpus = Some(raw.parse().map_err(|_| {
                    Failure::new(EXIT_INVALID, "--cpus must be a positive integer")
                })?);
                index += 1;
            }
            "--memory" => {
                index += 1;
                memory = Some(next_value(args, index, "--memory requires a value")?);
                index += 1;
            }
            "--role" => {
                index += 1;
                let role = next_value(args, index, "--role requires router or docker")?;
                if role != "router" && role != "docker" {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        "--role must be router or docker",
                    ));
                }
                roles.push(role);
                index += 1;
            }
            "--cloud-init" => {
                index += 1;
                cloud_init = Some(next_value(args, index, "--cloud-init requires a path")?);
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack vm add option: {argument}"),
                ));
            }
            argument => {
                if name.is_some() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "stack vm add accepts only one VM name",
                    ));
                }
                name = Some(argument.to_string());
                index += 1;
            }
        }
    }
    let name = name.ok_or_else(|| Failure::new(EXIT_USAGE, "stack vm add requires a VM name"))?;
    Ok(VmAddOptions {
        common,
        name,
        vm: AddVmOptions {
            from_image,
            network,
            ip,
            disk,
            cpus,
            memory,
            roles,
            cloud_init,
        },
    })
}

fn net_command(args: &[String], requested_format: Format) -> ExitCode {
    let action = args.first().map(String::as_str);
    match action {
        Some("add") => net_add(&args[1..], requested_format),
        Some("remove") => net_remove(&args[1..], requested_format),
        Some(other) => {
            emit_failure(
                requested_format,
                "stack.net",
                &Failure::new(EXIT_USAGE, format!("unknown stack net subcommand: {other}")),
            );
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            emit_failure(
                requested_format,
                "stack.net",
                &Failure::new(EXIT_USAGE, net_usage()),
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn net_add(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_net_add(args) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.net.add", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.common.format;
    let config_file = config_path(&options.common.config);
    let mut environment = match load_environment(&config_file, format, "stack.net.add") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = add_network(&mut environment, &options.name, &options.network) {
        emit_validation_failure(format, "stack.net.add", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.net.add",
                json!({
                    "path": config_file,
                    "network": options.name,
                    "cidr": options.network.cidr,
                }),
                format!(
                    "added network {} ({}) to {}",
                    options.name,
                    options.network.cidr,
                    config_file.display()
                ),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.net.add", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

fn net_remove(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_named_remove(args, "stack net remove") {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.net.remove", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.format;
    let config_file = config_path(&options.config);
    let mut environment = match load_environment(&config_file, format, "stack.net.remove") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = remove_network(&mut environment, &options.name) {
        emit_validation_failure(format, "stack.net.remove", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.net.remove",
                json!({
                    "path": config_file,
                    "network": options.name,
                }),
                format!(
                    "removed network {} from {}",
                    options.name,
                    config_file.display()
                ),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.net.remove", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

struct NetAddOptions {
    common: CommonOptions,
    name: String,
    network: AddNetworkOptions,
}

fn parse_net_add(args: &[String]) -> Result<NetAddOptions, Failure> {
    let mut common = CommonOptions::default();
    let mut name = None;
    let mut cidr = None;
    let mut mode = NetworkMode::Shared;
    let mut backend = NetworkBackend::Vmnet;
    let mut nat_egress = true;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                common.config = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--format" => {
                index += 1;
                common.format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            "--cidr" => {
                index += 1;
                cidr = Some(next_value(args, index, "--cidr requires a value")?);
                index += 1;
            }
            "--mode" => {
                index += 1;
                let raw = next_value(args, index, "--mode requires shared or host")?;
                mode = match raw.as_str() {
                    "shared" => NetworkMode::Shared,
                    "host" => NetworkMode::Host,
                    _ => {
                        return Err(Failure::new(EXIT_INVALID, "--mode must be shared or host"));
                    }
                };
                index += 1;
            }
            "--backend" => {
                index += 1;
                let raw = next_value(args, index, "--backend requires vmnet or docker")?;
                backend = match raw.as_str() {
                    "vmnet" => NetworkBackend::Vmnet,
                    "docker" => NetworkBackend::Docker,
                    _ => {
                        return Err(Failure::new(
                            EXIT_INVALID,
                            "--backend must be vmnet or docker",
                        ));
                    }
                };
                index += 1;
            }
            "--nat-egress" => {
                nat_egress = true;
                index += 1;
            }
            "--no-nat-egress" => {
                nat_egress = false;
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack net add option: {argument}"),
                ));
            }
            argument => {
                if name.is_some() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "stack net add accepts only one network name",
                    ));
                }
                name = Some(argument.to_string());
                index += 1;
            }
        }
    }
    let name =
        name.ok_or_else(|| Failure::new(EXIT_USAGE, "stack net add requires a network name"))?;
    let cidr = cidr.ok_or_else(|| Failure::new(EXIT_USAGE, "--cidr is required"))?;
    if backend == NetworkBackend::Docker {
        nat_egress = false;
    }
    Ok(NetAddOptions {
        common,
        name,
        network: AddNetworkOptions {
            cidr,
            mode,
            backend,
            nat_egress,
        },
    })
}

fn volume_command(args: &[String], requested_format: Format) -> ExitCode {
    let action = args.first().map(String::as_str);
    match action {
        Some("add") => volume_add(&args[1..], requested_format),
        Some("remove") => volume_remove(&args[1..], requested_format),
        Some(other) => {
            emit_failure(
                requested_format,
                "stack.volume",
                &Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack volume subcommand: {other}"),
                ),
            );
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            emit_failure(
                requested_format,
                "stack.volume",
                &Failure::new(EXIT_USAGE, volume_usage()),
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn volume_add(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_volume_add(args) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.volume.add", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.common.format;
    let config_file = config_path(&options.common.config);
    let mut environment = match load_environment(&config_file, format, "stack.volume.add") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = add_volume(&mut environment, &options.name, &options.path) {
        emit_validation_failure(format, "stack.volume.add", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.volume.add",
                json!({
                    "path": config_file,
                    "volume": options.name,
                    "hostPath": options.path,
                }),
                format!(
                    "added volume {} -> {} to {}",
                    options.name,
                    options.path,
                    config_file.display()
                ),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.volume.add", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

fn volume_remove(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_named_remove(args, "stack volume remove") {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.volume.remove", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.format;
    let config_file = config_path(&options.config);
    let mut environment = match load_environment(&config_file, format, "stack.volume.remove") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = remove_volume(&mut environment, &options.name) {
        emit_validation_failure(format, "stack.volume.remove", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.volume.remove",
                json!({
                    "path": config_file,
                    "volume": options.name,
                }),
                format!(
                    "removed volume {} from {}",
                    options.name,
                    config_file.display()
                ),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.volume.remove", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

struct VolumeAddOptions {
    common: CommonOptions,
    name: String,
    path: String,
}

fn parse_volume_add(args: &[String]) -> Result<VolumeAddOptions, Failure> {
    let mut common = CommonOptions::default();
    let mut name = None;
    let mut path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                common.config = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--format" => {
                index += 1;
                common.format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack volume add option: {argument}"),
                ));
            }
            argument => {
                if name.is_none() {
                    name = Some(argument.to_string());
                } else if path.is_none() {
                    path = Some(argument.to_string());
                } else {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "stack volume add accepts only NAME PATH",
                    ));
                }
                index += 1;
            }
        }
    }
    let name =
        name.ok_or_else(|| Failure::new(EXIT_USAGE, "stack volume add requires a volume name"))?;
    let path = path.ok_or_else(|| Failure::new(EXIT_USAGE, "stack volume add requires a path"))?;
    Ok(VolumeAddOptions { common, name, path })
}

fn mount_command(args: &[String], requested_format: Format) -> ExitCode {
    let action = args.first().map(String::as_str);
    match action {
        Some("add") => mount_add(&args[1..], requested_format),
        Some("remove") => mount_remove(&args[1..], requested_format),
        Some(other) => {
            emit_failure(
                requested_format,
                "stack.mount",
                &Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack mount subcommand: {other}"),
                ),
            );
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            emit_failure(
                requested_format,
                "stack.mount",
                &Failure::new(EXIT_USAGE, mount_usage()),
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn mount_add(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_mount_add(args) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.mount.add", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.common.format;
    let config_file = config_path(&options.common.config);
    let mut environment = match load_environment(&config_file, format, "stack.mount.add") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = add_mount(
        &mut environment,
        &options.vm,
        &options.source,
        &options.target,
        options.read_only,
    ) {
        emit_validation_failure(format, "stack.mount.add", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.mount.add",
                json!({
                    "path": config_file,
                    "vm": options.vm,
                    "source": options.source,
                    "target": options.target,
                    "readOnly": options.read_only,
                }),
                format!(
                    "added mount {} -> {} on VM {}",
                    options.source, options.target, options.vm
                ),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.mount.add", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

fn mount_remove(args: &[String], requested_format: Format) -> ExitCode {
    let options = match parse_mount_remove(args) {
        Ok(options) => options,
        Err(failure) => {
            emit_failure(requested_format, "stack.mount.remove", &failure);
            return ExitCode::from(failure.code);
        }
    };
    let format = options.common.format;
    let config_file = config_path(&options.common.config);
    let mut environment = match load_environment(&config_file, format, "stack.mount.remove") {
        Ok(environment) => environment,
        Err(code) => return ExitCode::from(code),
    };
    if let Err(issues) = remove_mount(&mut environment, &options.vm, &options.target) {
        emit_validation_failure(format, "stack.mount.remove", &config_file, &issues);
        return ExitCode::from(EXIT_INVALID);
    }
    match write_environment_atomic(&config_file, &environment) {
        Ok(_) => {
            emit_success(
                format,
                "stack.mount.remove",
                json!({
                    "path": config_file,
                    "vm": options.vm,
                    "target": options.target,
                }),
                format!("removed mount {} from VM {}", options.target, options.vm),
            );
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(format, "stack.mount.remove", &config_file, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

struct MountAddOptions {
    common: CommonOptions,
    vm: String,
    source: String,
    target: String,
    read_only: bool,
}

fn parse_mount_add(args: &[String]) -> Result<MountAddOptions, Failure> {
    let mut common = CommonOptions::default();
    let mut vm = None;
    let mut source = None;
    let mut target = None;
    let mut read_only = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                common.config = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--format" => {
                index += 1;
                common.format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            "--source" => {
                index += 1;
                source = Some(next_value(args, index, "--source requires a value")?);
                index += 1;
            }
            "--target" => {
                index += 1;
                target = Some(next_value(args, index, "--target requires a value")?);
                index += 1;
            }
            "--read-only" => {
                read_only = true;
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack mount add option: {argument}"),
                ));
            }
            argument => {
                if vm.is_none() {
                    vm = Some(argument.to_string());
                } else {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "stack mount add accepts only one VM name",
                    ));
                }
                index += 1;
            }
        }
    }
    let vm = vm.ok_or_else(|| Failure::new(EXIT_USAGE, "stack mount add requires a VM name"))?;
    let source = source.ok_or_else(|| Failure::new(EXIT_USAGE, "--source is required"))?;
    let target = target.ok_or_else(|| Failure::new(EXIT_USAGE, "--target is required"))?;
    Ok(MountAddOptions {
        common,
        vm,
        source,
        target,
        read_only,
    })
}

struct MountRemoveOptions {
    common: CommonOptions,
    vm: String,
    target: String,
}

fn parse_mount_remove(args: &[String]) -> Result<MountRemoveOptions, Failure> {
    let mut common = CommonOptions::default();
    let mut vm = None;
    let mut target = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                common.config = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--format" => {
                index += 1;
                common.format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            "--target" => {
                index += 1;
                target = Some(next_value(args, index, "--target requires a value")?);
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown stack mount remove option: {argument}"),
                ));
            }
            argument => {
                if vm.is_none() {
                    vm = Some(argument.to_string());
                } else {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        "stack mount remove accepts only one VM name",
                    ));
                }
                index += 1;
            }
        }
    }
    let vm = vm.ok_or_else(|| Failure::new(EXIT_USAGE, "stack mount remove requires a VM name"))?;
    let target = target.ok_or_else(|| Failure::new(EXIT_USAGE, "--target is required"))?;
    Ok(MountRemoveOptions { common, vm, target })
}

#[derive(Debug)]
struct CommonOptions {
    config: PathBuf,
    format: Format,
}

impl Default for CommonOptions {
    fn default() -> Self {
        Self {
            config: PathBuf::from("."),
            format: Format::Human,
        }
    }
}

struct NamedRemoveOptions {
    config: PathBuf,
    name: String,
    format: Format,
}

fn parse_named_remove(args: &[String], usage_label: &str) -> Result<NamedRemoveOptions, Failure> {
    let mut config = PathBuf::from(".");
    let mut name = None;
    let mut format = Format::Human;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-C" | "--config" => {
                index += 1;
                config = PathBuf::from(next_value(args, index, "-C requires a path")?);
                index += 1;
            }
            "--format" => {
                index += 1;
                format = parse_format_arg(args.get(index))?;
                index += 1;
            }
            argument if argument.starts_with('-') => {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!("unknown {usage_label} option: {argument}"),
                ));
            }
            argument => {
                if name.is_some() {
                    return Err(Failure::new(
                        EXIT_USAGE,
                        format!("{usage_label} accepts only one name"),
                    ));
                }
                name = Some(argument.to_string());
                index += 1;
            }
        }
    }
    let name =
        name.ok_or_else(|| Failure::new(EXIT_USAGE, format!("{usage_label} requires a name")))?;
    Ok(NamedRemoveOptions {
        config,
        name,
        format,
    })
}

fn load_environment(path: &Path, format: Format, command: &str) -> Result<Environment, u8> {
    match validate_path(path) {
        Ok(environment) => Ok(environment),
        Err(issues) => {
            emit_validation_failure(format, command, path, &issues);
            Err(EXIT_INVALID)
        }
    }
}

fn next_value(args: &[String], index: usize, message: &str) -> Result<String, Failure> {
    args.get(index)
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_USAGE, message))
}

fn parse_format_arg(value: Option<&String>) -> Result<Format, Failure> {
    match value.map(String::as_str) {
        Some("human") => Ok(Format::Human),
        Some("json") => Ok(Format::Json),
        _ => Err(Failure::new(EXIT_USAGE, "--format requires human or json")),
    }
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| Format::Json)
        .unwrap_or(Format::Human)
}

fn emit_success(format: Format, command: &str, payload: Value, message: String) {
    match format {
        Format::Human => println!("{message}"),
        Format::Json => {
            let envelope = json!({
                "apiVersion": API_VERSION,
                "command": command,
                "status": "ok",
                "exit_code": 0,
                "summary": { "message": message },
            });
            let mut envelope = envelope;
            if let Some(object) = envelope.as_object_mut() {
                if let Some(payload_object) = payload.as_object() {
                    for (key, value) in payload_object {
                        object.insert(key.clone(), value.clone());
                    }
                }
            }
            println!("{}", envelope);
        }
    }
}

fn emit_validation_failure(format: Format, command: &str, path: &Path, issues: &[ValidationIssue]) {
    match format {
        Format::Human => {
            eprintln!("invalid: {}", path.display());
            for issue in issues {
                eprintln!("  {} [{}] {}", issue.path, issue.kind, issue.message);
            }
        }
        Format::Json => {
            let errors: Vec<Value> = issues
                .iter()
                .map(|issue| {
                    json!({
                        "path": issue.path,
                        "message": issue.message,
                        "kind": issue.kind,
                    })
                })
                .collect();
            eprintln!(
                "{}",
                json!({
                    "apiVersion": API_VERSION,
                    "command": command,
                    "status": "fail",
                    "exit_code": EXIT_INVALID,
                    "summary": {
                        "message": "hypernetwork/v1 config is invalid",
                        "errors": issues.len(),
                    },
                    "config": { "path": path },
                    "errors": errors,
                })
            );
        }
    }
}

fn emit_failure(format: Format, command: &str, failure: &Failure) {
    match format {
        Format::Human => eprintln!("{}", failure.message),
        Format::Json => eprintln!(
            "{}",
            json!({
                "apiVersion": API_VERSION,
                "command": command,
                "status": "fail",
                "exit_code": failure.code,
                "error": { "message": failure.message },
            })
        ),
    }
}

fn usage() -> &'static str {
    "usage: vzctl stack init [DIR] --name <project> [--cidr CIDR] [--force] [-C path] [--format human|json]"
}

fn vm_usage() -> &'static str {
    "usage: vzctl stack vm add <name> [-C path] [--from image-key|pull-alias] [--network net] [--ip addr] [--disk size] [--cpus N] [--memory size] [--role router|docker] [--cloud-init path] [--format human|json]\n       vzctl stack vm remove <name> [-C path] [--format human|json]"
}

fn net_usage() -> &'static str {
    "usage: vzctl stack net add <name> --cidr CIDR [-C path] [--mode shared|host] [--backend vmnet|docker] [--nat-egress|--no-nat-egress] [--format human|json]\n       vzctl stack net remove <name> [-C path] [--format human|json]"
}

fn volume_usage() -> &'static str {
    "usage: vzctl stack volume add <name> <path> [-C path] [--format human|json]\n       vzctl stack volume remove <name> [-C path] [--format human|json]"
}

fn mount_usage() -> &'static str {
    "usage: vzctl stack mount add <vm> --source <volume> --target <path> [--read-only] [-C path] [--format human|json]\n       vzctl stack mount remove <vm> --target <path> [-C path] [--format human|json]"
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
