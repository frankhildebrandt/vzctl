//! Per-VM identity and cloud-init seed construction.

use crate::*;

pub(crate) fn new_vm_identity(
    vm_id: &str,
    nic_count: usize,
) -> Result<VmIdentity, VmCreateFailure> {
    let mut random = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut random))
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot generate VM identity: {error}"),
            )
        })?;

    random[6] = (random[6] & 0x0f) | 0x40;
    random[8] = (random[8] & 0x3f) | 0x80;
    let instance_id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        random[0], random[1], random[2], random[3],
        random[4], random[5], random[6], random[7],
        random[8], random[9], random[10], random[11],
        random[12], random[13], random[14], random[15],
    );
    let count = nic_count.max(1);
    let mut mac_addresses = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 17 + (index % 3) * 5;
        // Refresh entropy for additional NICs beyond the first buffer window.
        let mut bytes = [
            random[offset % 32],
            random[(offset + 1) % 32],
            random[(offset + 2) % 32],
            random[(offset + 3) % 32],
            random[(offset + 4) % 32],
        ];
        if index > 0 {
            File::open("/dev/urandom")
                .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
                .map_err(|error| {
                    VmCreateFailure::new(
                        EXIT_VM_DISK_PREP_FAILED,
                        format!("cannot generate NIC MAC: {error}"),
                    )
                })?;
        }
        mac_addresses.push(format!(
            "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]
        ));
    }
    let fqdn = cloud_init_fqdn(vm_id);
    let hostname = fqdn.split('.').next().unwrap_or("vm").to_string();
    Ok(VmIdentity {
        instance_id,
        hostname,
        fqdn,
        mac_addresses,
    })
}

