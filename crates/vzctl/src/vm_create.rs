//! VM creation, identity-independent disks and manifest contract.

use crate::*;

pub(crate) fn vm_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        None | Some("help") | Some("-h") | Some("--help") => {
            eprintln!(
                "usage: vzctl vm create|list|start|stop|restart|delete|inspect|logs|exec|transfer|attach|services|ps|mount|unmount|mounts|modify ..."
            );
            ExitCode::from(EXIT_USAGE)
        }
        Some("create") => vm_create_command(args.collect()),
        Some(command) => vm::command(
            std::iter::once(command.to_string()).chain(args),
            &supervisor_socket_path(),
        ),
    }
}

pub(crate) fn vm_create_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let mut options = match parse_vm_create_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_vm_create_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };

    let socket_path = supervisor_socket_path();
    let selection = match network::ensure_vm_network(
        &socket_path,
        &options.id,
        options.requested_network.as_deref(),
    ) {
        Ok(selection) => selection,
        Err(failure) => {
            let failure = VmCreateFailure::new(failure.code, failure.message);
            emit_vm_create_failure(options.format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    options.network = Some(selection.clone());
    // Prefer all DB attachments (attach_nets runs before create) so routers get
    // every NIC; fall back to the ensure selection for single-homed VMs.
    options.networks = match network::list_vm_attachments(&socket_path, &options.id) {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => vec![selection.clone()],
        Err(failure) => {
            let failure = VmCreateFailure::new(failure.code, failure.message);
            emit_vm_create_failure(options.format, &failure);
            return ExitCode::from(failure.code);
        }
    };

    match create_vm_bundle(&options, &NativeVmDiskBackend) {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => print_vm_create_human(&result),
                OutputFormat::Json => println!("{}", vm_create_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            network::rollback_vm_network(&socket_path, &selection, &options.id);
            emit_vm_create_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

pub(crate) fn parse_vm_create_options(
    args: impl Iterator<Item = String>,
) -> Result<VmCreateOptions, VmCreateFailure> {
    let mut id = None;
    let mut from = None;
    let mut disk_gib = None;
    let mut cpus = None;
    let mut memory_mib = None;
    let mut roles = Vec::new();
    let mut requested_network = None;
    let mut root_password = None;
    let mut cloud_init = None;
    let mut project = None;
    let mut mount_list = Vec::new();
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => {
                from = Some(args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--from requires a sealed image")
                })?);
            }
            "--disk" | "--data-disk" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--disk requires a GiB size")
                })?;
                let size = value.parse::<u64>().map_err(|_| {
                    VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        format!("invalid VM disk size: {value}"),
                    )
                })?;
                if size == 0 {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "VM disk size must be greater than zero",
                    ));
                }
                disk_gib = Some(size);
            }
            "--cpus" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--cpus requires a positive integer")
                })?;
                let parsed = value.parse::<u32>().map_err(|_| {
                    VmCreateFailure::new(EXIT_INVALID_INPUT, format!("invalid --cpus: {value}"))
                })?;
                if parsed == 0 {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "--cpus must be greater than zero",
                    ));
                }
                cpus = Some(parsed);
            }
            "--memory" => {
                let value = args
                    .next()
                    .ok_or_else(|| VmCreateFailure::new(EXIT_USAGE, "--memory requires a size"))?;
                memory_mib = Some(
                    parse_memory_mib(&value)
                        .map_err(|message| VmCreateFailure::new(EXIT_INVALID_INPUT, message))?,
                );
            }
            "--role" => {
                let role = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--role requires a role name")
                })?;
                if role != "router" && role != "docker" {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        format!("unsupported VM role: {role}"),
                    ));
                }
                if !roles.contains(&role) {
                    roles.push(role);
                }
            }
            "--cloud-init" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--cloud-init requires a path")
                })?;
                cloud_init = Some(PathBuf::from(value));
            }
            "--project" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--project requires a project name")
                })?;
                if value.is_empty() {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "--project must not be empty",
                    ));
                }
                project = Some(value);
            }
            "--network" => {
                requested_network = Some(args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--network requires a network name")
                })?);
            }
            "--mount" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(
                        EXIT_USAGE,
                        "--mount requires tag=…,source=…,target=…[,ro]",
                    )
                })?;
                let mount = mounts::parse_mount_flag(&value)
                    .map_err(|message| VmCreateFailure::new(EXIT_INVALID_INPUT, message))?;
                if mount_list.iter().any(|existing: &mounts::ResolvedMount| {
                    existing.name == mount.name || existing.target == mount.target
                }) {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        format!(
                            "duplicate mount name or target: {} → {}",
                            mount.name, mount.target
                        ),
                    ));
                }
                mount_list.push(mount);
            }
            "--root-password" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--root-password requires a password")
                })?;
                if value.is_empty() {
                    return Err(VmCreateFailure::new(
                        EXIT_INVALID_INPUT,
                        "--root-password must not be empty",
                    ));
                }
                root_password = Some(value);
            }
            "--format" => {
                let value = args.next().ok_or_else(|| {
                    VmCreateFailure::new(EXIT_USAGE, "--format requires human or json")
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(VmCreateFailure::new(
                            EXIT_USAGE,
                            format!("unsupported vm create format: {value}"),
                        ))
                    }
                };
            }
            "-h" | "--help" => {
                return Err(VmCreateFailure::new(
                    EXIT_USAGE,
                    "usage: vzctl vm create <id> --from <sealed> --disk <GiB> \
                     [--cpus N] [--memory <SIZE>] [--network <name>] [--role router|docker] \
                     [--mount tag=…,source=…,target=…[,ro]] [--cloud-init PATH] [--project P] \
                     [--root-password <secret>] [--format human|json]",
                ))
            }
            _ if arg.starts_with('-') => {
                return Err(VmCreateFailure::new(
                    EXIT_USAGE,
                    format!("unknown vm create option: {arg}"),
                ))
            }
            _ if id.is_none() => id = Some(arg),
            _ => {
                return Err(VmCreateFailure::new(
                    EXIT_USAGE,
                    format!("unexpected vm create argument: {arg}"),
                ))
            }
        }
    }

    let id = id.ok_or_else(|| {
        VmCreateFailure::new(
            EXIT_USAGE,
            "usage: vzctl vm create <id> --from <sealed> --disk <GiB> \
             [--cpus N] [--memory <SIZE>] [--network <name>] [--role router|docker] \
             [--mount tag=…,source=…,target=…[,ro]] [--cloud-init PATH] [--project P] \
             [--root-password <secret>] [--format human|json]",
        )
    })?;
    if !valid_vm_id(&id) {
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            "vm id must be a flat label (1-63) or project/vm (segments alphanumeric/._-, total ≤127)",
        ));
    }
    let from =
        from.ok_or_else(|| VmCreateFailure::new(EXIT_USAGE, "vm create requires --from <sealed>"))?;
    let disk_gib = disk_gib
        .ok_or_else(|| VmCreateFailure::new(EXIT_USAGE, "vm create requires --disk <GiB>"))?;
    if roles.iter().any(|role| role == "docker") && project.is_none() {
        project = Some("default".to_string());
    }
    let id = resolve_create_vm_id(&id, project.as_deref())
        .map_err(|message| VmCreateFailure::new(EXIT_INVALID_INPUT, message))?;

    Ok(VmCreateOptions {
        id,
        from,
        disk_gib,
        cpus: cpus.unwrap_or(DEFAULT_VM_CPUS),
        memory_mib: memory_mib.unwrap_or(DEFAULT_VM_MEMORY_MIB),
        roles,
        requested_network,
        network: None,
        networks: Vec::new(),
        root_password,
        cloud_init,
        project,
        mounts: mount_list,
        format,
    })
}