pub(crate) fn cloud_init_fqdn(vm_id: &str) -> String {
    let normalized = vm_id_basename(vm_id)
        .to_ascii_lowercase()
        .replace('_', "-")
        .split('.')
        .filter(|label| !label.is_empty())
        .map(|label| label.trim_matches('-'))
        .filter(|label| !label.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    if normalized.is_empty() {
        "vm".to_string()
    } else {
        normalized
    }
}

pub(crate) fn prepare_cloud_init_seed(
    backend: &dyn VmDiskBackend,
    bundle_path: &Path,
    cidata_path: &Path,
    agent_token_path: &Path,
    identity: &VmIdentity,
    roles: &[String],
    networks: &[network::VmNetworkSelection],
    docker_bip: Option<&str>,
    root_password: Option<&str>,
    cloud_init: Option<&Path>,
    project: Option<&str>,
) -> Result<CloudInitSummary, VmCreateFailure> {
    let mut token_bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut token_bytes))
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot generate guest-agent token: {error}"),
            )
        })?;
    let token = base64url_unpadded(&token_bytes);
    write_private_file(agent_token_path, format!("{token}\n").as_bytes())?;

    let seed_directory = bundle_path.join(".cidata.seed");
    fs::create_dir(&seed_directory).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot create cloud-init staging directory {}: {error}",
                seed_directory.display()
            ),
        )
    })?;
    fs::set_permissions(&seed_directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot protect cloud-init staging directory {}: {error}",
                seed_directory.display()
            ),
        )
    })?;

    let meta_data = format!(
        "instance-id: {}\nlocal-hostname: {}\n",
        identity.instance_id, identity.hostname
    );
    let network_config = render_cloud_init_network_config(identity, networks);

    let mut system = serde_yaml::Mapping::new();
    system.insert(
        serde_yaml::Value::String("preserve_hostname".into()),
        serde_yaml::Value::Bool(false),
    );
    system.insert(
        serde_yaml::Value::String("hostname".into()),
        serde_yaml::Value::String(identity.hostname.clone()),
    );
    system.insert(
        serde_yaml::Value::String("fqdn".into()),
        serde_yaml::Value::String(identity.fqdn.clone()),
    );
    system.insert(
        serde_yaml::Value::String("prefer_fqdn_over_hostname".into()),
        serde_yaml::Value::Bool(true),
    );
    system.insert(
        serde_yaml::Value::String("manage_etc_hosts".into()),
        serde_yaml::Value::Bool(true),
    );
    system.insert(
        serde_yaml::Value::String("ssh_deletekeys".into()),
        serde_yaml::Value::Bool(true),
    );
    system.insert(
        serde_yaml::Value::String("ssh_genkeytypes".into()),
        serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("ed25519".into()),
            serde_yaml::Value::String("rsa".into()),
        ]),
    );
    system.insert(
        serde_yaml::Value::String("growpart".into()),
        serde_yaml::Value::Mapping({
            let mut growpart = serde_yaml::Mapping::new();
            growpart.insert(
                serde_yaml::Value::String("mode".into()),
                serde_yaml::Value::String("auto".into()),
            );
            growpart.insert(
                serde_yaml::Value::String("devices".into()),
                serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("/".into())]),
            );
            growpart
        }),
    );
    system.insert(
        serde_yaml::Value::String("resize_rootfs".into()),
        serde_yaml::Value::Bool(true),
    );
    if let Some(password) = root_password {
        system.insert(
            serde_yaml::Value::String("disable_root".into()),
            serde_yaml::Value::Bool(false),
        );
        system.insert(
            serde_yaml::Value::String("ssh_pwauth".into()),
            serde_yaml::Value::Bool(true),
        );
        let mut user = serde_yaml::Mapping::new();
        user.insert(
            serde_yaml::Value::String("name".into()),
            serde_yaml::Value::String("root".into()),
        );
        user.insert(
            serde_yaml::Value::String("password".into()),
            serde_yaml::Value::String(password.to_string()),
        );
        user.insert(
            serde_yaml::Value::String("type".into()),
            serde_yaml::Value::String("text".into()),
        );
        let mut chpasswd = serde_yaml::Mapping::new();
        chpasswd.insert(
            serde_yaml::Value::String("expire".into()),
            serde_yaml::Value::Bool(false),
        );
        chpasswd.insert(
            serde_yaml::Value::String("users".into()),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(user)]),
        );
        system.insert(
            serde_yaml::Value::String("chpasswd".into()),
            serde_yaml::Value::Mapping(chpasswd),
        );
    }

    let mut write_files = vec![serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/var/lib/vzctl/agent.token".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("vzctl-agent:vzctl-agent".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0600".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(format!("{token}\n")),
        );
        file
    })];
    append_virtiofs_bind_files(&mut write_files);
    append_router_apply_files(&mut write_files);
    append_ca_inject_files(&mut write_files);
    let mut runcmd = Vec::new();
    append_agent_privilege_files(&mut write_files, &mut runcmd);
    if let Ok(ca_files) = crate::certs::nocloud_ca_write_files(&state_dir()) {
        for file in ca_files {
            let path = file["path"].as_str().unwrap_or_default();
            let content = file["content"].as_str().unwrap_or_default();
            let permissions = file["permissions"].as_str().unwrap_or("0644");
            write_files.push(serde_yaml::Value::Mapping({
                let mut map = serde_yaml::Mapping::new();
                map.insert(
                    serde_yaml::Value::String("path".into()),
                    serde_yaml::Value::String(path.into()),
                );
                map.insert(
                    serde_yaml::Value::String("permissions".into()),
                    serde_yaml::Value::String(permissions.into()),
                );
                map.insert(
                    serde_yaml::Value::String("content".into()),
                    serde_yaml::Value::String(content.into()),
                );
                map
            }));
        }
        append_ca_trust_update_command(&mut runcmd);
    }

    if roles.iter().any(|role| role == "router") {
        // Packages are installed before cloud-final runcmd. bootcmd protects
        // earlier starts; service guards also cover package-triggered Docker
        // startup before cloud-final has enabled forwarding.
        system.insert(
            serde_yaml::Value::String("packages".into()),
            serde_yaml::to_value(json!(["nftables", "python3"])).expect("static package list"),
        );
        system.insert(
            serde_yaml::Value::String("bootcmd".into()),
            serde_yaml::to_value(json!([["sh", "-ec",
                "if command -v nft >/dev/null 2>&1; then nft list table inet vzctl >/dev/null 2>&1 || printf '%s\\n' 'table inet vzctl { chain forward { type filter hook forward priority 0; policy drop; } }' | nft -f -; else sysctl -q -w net.ipv4.ip_forward=0; fi"
            ]])).expect("static bootstrap command"),
        );
        for service in ["vzctl-agent", "docker"] {
            if service == "docker" && !roles.iter().any(|role| role == "docker") {
                continue;
            }
            write_files.push(serde_yaml::to_value(json!({
                "path": format!("/etc/systemd/system/{service}.service.d/20-vzctl-router-bootstrap.conf"),
                "owner": "root:root",
                "permissions": "0644",
                "content": "# Managed by vzctl router-apply\n[Unit]\nAfter=systemd-sysctl.service nftables.service\n\n[Service]\nExecStartPre=+/usr/local/lib/vzctl/router-apply --bootstrap\n",
            })).expect("static router service guard"));
        }
        write_files.push(serde_yaml::Value::Mapping({
            let mut file = serde_yaml::Mapping::new();
            file.insert(
                serde_yaml::Value::String("path".into()),
                serde_yaml::Value::String("/etc/sysctl.d/90-vzctl-router.conf".into()),
            );
            file.insert(
                serde_yaml::Value::String("owner".into()),
                serde_yaml::Value::String("root:root".into()),
            );
            file.insert(
                serde_yaml::Value::String("permissions".into()),
                serde_yaml::Value::String("0644".into()),
            );
            file.insert(
                serde_yaml::Value::String("content".into()),
                serde_yaml::Value::String("net.ipv4.ip_forward=0\n".into()),
            );
            file
        }));
        runcmd.insert(
            0,
            serde_yaml::to_value(json!([
                "sh",
                "-ec",
                "sysctl --system && /usr/local/lib/vzctl/router-apply --bootstrap"
            ]))
            .expect("static router bootstrap command"),
        );
    }

    if roles.iter().any(|role| role == "docker") {
        let project = project.unwrap_or("default");
        let (_, pubkey) = docker::ensure_ssh_keypair(&state_dir(), project)
            .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
        let include_engine = cloud_init.is_none();
        let docker_cfg = docker::docker_role_cloud_config(&pubkey, include_engine, docker_bip);
        let system_value = serde_yaml::Value::Mapping(system.clone());
        let mut merged = docker::merge_cloud_config(system_value, Some(docker_cfg));
        if let Some(path) = cloud_init {
            let user = docker::load_user_cloud_init(path)
                .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
            merged = docker::merge_cloud_config(merged, Some(user));
        }
        if let Some(bip) = docker_bip {
            // User cloud-init may omit daemon.json; ensure bip after merge.
            merged = docker::ensure_docker_daemon_bip(merged, bip);
        }
        if let serde_yaml::Value::Mapping(ref mut map) = merged {
            let existing_files = map
                .remove(serde_yaml::Value::String("write_files".into()))
                .and_then(|value| match value {
                    serde_yaml::Value::Sequence(items) => Some(items),
                    _ => None,
                })
                .unwrap_or_default();
            let mut combined = write_files.clone();
            combined.extend(existing_files);
            map.insert(
                serde_yaml::Value::String("write_files".into()),
                serde_yaml::Value::Sequence(combined),
            );
            let existing_runcmd = map
                .remove(serde_yaml::Value::String("runcmd".into()))
                .and_then(|value| match value {
                    serde_yaml::Value::Sequence(items) => Some(items),
                    _ => None,
                })
                .unwrap_or_default();
            let mut combined_cmd = runcmd.clone();
            combined_cmd.extend(existing_runcmd);
            if !combined_cmd.is_empty() {
                map.insert(
                    serde_yaml::Value::String("runcmd".into()),
                    serde_yaml::Value::Sequence(combined_cmd),
                );
            }
        }
        let summary = summarize_cloud_init(&merged, roles, cloud_init.is_some());
        let user_data = docker::render_user_data(&merged)
            .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
        write_cloud_init_iso(
            backend,
            &seed_directory,
            cidata_path,
            &meta_data,
            &network_config,
            &user_data,
        )?;
        return Ok(summary);
    }

    system.insert(
        serde_yaml::Value::String("write_files".into()),
        serde_yaml::Value::Sequence(write_files),
    );
    if !runcmd.is_empty() {
        system.insert(
            serde_yaml::Value::String("runcmd".into()),
            serde_yaml::Value::Sequence(runcmd),
        );
    }
    let mut merged = serde_yaml::Value::Mapping(system);
    if let Some(path) = cloud_init {
        let user = docker::load_user_cloud_init(path)
            .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
        merged = docker::merge_cloud_config(merged, Some(user));
    }
    let summary = summarize_cloud_init(&merged, roles, cloud_init.is_some());
    let user_data = docker::render_user_data(&merged)
        .map_err(|error| VmCreateFailure::new(EXIT_VM_DISK_PREP_FAILED, error))?;
    write_cloud_init_iso(
        backend,
        &seed_directory,
        cidata_path,
        &meta_data,
        &network_config,
        &user_data,
    )?;
    Ok(summary)
}

pub(crate) fn summarize_cloud_init(
    merged: &serde_yaml::Value,
    roles: &[String],
    custom: bool,
) -> CloudInitSummary {
    let sequence = |key: &str| {
        merged
            .as_mapping()
            .and_then(|map| map.get(serde_yaml::Value::String(key.to_string())))
            .and_then(serde_yaml::Value::as_sequence)
    };
    let packages = sequence("packages")
        .into_iter()
        .flatten()
        .filter_map(serde_yaml::Value::as_str)
        .map(safe_cloud_init_label)
        .collect();
    let write_files = sequence("write_files")
        .into_iter()
        .flatten()
        .filter_map(serde_yaml::Value::as_mapping)
        .filter_map(|file| file.get(serde_yaml::Value::String("path".to_string())))
        .filter_map(serde_yaml::Value::as_str)
        .map(safe_cloud_init_label)
        .collect();
    let commands = sequence("bootcmd").map(Vec::len).unwrap_or(0)
        + sequence("runcmd").map(Vec::len).unwrap_or(0);
    let users = sequence("users").map(Vec::len).unwrap_or(0);
    CloudInitSummary {
        datasource: "nocloud",
        custom,
        roles: roles.to_vec(),
        packages,
        write_files,
        commands,
        users,
    }
}