pub(crate) fn parse_memory_mib(value: &str) -> Result<u64, String> {
    if value.is_empty() {
        return Err("invalid --memory: empty".to_string());
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        let mib = value
            .parse::<u64>()
            .map_err(|_| format!("invalid --memory: {value}"))?;
        if mib < 256 {
            return Err(format!(
                "--memory must be at least 256 MiB; bare values are MiB (use {value}Gi for {value} GiB)"
            ));
        }
        return Ok(mib);
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| format!("invalid --memory: {value}"))?;
    let number = value[..split]
        .parse::<u64>()
        .map_err(|_| format!("invalid --memory: {value}"))?;
    if number == 0 {
        return Err("--memory must be greater than zero".to_string());
    }
    let unit = value[split..].to_ascii_lowercase();
    let mib = match unit.as_str() {
        "m" | "mb" | "mi" | "mib" => Ok(number),
        "g" | "gb" | "gi" | "gib" => number
            .checked_mul(1024)
            .ok_or_else(|| "--memory is too large".to_string()),
        "t" | "tb" | "ti" | "tib" => number
            .checked_mul(1024 * 1024)
            .ok_or_else(|| "--memory is too large".to_string()),
        _ => Err(format!(
            "--memory must use MiB/GiB/TiB or a bare MiB integer: {value}"
        )),
    }?;
    if mib < 256 {
        return Err("--memory must be at least 256 MiB".to_string());
    }
    Ok(mib)
}