pub(crate) fn safe_cloud_init_label(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if [
        "password",
        "passwd",
        "token",
        "secret",
        "authorization",
        "private-key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        "[redacted]".to_string()
    } else {
        value.chars().take(160).collect()
    }
}

pub(crate) fn write_cloud_init_iso(
    backend: &dyn VmDiskBackend,
    seed_directory: &Path,
    cidata_path: &Path,
    meta_data: &str,
    network_config: &str,
    user_data: &str,
) -> Result<(), VmCreateFailure> {
    let seed_result = (|| {
        write_private_file(&seed_directory.join("meta-data"), meta_data.as_bytes())?;
        write_private_file(
            &seed_directory.join("network-config"),
            network_config.as_bytes(),
        )?;
        write_private_file(&seed_directory.join("user-data"), user_data.as_bytes())?;
        backend
            .create_cloud_init_iso(seed_directory, cidata_path)
            .map_err(|error| {
                VmCreateFailure::new(
                    EXIT_VM_DISK_PREP_FAILED,
                    format!(
                        "cannot create NoCloud seed {}: {error}",
                        cidata_path.display()
                    ),
                )
            })?;
        fs::set_permissions(cidata_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!(
                    "cannot protect NoCloud seed {}: {error}",
                    cidata_path.display()
                ),
            )
        })
    })();
    let cleanup_result = fs::remove_dir_all(seed_directory);
    seed_result?;
    cleanup_result.map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!(
                "cannot remove cloud-init staging directory {}: {error}",
                seed_directory.display()
            ),
        )
    })?;
    Ok(())
}

pub(crate) fn append_virtiofs_bind_files(write_files: &mut Vec<serde_yaml::Value>) {
    let script = guest_utils::VIRTIOFS_BIND_SCRIPT;
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/usr/local/lib/vzctl/virtiofs-bind".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0755".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(script.to_string()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-virtiofs".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(
                "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/virtiofs-bind\n".into(),
            ),
        );
        file
    }));
}

pub(crate) fn append_router_apply_files(write_files: &mut Vec<serde_yaml::Value>) {
    let script = guest_utils::ROUTER_APPLY_SCRIPT;
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/usr/local/lib/vzctl/router-apply".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0755".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(script.to_string()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-router".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(
                "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/router-apply\n".into(),
            ),
        );
        file
    }));
}

pub(crate) fn append_ca_inject_files(write_files: &mut Vec<serde_yaml::Value>) {
    let script = guest_utils::CA_INJECT_SCRIPT;
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/usr/local/lib/vzctl/ca-inject".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0755".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(script.to_string()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-ca".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(
                "vzctl-agent ALL=(root) NOPASSWD: /usr/local/lib/vzctl/ca-inject\n".into(),
            ),
        );
        file
    }));
}

pub(crate) fn append_ca_trust_update_command(runcmd: &mut Vec<serde_yaml::Value>) {
    runcmd.push(serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::String("sh".into()),
        serde_yaml::Value::String("-c".into()),
        serde_yaml::Value::String(
            "/usr/sbin/update-ca-certificates && openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt /usr/local/share/ca-certificates/vzctl-local.crt"
                .into(),
        ),
    ]));
}