pub(crate) fn valid_vm_id_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 63
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && segment.as_bytes()[0].is_ascii_alphanumeric()
}

pub(crate) fn valid_vm_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 127 {
        return false;
    }
    match id.split_once('/') {
        None => valid_vm_id_segment(id),
        Some((project, vm)) => {
            !project.contains('/')
                && !vm.contains('/')
                && valid_vm_id_segment(project)
                && valid_vm_id_segment(vm)
        }
    }
}

pub(crate) fn runtime_vm_id(project: &str, name: &str) -> String {
    format!("{project}/{name}")
}

pub(crate) fn vm_id_basename(id: &str) -> &str {
    id.rsplit_once('/').map(|(_, name)| name).unwrap_or(id)
}

pub(crate) fn resolve_create_vm_id(id: &str, project: Option<&str>) -> Result<String, String> {
    let Some(project) = project.filter(|value| !value.is_empty()) else {
        return Ok(id.to_string());
    };
    if !valid_vm_id_segment(project) {
        return Err(format!("invalid --project name: {project}"));
    }
    match id.split_once('/') {
        None => {
            let namespaced = runtime_vm_id(project, id);
            if !valid_vm_id(&namespaced) {
                return Err(format!("namespaced vm id is invalid: {namespaced}"));
            }
            Ok(namespaced)
        }
        Some((prefix, _)) => {
            if prefix == project {
                Ok(id.to_string())
            } else {
                Err(format!(
                    "vm id {id} does not match --project {project} (expected {project}/…)"
                ))
            }
        }
    }
}

pub(crate) fn create_vm_bundle(
    options: &VmCreateOptions,
    backend: &dyn VmDiskBackend,
) -> Result<VmCreateResult, VmCreateFailure> {
    create_vm_bundle_in_dirs(options, backend, &images_dir(), &state_dir().join("vms"))
}

pub(crate) fn create_vm_bundle_in_dirs(
    options: &VmCreateOptions,
    backend: &dyn VmDiskBackend,
    images_directory: &Path,
    vms_directory: &Path,
) -> Result<VmCreateResult, VmCreateFailure> {
    let source_path =
        resolve_vm_image_input(&options.from, images_directory).map_err(vm_failure_from_seal)?;
    let source_path = fs::canonicalize(&source_path).map_err(|error| {
        VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!("cannot resolve image {}: {error}", source_path.display()),
        )
    })?;
    let marker_path = seal_marker_path(images_directory, &source_path);
    if !marker_path.is_file() {
        return Err(VmCreateFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "sealed marker missing for {}; run `vzctl image seal` first",
                source_path.display()
            ),
        ));
    }
    let name = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image")
        .to_string();
    let source =
        read_existing_seal(&marker_path, &source_path, name).map_err(vm_failure_from_seal)?;
    if source.image_format != "raw" {
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "vm create requires a sealed raw image; {} is {}",
                source.source_path.display(),
                source.image_format
            ),
        ));
    }

    let size_bytes = options
        .disk_gib
        .checked_mul(1024 * 1024 * 1024)
        .ok_or_else(|| {
            VmCreateFailure::new(EXIT_INVALID_INPUT, "VM disk size exceeds u64 bytes")
        })?;
    let base_bytes = fs::metadata(&source.source_path)
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!(
                    "cannot stat sealed base {}: {error}",
                    source.source_path.display()
                ),
            )
        })?
        .len();
    if size_bytes < base_bytes {
        let minimum_gib = base_bytes.div_ceil(1024 * 1024 * 1024);
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "VM disk must be at least {minimum_gib} GiB for sealed base {}",
                source.source_path.display()
            ),
        ));
    }
    fs::create_dir_all(vms_directory).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot create VM directory {}: {error}",
                vms_directory.display()
            ),
        )
    })?;
    let bundle_path = vms_directory.join(&options.id);
    if bundle_path.exists() {
        return Err(VmCreateFailure::new(
            EXIT_INVALID_INPUT,
            format!("VM bundle already exists: {}", bundle_path.display()),
        ));
    }
    fs::create_dir_all(&bundle_path).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!("cannot create VM bundle {}: {error}", bundle_path.display()),
        )
    })?;

    let root_disk_path = bundle_path.join("disk.raw");
    let cidata_path = bundle_path.join("cidata.iso");
    let agent_token_path = bundle_path.join("agent.token");
    let result = prepare_vm_disks(
        options,
        backend,
        source,
        bundle_path,
        root_disk_path,
        cidata_path,
        agent_token_path,
        size_bytes,
    );
    if result.is_err() {
        let _ = fs::remove_dir_all(vms_directory.join(&options.id));
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_vm_disks(
    options: &VmCreateOptions,
    backend: &dyn VmDiskBackend,
    source: ImageSealResult,
    bundle_path: PathBuf,
    root_disk_path: PathBuf,
    cidata_path: PathBuf,
    agent_token_path: PathBuf,
    size_bytes: u64,
) -> Result<VmCreateResult, VmCreateFailure> {
    let filesystem = backend
        .filesystem_type(&bundle_path)
        .unwrap_or_else(|| "unknown".to_string());
    if !filesystem.eq_ignore_ascii_case("apfs") {
        return Err(VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "VM storage must be on APFS so writable roots share sealed base blocks; {} is on {filesystem}",
                bundle_path.display()
            ),
        ));
    }
    backend
        .clone_linked(&source.source_path, &root_disk_path)
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!(
                    "clonefile failed for shared base {} -> {}: {error}",
                    source.source_path.display(),
                    root_disk_path.display()
                ),
            )
        })?;
    let clone_mode = CloneMode::Linked;
    // The configured VM disk size is the capacity of the writable root clone.
    // APFS keeps unchanged extents shared with the sealed base image, while the
    // sparse extension becomes usable through cloud-init growpart/resizefs.
    ensure_root_disk_min_bytes(&root_disk_path, size_bytes)?;

    let all_networks = if options.networks.is_empty() {
        options.network.iter().cloned().collect::<Vec<_>>()
    } else {
        options.networks.clone()
    };
    let nic_networks = all_networks
        .iter()
        .filter(|network| !network.is_docker_backend())
        .cloned()
        .collect::<Vec<_>>();
    let docker_bip = all_networks
        .iter()
        .find(|network| network.is_docker_backend())
        .map(|network| format!("{}/{}", network.ip, network.prefix));
    let nic_count = nic_networks.len().max(1);
    let identity = new_vm_identity(&options.id, nic_count)?;
    let cloud_init_summary = prepare_cloud_init_seed(
        backend,
        &bundle_path,
        &cidata_path,
        &agent_token_path,
        &identity,
        &options.roles,
        &nic_networks,
        docker_bip.as_deref(),
        options.root_password.as_deref(),
        options.cloud_init.as_deref(),
        options.project.as_deref(),
    )?;

    // Identity / manifest NICs must align with helper virtio NICs (vmnet only).
    // Docker-backend attachments are logical (bip) and must not occupy index 0
    // when sorted ahead of the real NIC (e.g. containers before lan).
    let result = VmCreateResult {
        id: options.id.clone(),
        bundle_path,
        source,
        root_disk_path,
        cidata_path,
        agent_token_path,
        cloud_init_summary,
        disk_gib: options.disk_gib,
        cpus: options.cpus,
        memory_mib: options.memory_mib,
        roles: options.roles.clone(),
        mounts: options.mounts.clone(),
        clone_mode,
        identity,
        network: nic_networks.first().cloned(),
        networks: nic_networks,
    };
    write_vm_manifest(&result)?;
    Ok(result)
}