/// Passwordless sudo for `vzctl-agent` plus a unit refresh so clones pick up
/// NoNewPrivileges=no without rebaking the sealed base.
///
/// systemd loads `vzctl-agent.service` before cloud-config `write_files` runs,
/// so the first start can still inherit the sealed unit's `no_new_privs`.
/// `daemon-reload` + `restart` in runcmd (cloud-final) restarts under the
/// rewritten unit; later boots already read the on-disk unit.
pub(crate) fn append_agent_privilege_files(
    write_files: &mut Vec<serde_yaml::Value>,
    runcmd: &mut Vec<serde_yaml::Value>,
) {
    let unit = guest_utils::AGENT_SYSTEMD_UNIT;
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/sudoers.d/vzctl-agent".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0440".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String("vzctl-agent ALL=(ALL) NOPASSWD:ALL\n".into()),
        );
        file
    }));
    write_files.push(serde_yaml::Value::Mapping({
        let mut file = serde_yaml::Mapping::new();
        file.insert(
            serde_yaml::Value::String("path".into()),
            serde_yaml::Value::String("/etc/systemd/system/vzctl-agent.service".into()),
        );
        file.insert(
            serde_yaml::Value::String("owner".into()),
            serde_yaml::Value::String("root:root".into()),
        );
        file.insert(
            serde_yaml::Value::String("permissions".into()),
            serde_yaml::Value::String("0644".into()),
        );
        file.insert(
            serde_yaml::Value::String("content".into()),
            serde_yaml::Value::String(unit.to_string()),
        );
        file
    }));
    runcmd.push(serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::String("systemctl".into()),
        serde_yaml::Value::String("daemon-reload".into()),
    ]));
    runcmd.push(serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::String("systemctl".into()),
        serde_yaml::Value::String("restart".into()),
        serde_yaml::Value::String("vzctl-agent.service".into()),
    ]));
}

#[cfg(test)]
pub(crate) fn cloud_init_root_password_snippet(password: &str) -> String {
    let quoted = serde_yaml::to_string(password)
        .unwrap_or_else(|_| format!("\"{password}\""))
        .trim()
        .to_string();
    format!(
        "disable_root: false\nssh_pwauth: true\nchpasswd:\n  expire: false\n  users:\n    - name: root\n      password: {quoted}\n      type: text\n"
    )
}

pub(crate) fn render_cloud_init_network_config(
    identity: &VmIdentity,
    networks: &[network::VmNetworkSelection],
) -> String {
    if networks.is_empty() {
        return format!(
            "version: 2\nethernets:\n  nic0:\n    match:\n      macaddress: \"{}\"\n    set-name: enp0s1\n    dhcp4: true\n    dhcp6: false\n",
            identity.mac_addresses[0]
        );
    }
    let mut body = String::from("version: 2\nethernets:\n");
    for (index, network) in networks.iter().enumerate() {
        let mac = identity
            .mac_addresses
            .get(index)
            .unwrap_or(&identity.mac_addresses[0]);
        let iface = format!("enp0s{}", index + 1);
        let search = network
            .project
            .as_ref()
            .map_or_else(String::new, |project| {
                format!("      search:\n        - {project}.vz.test\n")
            });
        // Only the primary NIC gets a default route; extra NICs stay link-local.
        let routes = if index == 0 {
            format!(
                "    routes:\n      - to: default\n        via: {}\n        on-link: true\n",
                network.gateway
            )
        } else {
            String::new()
        };
        let nameservers = if index == 0 {
            format!(
                "    nameservers:\n      addresses:\n        - {}\n{}",
                network.dns, search
            )
        } else {
            String::new()
        };
        body.push_str(&format!(
            "  nic{index}:\n    match:\n      macaddress: \"{mac}\"\n    set-name: {iface}\n    dhcp4: false\n    dhcp6: false\n    addresses:\n      - {}/{}\n{routes}{nameservers}",
            network.ip, network.prefix
        ));
    }
    body
}

pub(crate) fn ensure_root_disk_min_bytes(
    path: &Path,
    min_bytes: u64,
) -> Result<(), VmCreateFailure> {
    let metadata = fs::metadata(path).map_err(|error| {
        VmCreateFailure::new(
            EXIT_VM_DISK_PREP_FAILED,
            format!("cannot stat root disk {}: {error}", path.display()),
        )
    })?;
    if metadata.len() >= min_bytes {
        return Ok(());
    }
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(min_bytes))
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!(
                    "cannot grow root disk {} to {min_bytes} bytes: {error}",
                    path.display()
                ),
            )
        })
}

pub(crate) fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), VmCreateFailure> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot create private file {}: {error}", path.display()),
            )
        })?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            VmCreateFailure::new(
                EXIT_VM_DISK_PREP_FAILED,
                format!("cannot write private file {}: {error}", path.display()),
            )
        })
}

pub(crate) fn base64url_unpadded(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        output.push(ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(value & 0x3f) as usize] as char);
        }
    }
    output
}

pub(crate) fn emit_vm_create_failure(format: OutputFormat, failure: &VmCreateFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": VM_CREATE_API_VERSION,
                "command": "vm.create",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}