pub(crate) fn vm_failure_from_seal(failure: SealFailure) -> VmCreateFailure {
    VmCreateFailure::new(failure.code, failure.message)
}

pub(crate) fn write_vm_manifest(result: &VmCreateResult) -> Result<(), VmCreateFailure> {
    let manifest_path = result.bundle_path.join("vm.json");
    let manifest = json!({
        "apiVersion": "vzctl.dev/vm-bundle/v1",
        "managed-by": "vzctl",
        "vm_id": result.id,
        "roles": result.roles,
        "mounts": result.mounts.iter().map(mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "resources": {
            "cpus": result.cpus,
            "memory_mib": result.memory_mib,
        },
        "base": {
            "path": result.source.source_path,
            "marker": result.source.marker_path,
            "read_only": true,
        },
        "disks": {
            "root": {
                "path": result.root_disk_path,
                "clone": result.clone_mode.as_str(),
                "size_gib": result.disk_gib,
                "sparse": true,
            },
        },
        "identity": {
            "instance_id": result.identity.instance_id,
            "hostname": result.identity.hostname,
            "fqdn": result.identity.fqdn,
            "nics": result.identity.mac_addresses.iter().enumerate().map(|(index, mac)| {
                let network = result.networks.get(index).or(result.network.as_ref());
                json!({
                    "index": index,
                    "mac": mac,
                    "address": network.map(|value| value.ip.as_str()).unwrap_or("dhcp"),
                    "network": network.map(|value| value.network.as_str()),
                    "cidr": network.map(|value| value.cidr.as_str()),
                })
            }).collect::<Vec<_>>(),
        },
        "cloud_init": {
            "seed": result.cidata_path,
            "agent_token": result.agent_token_path,
            "summary": result.cloud_init_summary,
        },
    });
    fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).expect("VM manifest serializes")
        ),
    )
    .map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot write VM manifest {}: {error}",
                manifest_path.display()
            ),
        )
    })
}

pub(crate) fn print_vm_create_human(result: &VmCreateResult) {
    println!("created VM bundle {}", result.id);
    println!("  bundle: {}", result.bundle_path.display());
    println!(
        "  resources: {} cpus, {} MiB memory",
        result.cpus, result.memory_mib
    );
    println!(
        "  root: {} ({} GiB sparse, shared-base {})",
        result.root_disk_path.display(),
        result.disk_gib,
        result.clone_mode.as_str()
    );
    println!(
        "  base: {} (read-only)",
        result.source.source_path.display()
    );
    println!("  hostname: {}", result.identity.fqdn);
    println!("  mac: {}", result.identity.mac_addresses.join(", "));
    if !result.roles.is_empty() {
        println!("  roles: {}", result.roles.join(", "));
    }
    for mount in &result.mounts {
        println!(
            "  mount: {} → {} ({})",
            mount.source.display(),
            mount.target,
            if mount.read_only { "ro" } else { "rw" }
        );
    }
    if let Some(network) = &result.network {
        println!(
            "  network: {} ({}/{}, gateway {}){}",
            network.network,
            network.ip,
            network.prefix,
            network.gateway,
            if network.automatic { " [default]" } else { "" }
        );
    }
    println!("  cloud-init: {}", result.cidata_path.display());
}

pub(crate) fn vm_create_json(result: &VmCreateResult) -> Value {
    json!({
        "apiVersion": VM_CREATE_API_VERSION,
        "command": "vm.create",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": "VM bundle created",
            "vm_id": result.id,
            "clone": result.clone_mode.as_str(),
            "warnings": 0,
        },
        "vm": {
            "id": result.id,
            "bundle": result.bundle_path,
            "managed-by": "vzctl",
            "roles": result.roles,
            "mounts": result.mounts.iter().map(mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
            "resources": {
                "cpus": result.cpus,
                "memory_mib": result.memory_mib,
            },
        },
        "network": result.network.as_ref().map(|network| {
            json!({
                "name": network.network,
                "cidr": network.cidr,
                "ip": network.ip,
                "prefix": network.prefix,
                "gateway": network.gateway,
                "dns": network.dns,
                "automatic": network.automatic,
            })
        }),
        "image": {
            "name": result.source.name,
            "path": result.source.source_path,
            "marker": result.source.marker_path,
            "read_only": true,
        },
        "disks": {
            "root": {
                "path": result.root_disk_path,
                "clone": result.clone_mode.as_str(),
                "size_gib": result.disk_gib,
                "sparse": true,
                "read_only": false,
            },
        },
        "identity": {
            "instance_id": result.identity.instance_id,
            "hostname": result.identity.hostname,
            "fqdn": result.identity.fqdn,
            "nics": result.identity.mac_addresses.iter().enumerate().map(|(index, mac)| {
                let network = result.networks.get(index).or(result.network.as_ref());
                json!({
                    "index": index,
                    "mac": mac,
                    "address": network.map(|value| value.ip.as_str()).unwrap_or("dhcp"),
                    "network": network.map(|value| value.network.as_str()),
                })
            }).collect::<Vec<_>>(),
        },
        "cloud_init": {
            "seed": result.cidata_path,
            "agent_token": result.agent_token_path,
            "summary": result.cloud_init_summary,
        },
        "warnings": [],
    })
}

impl VmDiskBackend for NativeVmDiskBackend {
    fn filesystem_type(&self, path: &Path) -> Option<String> {
        filesystem_type(path)
    }

    fn clone_linked(&self, source: &Path, destination: &Path) -> Result<(), io::Error> {
        clonefile_path(source, destination)?;
        let mut permissions = fs::metadata(destination)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(destination, permissions)
    }

    fn create_cloud_init_iso(
        &self,
        seed_directory: &Path,
        destination: &Path,
    ) -> Result<(), io::Error> {
        let output = Command::new("hdiutil")
            .args([
                "makehybrid",
                "-iso",
                "-joliet",
                "-default-volume-name",
                "cidata",
            ])
            .arg("-o")
            .arg(destination)
            .arg(seed_directory)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(command_text(&output)))
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn clonefile_path(source: &Path, destination: &Path) -> Result<(), io::Error> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn clonefile(source: *const c_char, destination: *const c_char, flags: c_int) -> c_int;
    }

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    if unsafe { clonefile(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn clonefile_path(_source: &Path, _destination: &Path) -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "clonefile is only available on macOS",
    ))
}
