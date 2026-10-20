use super::*;

#[test]
fn ca_nocloud_assets_include_privileged_installer_and_trust_update() {
    let mut write_files = Vec::new();
    append_ca_inject_files(&mut write_files);
    let files = serde_yaml::to_string(&write_files).unwrap();
    assert!(files.contains("/usr/local/lib/vzctl/ca-inject"));
    assert!(files.contains("/etc/sudoers.d/vzctl-ca"));
    assert!(files.contains("NOPASSWD: /usr/local/lib/vzctl/ca-inject"));

    let mut runcmd = Vec::new();
    append_ca_trust_update_command(&mut runcmd);
    let commands = serde_yaml::to_string(&runcmd).unwrap();
    assert!(commands.contains("/usr/sbin/update-ca-certificates"));
    assert!(commands.contains("openssl verify"));
}
use std::cell::RefCell;

#[test]
fn doctor_options_default_to_human_and_twenty_gib() {
    std::env::remove_var("VZCTL_DOCTOR_MIN_FREE_GIB");
    assert_eq!(
        parse_doctor_options(std::iter::empty()).unwrap(),
        DoctorOptions {
            format: OutputFormat::Human,
            min_free_gib: 20,
            stack: None,
        }
    );
}

#[test]
fn doctor_options_accept_stack_path() {
    let args = ["--stack", "/tmp/lab"].into_iter().map(str::to_string);
    let options = parse_doctor_options(args).unwrap();
    assert_eq!(
        options.stack.as_deref(),
        Some(std::path::Path::new("/tmp/lab"))
    );
}

#[test]
fn doctor_options_accept_json_and_disk_override() {
    let args = ["--format", "json", "--min-free-gib", "42"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_doctor_options(args).unwrap(),
        DoctorOptions {
            format: OutputFormat::Json,
            min_free_gib: 42,
            stack: None,
        }
    );
}

#[test]
fn version_json_matches_golden_contract() {
    let expected: Value =
        serde_json::from_str(include_str!("../tests/golden/version.json")).unwrap();
    assert_eq!(version_json("0.0.1"), expected);
}

#[test]
fn doctor_json_matches_golden_contract() {
    let checks = vec![
        Check::new(
            "host.macos",
            CheckStatus::Ok,
            "macOS 26 meets the baseline",
            json!({ "major": 26, "minimum_major": 26 }),
        ),
        Check::new(
            "codesign.vz-helper",
            CheckStatus::Warn,
            "vz-helper not found",
            json!({ "binary": "vz-helper", "found": false }),
        ),
    ];
    let expected: Value =
        serde_json::from_str(include_str!("../tests/golden/doctor.json")).unwrap();
    assert_eq!(doctor_json(&checks, 0), expected);
}

#[test]
fn supervisor_edge_remediation_includes_restart_action() {
    let remediation = supervisor_edge_remediation();
    assert_eq!(remediation["edge"]["action"], "restart");
    assert_eq!(remediation["edge"]["service"], "edge");
    assert_eq!(remediation["edge"]["label"], "com.vzctl.edge");
    assert_eq!(remediation["edge"]["cli"], "vzctl services restart edge");
}

#[test]
fn doctor_warns_for_external_network_state_but_fails_internal_dataplane() {
    for state in ["offline", "captive", "conflict", "degraded", "suspended"] {
        assert_eq!(
            resilience_doctor_status(state, true).map(|value| value.0),
            Some(CheckStatus::Warn)
        );
    }
    assert_eq!(
        resilience_doctor_status("degraded", false).map(|value| value.0),
        Some(CheckStatus::Fail)
    );
    assert_eq!(resilience_doctor_status("healthy", true), None);
}

#[test]
fn stable_exit_code_mapping_matches_cli_contract() {
    assert_eq!(EXIT_USAGE, 2);
    assert_eq!(EXIT_INVALID_INPUT, 3);
    assert_eq!(EXIT_INCOMPLETE_JOURNAL, 5);
    assert_eq!(EXIT_LEASE_HELD, 6);
    assert_eq!(EXIT_SUPERVISOR_UNHEALTHY, 10);
    assert_eq!(EXIT_HOST_UNSUPPORTED, 11);
    assert_eq!(EXIT_UNAVAILABLE, 12);
    assert_eq!(EXIT_IMAGE_CUSTOMIZE_FAILED, 13);
    assert_eq!(EXIT_IMAGE_INVARIANT_FAILED, 14);
    assert_eq!(EXIT_IMAGE_STATE_FAILED, 15);
    assert_eq!(EXIT_VM_DISK_PREP_FAILED, 16);
    assert_eq!(network::EXIT_NETWORK, 17);
    assert_eq!(route::EXIT_ROUTE, 18);
    assert_eq!(dns::EXIT_RESOLVER, 19);
    assert_eq!(dns::EXIT_DNS_QUERY, 20);
    assert_eq!(image::EXIT_IMAGE_NETWORK, 21);
    assert_eq!(image::EXIT_IMAGE_CHECKSUM, 22);
    assert_eq!(image::EXIT_IMAGE_ARCH, 23);
}

#[test]
fn image_pull_options_accept_alias_and_json_in_any_order() {
    let args = ["--format", "json", "ubuntu-latest"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_image_pull_options(args).unwrap(),
        ImagePullOptions {
            alias: "ubuntu-latest".to_string(),
            format: OutputFormat::Json,
        }
    );
}

#[test]
fn image_pull_json_exposes_digest_alias_and_unsealed_state() {
    let result = image::PullResult {
        requested_alias: "coreos-latest".to_string(),
        canonical_alias: "fedora-coreos-latest".to_string(),
        distribution: "Fedora CoreOS".to_string(),
        release: "stable".to_string(),
        source_url: "https://example.invalid/fcos.qcow2.xz".to_string(),
        source_format: "qcow2.xz".to_string(),
        source_algorithm: "sha256".to_string(),
        source_digest: "a".repeat(64),
        normalized_digest: "b".repeat(64),
        image_path: PathBuf::from("/images/objects/b.raw"),
        manifest_path: PathBuf::from("/images/aliases/coreos-latest.json"),
        unchanged: true,
        sealed: false,
        aliases: vec![
            "fedora-coreos-latest".to_string(),
            "coreos-latest".to_string(),
        ],
    };
    let value = image_pull_json(&result);
    assert_eq!(value["command"], "image.pull");
    assert_eq!(value["summary"]["change"], "unchanged");
    assert_eq!(value["image"]["canonical_alias"], "fedora-coreos-latest");
    assert_eq!(value["image"]["sealed"], false);
    assert_eq!(value["image"]["sha256"], "b".repeat(64));
}

#[test]
fn image_seal_options_accept_path_and_json_in_any_order() {
    let args = ["--format", "json", "--tag", "v1", "base.raw"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_image_seal_options(args).unwrap(),
        ImageSealOptions {
            input: "base.raw".to_string(),
            tag: Some("v1".to_string()),
            format: OutputFormat::Json,
            force: false,
        }
    );
}

#[test]
fn resolves_one_local_image_by_name() {
    let directory = test_directory("image-resolve");
    fs::create_dir_all(&directory).unwrap();
    let image = directory.join("ubuntu-base.qcow2");
    fs::write(&image, b"fake").unwrap();

    assert_eq!(
        resolve_image_input("ubuntu-base", &directory).unwrap(),
        image
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn image_cleanup_matches_guest_agent_base_pipeline() {
    assert!(IMAGE_CLEANUP_COMMANDS
        .iter()
        .any(|command| command.contains("cloud-init clean --logs")));
    assert!(IMAGE_CLEANUP_COMMANDS
        .iter()
        .any(|command| command.contains("truncate -s 0 /etc/machine-id")));
    assert!(IMAGE_CLEANUP_COMMANDS
        .iter()
        .any(|command| command.contains("/etc/ssh") && command.contains("ssh_host_*")));
    assert!(IMAGE_CLEANUP_COMMANDS
        .iter()
        .any(|command| command.contains("/var/lib/systemd/random-seed")));
    assert!(IMAGE_PRESERVATION_CHECKS
        .iter()
        .any(|command| command.contains("/usr/local/sbin/vzctl-agent")));
    assert!(IMAGE_PRESERVATION_CHECKS
        .iter()
        .any(|command| command.contains("vzctl-agent.service")));
    assert!(IMAGE_PRESERVATION_CHECKS
        .iter()
        .any(|command| command.contains("/etc/init.d/vzctl-agent")));
    assert!(IMAGE_PRESERVATION_CHECKS
        .iter()
        .any(|command| command.contains("image-metadata.json")));
}

#[test]
fn seal_writes_marker_locks_image_and_is_idempotent() {
    let directory = test_directory("image-seal");
    let images_directory = directory.join("images");
    fs::create_dir_all(&images_directory).unwrap();
    let image = directory.join("base.raw");
    fs::write(&image, b"fake image").unwrap();
    let options = ImageSealOptions {
        input: image.to_string_lossy().to_string(),
        tag: Some("v1".to_string()),
        format: OutputFormat::Json,
        force: false,
    };
    let backend = RecordingSealBackend::new("raw");

    let result = seal_image_in_dir(&options, &backend, &images_directory).unwrap();
    assert_eq!(
        backend.calls(),
        vec![
            "inspect",
            "preserved",
            "customize",
            "preserved",
            "clone-safe"
        ]
    );
    assert!(result.marker_path.is_file());
    assert_eq!(
        fs::metadata(&result.source_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o222,
        0
    );

    let unused_backend = RecordingSealBackend::new("raw");
    let repeated = seal_image_in_dir(&options, &unused_backend, &images_directory).unwrap();
    assert!(repeated.already_sealed);
    assert!(unused_backend.calls().is_empty());

    let mut writable = fs::metadata(&result.source_path).unwrap().permissions();
    writable.set_mode(0o600);
    fs::set_permissions(&result.source_path, writable).unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn image_seal_json_matches_golden_contract() {
    let result = ImageSealResult {
        name: "ubuntu-base".to_string(),
        source_path: PathBuf::from("/images/ubuntu-base.raw"),
        image_format: "raw".to_string(),
        marker_path: PathBuf::from("/images/ubuntu-base-abc.sealed.json"),
        already_sealed: false,
    };
    let expected: Value =
        serde_json::from_str(include_str!("../tests/golden/image-seal.json")).unwrap();
    assert_eq!(image_seal_json(&result), expected);
}

#[test]
fn vm_create_options_accept_required_flags_in_any_order() {
    let args = [
        "--data-disk",
        "64",
        "web-1",
        "--format",
        "json",
        "--role",
        "router",
        "--network",
        "lan",
        "--from",
        "ubuntu-base",
    ]
    .into_iter()
    .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(args).unwrap(),
        VmCreateOptions {
            id: "web-1".to_string(),
            from: "ubuntu-base".to_string(),
            disk_gib: 64,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: vec!["router".to_string()],
            requested_network: Some("lan".to_string()),
            network: None,
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: None,
            mounts: Vec::new(),
            format: OutputFormat::Json,
        }
    );
}

#[test]
fn vm_create_disk_flag_sets_root_capacity_without_extra_disk() {
    let args = ["web", "--from", "ubuntu-base", "--disk", "20"]
        .into_iter()
        .map(str::to_string);

    let options = parse_vm_create_options(args).unwrap();

    assert_eq!(options.disk_gib, 20);
}

#[test]
fn vm_create_rejects_unsafe_id_and_zero_data_disk() {
    let unsafe_id = ["../web", "--from", "base", "--data-disk", "1"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(unsafe_id).unwrap_err().code,
        EXIT_INVALID_INPUT
    );
    let zero_size = ["web", "--from", "base", "--data-disk", "0"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(zero_size).unwrap_err().code,
        EXIT_INVALID_INPUT
    );
}

#[test]
fn valid_vm_id_accepts_flat_and_project_slash_forms() {
    assert!(valid_vm_id("web"));
    assert!(valid_vm_id("edge-dmz/web"));
    assert!(!valid_vm_id("../web"));
    assert!(!valid_vm_id("a/b/c"));
    assert!(!valid_vm_id("/web"));
    assert!(!valid_vm_id("edge-dmz/"));
    assert!(!valid_vm_id(""));
}

#[test]
fn resolve_create_vm_id_namespaces_with_project() {
    assert_eq!(
        resolve_create_vm_id("web", None).unwrap(),
        "web".to_string()
    );
    assert_eq!(
        resolve_create_vm_id("web", Some("edge-dmz")).unwrap(),
        "edge-dmz/web".to_string()
    );
    assert_eq!(
        resolve_create_vm_id("edge-dmz/web", Some("edge-dmz")).unwrap(),
        "edge-dmz/web".to_string()
    );
    assert!(resolve_create_vm_id("other/web", Some("edge-dmz")).is_err());
}

#[test]
fn vm_create_with_project_prefixes_flat_id() {
    let args = [
        "web",
        "--from",
        "ubuntu-base",
        "--data-disk",
        "4",
        "--project",
        "edge-dmz",
    ]
    .into_iter()
    .map(str::to_string);
    let options = parse_vm_create_options(args).unwrap();
    assert_eq!(options.id, "edge-dmz/web");
    assert_eq!(options.project.as_deref(), Some("edge-dmz"));
}

#[test]
fn vm_create_rejects_project_prefix_mismatch() {
    let args = [
        "lab/web",
        "--from",
        "ubuntu-base",
        "--data-disk",
        "4",
        "--project",
        "edge-dmz",
    ]
    .into_iter()
    .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(args).unwrap_err().code,
        EXIT_INVALID_INPUT
    );
}

#[test]
fn docker_role_defaults_project_and_namespaces_id() {
    let args = [
        "web",
        "--from",
        "ubuntu-base",
        "--data-disk",
        "4",
        "--role",
        "docker",
    ]
    .into_iter()
    .map(str::to_string);
    let options = parse_vm_create_options(args).unwrap();
    assert_eq!(options.project.as_deref(), Some("default"));
    assert_eq!(options.id, "default/web");
}

#[test]
fn vm_create_accepts_root_password_flag() {
    let args = [
        "web",
        "--from",
        "ubuntu-base",
        "--data-disk",
        "4",
        "--root-password",
        "s3cret!",
    ]
    .into_iter()
    .map(str::to_string);
    let options = parse_vm_create_options(args).unwrap();
    assert_eq!(options.root_password.as_deref(), Some("s3cret!"));
    let snippet = cloud_init_root_password_snippet("s3cret!");
    assert!(snippet.contains("disable_root: false"));
    assert!(snippet.contains("ssh_pwauth: true"));
    assert!(snippet.contains("name: root"));
    assert!(snippet.contains("type: text"));
    assert!(snippet.contains("s3cret!"));
}

#[test]
fn vm_create_accepts_cpus_and_memory_flags() {
    let args = [
        "web",
        "--from",
        "ubuntu-base",
        "--data-disk",
        "4",
        "--cpus",
        "4",
        "--memory",
        "2Gi",
    ]
    .into_iter()
    .map(str::to_string);
    let options = parse_vm_create_options(args).unwrap();
    assert_eq!(options.cpus, 4);
    assert_eq!(options.memory_mib, 2048);
    assert_eq!(parse_memory_mib("2048").unwrap(), 2048);
    assert_eq!(parse_memory_mib("2G").unwrap(), 2048);
    assert_eq!(parse_memory_mib("2Gi").unwrap(), 2048);
}

#[test]
fn vm_create_rejects_invalid_cpus_and_memory() {
    let zero_cpus = ["web", "--from", "base", "--data-disk", "1", "--cpus", "0"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(zero_cpus).unwrap_err().code,
        EXIT_INVALID_INPUT
    );
    let bad_memory = ["web", "--from", "base", "--data-disk", "1", "--memory", "0"]
        .into_iter()
        .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(bad_memory).unwrap_err().code,
        EXIT_INVALID_INPUT
    );
    let junk_memory = [
        "web",
        "--from",
        "base",
        "--data-disk",
        "1",
        "--memory",
        "lots",
    ]
    .into_iter()
    .map(str::to_string);
    assert_eq!(
        parse_vm_create_options(junk_memory).unwrap_err().code,
        EXIT_INVALID_INPUT
    );
}

#[test]
fn vm_create_writes_resources_into_manifest() {
    let directory = std::env::temp_dir().join(format!(
        "vzctl-resources-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    fs::create_dir_all(&images_directory).unwrap();
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("apfs");
    let result = create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "web".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: 4,
            memory_mib: 2048,
            roles: Vec::new(),
            requested_network: None,
            network: None,
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: None,
            mounts: Vec::new(),
            format: OutputFormat::Json,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(result.bundle_path.join("vm.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["resources"]["cpus"], json!(4));
    assert_eq!(manifest["resources"]["memory_mib"], json!(2048));
    assert_eq!(manifest["mounts"], json!([]));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn vm_create_persists_mounts_in_manifest() {
    let directory = test_directory("vm-mounts-manifest");
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    let share = directory.join("share");
    fs::create_dir_all(&share).unwrap();
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("apfs");
    let result = create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "web".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: Vec::new(),
            requested_network: None,
            network: None,
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: None,
            mounts: vec![mounts::ResolvedMount {
                name: "app".to_string(),
                source: share.clone(),
                target: "/srv/app".to_string(),
                read_only: false,
            }],
            format: OutputFormat::Json,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(result.bundle_path.join("vm.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["mounts"][0]["name"], json!("app"));
    assert_eq!(manifest["mounts"][0]["target"], json!("/srv/app"));
    assert_eq!(
        manifest["mounts"][0]["source"].as_str().unwrap(),
        share.to_string_lossy()
    );
    let user_data = &backend.seeds()[0].2;
    assert!(user_data.contains("/usr/local/lib/vzctl/virtiofs-bind"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn vm_create_root_password_lands_in_user_data() {
    let directory = std::env::temp_dir().join(format!(
        "vzctl-rootpw-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    fs::create_dir_all(&images_directory).unwrap();
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("apfs");
    create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "web".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: Vec::new(),
            requested_network: None,
            network: None,
            networks: Vec::new(),
            root_password: Some("pass:word".to_string()),
            cloud_init: None,
            project: None,
            mounts: Vec::new(),
            format: OutputFormat::Json,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap();
    let user_data = &backend.seeds()[0].2;
    assert!(user_data.contains("disable_root: false"));
    assert!(user_data.contains("chpasswd:"));
    assert!(user_data.contains("name: root"));
    assert!(
        user_data.contains("pass:word")
            || user_data.contains("'pass:word'")
            || user_data.contains("\"pass:word\"")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn identity_helpers_produce_cloud_init_safe_values() {
    assert_eq!(cloud_init_fqdn("Web_01.Example"), "web-01.example");
    assert_eq!(cloud_init_fqdn("edge-dmz/Web_01"), "web-01");
    assert_eq!(cloud_init_fqdn("..."), "vm");
    assert_eq!(vm_id_basename("edge-dmz/web"), "web");
    assert_eq!(vm_id_basename("web"), "web");
    assert_eq!(base64url_unpadded(&[0xfb, 0xff, 0xef]), "-__v");
    assert_eq!(base64url_unpadded(&[0xff]), "_w");
}

#[test]
fn namespaced_vm_create_uses_nested_bundle_path() {
    let directory = test_directory("vm-namespaced-bundle");
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("apfs");
    let result = create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "edge-dmz/web".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: Vec::new(),
            requested_network: None,
            network: None,
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: Some("edge-dmz".to_string()),
            mounts: Vec::new(),
            format: OutputFormat::Json,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap();
    assert_eq!(result.id, "edge-dmz/web");
    assert_eq!(
        result.bundle_path,
        vms_directory.join("edge-dmz").join("web")
    );
    assert!(result.bundle_path.join("vm.json").is_file());
    assert_eq!(result.identity.hostname, "web");
    assert_eq!(result.identity.fqdn, "web");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn automatic_default_network_uses_only_hypervisor_dns_and_project_search() {
    let identity = VmIdentity {
        instance_id: "test".to_string(),
        hostname: "web".to_string(),
        fqdn: "web".to_string(),
        mac_addresses: vec!["02:12:34:56:78:9a".to_string()],
    };
    let network = network::VmNetworkSelection {
        network_id: "lan".to_string(),
        network: "lan".to_string(),
        cidr: "10.70.0.0/24".to_string(),
        ip: "10.70.0.10".to_string(),
        gateway: "10.70.0.0".to_string(),
        dns: "10.70.0.0".to_string(),
        project: Some("edge-dmz".to_string()),
        prefix: 24,
        automatic: true,
        created: true,
        backend: "vmnet".to_string(),
    };

    assert_eq!(
        render_cloud_init_network_config(&identity, std::slice::from_ref(&network)),
        concat!(
            "version: 2\n",
            "ethernets:\n",
            "  nic0:\n",
            "    match:\n",
            "      macaddress: \"02:12:34:56:78:9a\"\n",
            "    set-name: enp0s1\n",
            "    dhcp4: false\n",
            "    dhcp6: false\n",
            "    addresses:\n",
            "      - 10.70.0.10/24\n",
            "    routes:\n",
            "      - to: default\n",
            "        via: 10.70.0.0\n",
            "        on-link: true\n",
            "    nameservers:\n",
            "      addresses:\n",
            "        - 10.70.0.0\n",
            "      search:\n",
            "        - edge-dmz.vz.test\n",
        )
    );
}

#[test]
fn network_without_project_does_not_invent_a_dns_search_zone() {
    let identity = VmIdentity {
        instance_id: "test".to_string(),
        hostname: "web".to_string(),
        fqdn: "web".to_string(),
        mac_addresses: vec!["02:12:34:56:78:9a".to_string()],
    };
    let network = network::VmNetworkSelection {
        network_id: "lan".to_string(),
        network: "lan".to_string(),
        cidr: "10.70.0.0/24".to_string(),
        ip: "10.70.0.10".to_string(),
        gateway: "10.70.0.0".to_string(),
        dns: "10.70.0.0".to_string(),
        project: None,
        prefix: 24,
        automatic: true,
        created: true,
        backend: "vmnet".to_string(),
    };

    let config = render_cloud_init_network_config(&identity, std::slice::from_ref(&network));
    assert!(config.contains("addresses:\n        - 10.70.0.0"));
    assert!(!config.contains("search:"));
    assert!(!config.contains("nameserver 127.0.0.1"));
}

#[test]
fn two_linked_clones_share_base_blocks_and_expose_requested_root_capacity() {
    let directory = test_directory("vm-linked-clones");
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("apfs");
    let marker_path = fs::read_dir(&images_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let base_before = fs::read(&image).unwrap();
    let marker_before = fs::read(&marker_path).unwrap();

    let mut instance_ids = Vec::new();
    let mut mac_addresses = Vec::new();
    let mut tokens = Vec::new();
    for id in ["web-1", "web-2"] {
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: id.to_string(),
                from: image.to_string_lossy().to_string(),
                disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Json,
            },
            &backend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();
        assert_eq!(result.clone_mode, CloneMode::Linked);
        assert_eq!(
            fs::metadata(&result.root_disk_path).unwrap().len(),
            1024 * 1024 * 1024
        );
        assert!(!result.bundle_path.join("dataDisk.raw").exists());
        assert!(result.cidata_path.is_file());
        assert_eq!(
            fs::metadata(&result.cidata_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let token = fs::read_to_string(&result.agent_token_path).unwrap();
        assert_eq!(token.trim().len(), 43);
        assert!(!token.contains('='));
        instance_ids.push(result.identity.instance_id);
        mac_addresses.push(result.identity.mac_addresses[0].clone());
        tokens.push(token);
    }

    assert_ne!(instance_ids[0], instance_ids[1]);
    assert_ne!(mac_addresses[0], mac_addresses[1]);
    assert!(mac_addresses.iter().all(|mac| mac.starts_with("02:")));
    assert_ne!(tokens[0], tokens[1]);
    for (meta_data, network_config, user_data) in backend.seeds() {
        assert!(meta_data.contains("instance-id: "));
        assert!(network_config.contains("macaddress: \"02:"));
        assert!(network_config.contains("dhcp4: true"));
        assert!(user_data.contains("ssh_deletekeys: true"));
        assert!(user_data.contains("ed25519"));
        assert!(user_data.contains("rsa"));
        assert!(user_data.contains("ssh_genkeytypes"));
        assert!(user_data.contains("growpart:"));
        assert!(user_data.contains("resize_rootfs: true"));
    }
    assert_eq!(
        fs::metadata(&image).unwrap().permissions().mode() & 0o222,
        0
    );
    assert_eq!(fs::read(&image).unwrap(), base_before);
    assert_eq!(fs::read(&marker_path).unwrap(), marker_before);
    assert_eq!(backend.calls(), vec!["linked", "seed", "linked", "seed"]);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn non_apfs_refuses_to_copy_the_shared_base_image() {
    let directory = test_directory("vm-full-clone");
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("hfs");
    let failure = create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "web".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: Vec::new(),
            requested_network: None,
            network: None,
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: None,
            mounts: Vec::new(),
            format: OutputFormat::Human,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap_err();

    assert_eq!(failure.code, EXIT_VM_DISK_PREP_FAILED);
    assert!(failure.message.contains("APFS"));
    assert!(backend.calls().is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn router_role_persists_manifest_and_cloud_init_forwarding() {
    let directory = test_directory("vm-router-role");
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::new("apfs");
    let result = create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "router".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: vec!["router".to_string()],
            requested_network: Some("lan".to_string()),
            network: Some(network::VmNetworkSelection {
                network_id: "lan".to_string(),
                network: "lan".to_string(),
                cidr: "10.70.0.0/24".to_string(),
                ip: "10.70.0.10".to_string(),
                gateway: "10.70.0.0".to_string(),
                dns: "10.70.0.0".to_string(),
                project: Some("edge-dmz".to_string()),
                prefix: 24,
                automatic: false,
                created: true,
                backend: "vmnet".to_string(),
            }),
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: None,
            mounts: Vec::new(),
            format: OutputFormat::Json,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap();

    let manifest: Value =
        serde_json::from_slice(&fs::read(result.bundle_path.join("vm.json")).unwrap()).unwrap();
    assert_eq!(manifest["roles"], json!(["router"]));
    let user_data = &backend.seeds()[0].2;
    assert!(user_data.contains("/etc/sysctl.d/90-vzctl-router.conf"));
    assert!(user_data.contains("net.ipv4.ip_forward=0"));
    let cloud: serde_yaml::Value = serde_yaml::from_str(user_data).unwrap();
    let runcmd = cloud["runcmd"].as_sequence().unwrap();
    assert!(runcmd[0][2]
        .as_str()
        .unwrap()
        .starts_with("sysctl --system && /usr/local/lib/vzctl/router-apply --bootstrap"));
    assert!(cloud["packages"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some("nftables")));
    assert!(cloud["packages"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some("python3")));
    assert!(cloud["bootcmd"][0][2]
        .as_str()
        .unwrap()
        .contains("policy drop"));
    assert!(!serde_yaml::to_string(&cloud["runcmd"])
        .unwrap()
        .contains("iptables -P"));
    assert!(user_data.contains("20-vzctl-router-bootstrap.conf"));
    assert!(user_data.contains("ExecStartPre=+/usr/local/lib/vzctl/router-apply --bootstrap"));
    assert!(user_data.contains("/usr/local/lib/vzctl/virtiofs-bind"));
    assert!(user_data.contains("/etc/sudoers.d/vzctl-virtiofs"));
    assert!(user_data.contains("/etc/sudoers.d/vzctl-agent"));
    assert!(user_data.contains("vzctl-agent ALL=(ALL) NOPASSWD:ALL"));
    assert!(user_data.contains("NoNewPrivileges=no"));
    assert!(user_data.contains("systemctl"));
    assert!(user_data.contains("daemon-reload"));
    assert!(user_data.contains("vzctl-agent.service"));
    let network_config = &backend.seeds()[0].1;
    assert!(network_config.contains("10.70.0.10/24"));
    assert!(network_config.contains("via: 10.70.0.0"));
    assert!(network_config.contains("on-link: true"));
    assert!(network_config.contains("addresses:\n        - 10.70.0.0"));
    assert!(network_config.contains("search:\n        - edge-dmz.vz.test"));
    assert!(network_config.contains("set-name: enp0s1"));
    assert!(!network_config.contains("10.70.0.2"));
    assert!(!network_config.contains("dhcp4: true"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn clonefile_failure_is_exit_16_and_removes_partial_bundle() {
    let directory = test_directory("vm-clone-failure");
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    let image = prepare_sealed_test_image(&directory, &images_directory);
    let backend = RecordingVmDiskBackend::failing_clone("apfs");
    let failure = create_vm_bundle_in_dirs(
        &VmCreateOptions {
            id: "web".to_string(),
            from: image.to_string_lossy().to_string(),
            disk_gib: 1,
            cpus: DEFAULT_VM_CPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            roles: Vec::new(),
            requested_network: None,
            network: None,
            networks: Vec::new(),
            root_password: None,
            cloud_init: None,
            project: None,
            mounts: Vec::new(),
            format: OutputFormat::Json,
        },
        &backend,
        &images_directory,
        &vms_directory,
    )
    .unwrap_err();

    assert_eq!(failure.code, EXIT_VM_DISK_PREP_FAILED);
    assert!(failure.message.contains("clonefile failed"));
    assert!(!vms_directory.join("web").exists());
    assert_eq!(
        fs::metadata(&image).unwrap().permissions().mode() & 0o222,
        0
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn two_native_apfs_clonefiles_diverge_without_changing_shared_base() {
    let directory = test_directory("vm-native-apfs");
    fs::create_dir_all(&directory).unwrap();
    if !filesystem_type(&directory).is_some_and(|value| value.eq_ignore_ascii_case("apfs")) {
        fs::remove_dir_all(directory).unwrap();
        return;
    }
    let images_directory = directory.join("images");
    let vms_directory = directory.join("vms");
    fs::create_dir_all(&images_directory).unwrap();
    let image = directory.join("base.raw");
    let original = vec![0x5a; 4 * 1024 * 1024];
    let read_original_extent = |path: &Path| {
        let mut bytes = vec![0_u8; original.len()];
        std::io::Read::read_exact(&mut File::open(path).unwrap(), &mut bytes).unwrap();
        bytes
    };
    fs::write(&image, &original).unwrap();
    seal_image_in_dir(
        &ImageSealOptions {
            input: image.to_string_lossy().to_string(),
            tag: Some("v1".to_string()),
            format: OutputFormat::Json,
            force: false,
        },
        &RecordingSealBackend::new("raw"),
        &images_directory,
    )
    .unwrap();

    let mut results = Vec::new();
    for id in ["native-a", "native-b"] {
        let result = create_vm_bundle_in_dirs(
            &VmCreateOptions {
                id: id.to_string(),
                from: image.to_string_lossy().to_string(),
                disk_gib: 1,
                cpus: DEFAULT_VM_CPUS,
                memory_mib: DEFAULT_VM_MEMORY_MIB,
                roles: Vec::new(),
                requested_network: None,
                network: None,
                networks: Vec::new(),
                root_password: None,
                cloud_init: None,
                project: None,
                mounts: Vec::new(),
                format: OutputFormat::Human,
            },
            &NativeVmDiskBackend,
            &images_directory,
            &vms_directory,
        )
        .unwrap();
        assert_eq!(result.clone_mode, CloneMode::Linked);
        assert_eq!(read_original_extent(&result.root_disk_path), original);
        assert_eq!(fs::metadata(&result.root_disk_path).unwrap().len(), 1 << 30);
        results.push(result);
    }
    let mut clone = OpenOptions::new()
        .write(true)
        .open(&results[0].root_disk_path)
        .unwrap();
    clone.write_all(b"diverged").unwrap();
    clone.sync_all().unwrap();
    assert_eq!(fs::read(&image).unwrap(), original);
    assert_ne!(read_original_extent(&results[0].root_disk_path), original);
    assert_eq!(read_original_extent(&results[1].root_disk_path), original);
    assert_eq!(
        fs::metadata(&image).unwrap().permissions().mode() & 0o222,
        0
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn native_backend_builds_private_nocloud_iso() {
    let directory = test_directory("vm-native-cidata");
    let seed_directory = directory.join("seed");
    fs::create_dir_all(&seed_directory).unwrap();
    fs::write(seed_directory.join("meta-data"), "instance-id: test\n").unwrap();
    fs::write(seed_directory.join("network-config"), "version: 2\n").unwrap();
    fs::write(seed_directory.join("user-data"), "#cloud-config\n").unwrap();
    let destination = directory.join("cidata.iso");

    NativeVmDiskBackend
        .create_cloud_init_iso(&seed_directory, &destination)
        .unwrap();

    assert!(destination.is_file());
    assert!(fs::metadata(&destination).unwrap().len() > 0);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn vm_create_json_matches_golden_contract() {
    let result = VmCreateResult {
        id: "web".to_string(),
        bundle_path: PathBuf::from("/state/vms/web"),
        source: ImageSealResult {
            name: "ubuntu-base".to_string(),
            source_path: PathBuf::from("/images/ubuntu-base.raw"),
            image_format: "raw".to_string(),
            marker_path: PathBuf::from("/images/ubuntu-base-abc.sealed.json"),
            already_sealed: true,
        },
        root_disk_path: PathBuf::from("/state/vms/web/disk.raw"),
        cidata_path: PathBuf::from("/state/vms/web/cidata.iso"),
        agent_token_path: PathBuf::from("/state/vms/web/agent.token"),
        cloud_init_summary: CloudInitSummary {
            datasource: "nocloud",
            custom: false,
            roles: Vec::new(),
            packages: Vec::new(),
            write_files: vec!["/var/lib/vzctl/agent.token".to_string()],
            commands: 1,
            users: 0,
        },
        disk_gib: 64,
        cpus: DEFAULT_VM_CPUS,
        memory_mib: DEFAULT_VM_MEMORY_MIB,
        roles: Vec::new(),
        mounts: Vec::new(),
        clone_mode: CloneMode::Linked,
        identity: VmIdentity {
            instance_id: "123e4567-e89b-42d3-a456-426614174000".to_string(),
            hostname: "web".to_string(),
            fqdn: "web".to_string(),
            mac_addresses: vec!["02:12:34:56:78:9a".to_string()],
        },
        network: Some(network::VmNetworkSelection {
            network_id: "lan".to_string(),
            network: "lan".to_string(),
            cidr: "10.70.0.0/24".to_string(),
            ip: "10.70.0.10".to_string(),
            gateway: "10.70.0.0".to_string(),
            dns: "10.70.0.0".to_string(),
            project: Some("edge-dmz".to_string()),
            prefix: 24,
            automatic: true,
            created: true,
            backend: "vmnet".to_string(),
        }),
        networks: Vec::new(),
    };
    let expected: Value =
        serde_json::from_str(include_str!("../tests/golden/vm-create.json")).unwrap();
    assert_eq!(vm_create_json(&result), expected);
}

#[test]
fn cloud_init_summary_never_contains_secret_values_or_commands() {
    let config: serde_yaml::Value = serde_yaml::from_str(
        r#"
packages: [curl, docker-ce]
write_files:
  - path: /etc/example.conf
content: api_token=super-secret
runcmd:
  - [sh, -c, "send --password hunter2"]
users:
  - name: demo
passwd: hidden
"#,
    )
    .unwrap();
    let summary = summarize_cloud_init(&config, &["docker".to_string()], true);
    let encoded = serde_json::to_string(&summary).unwrap();
    assert_eq!(summary.packages, ["curl", "docker-ce"]);
    assert_eq!(summary.write_files, ["/etc/example.conf"]);
    assert_eq!(summary.commands, 1);
    assert_eq!(summary.users, 1);
    assert!(!encoded.contains("super-secret"));
    assert!(!encoded.contains("hunter2"));
    assert!(!encoded.contains("hidden"));
}

#[test]
fn event_options_parse_filter_list() {
    let args = ["--filter", "vm.*,apply.*"].into_iter().map(str::to_string);
    assert_eq!(
        parse_events_options(args).unwrap(),
        EventsOptions {
            filter: Some("vm.*,apply.*".to_string()),
        }
    );
}

#[test]
fn event_filter_accepts_exact_and_suffix_wildcards() {
    assert!(valid_event_filter("vm.*, apply.failed"));
    assert!(valid_event_filter("*"));
    assert!(!valid_event_filter("vm.*.failed"));
    assert!(!valid_event_filter("vm.,"));
}

#[test]
fn parses_available_kib_from_df() {
    let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
              /dev/disk3s1 100000 25000 75000 25% /System/Volumes/Data\n";
    assert_eq!(parse_df_available_kib(df), Some(75_000));
}

#[test]
fn detects_virtualization_entitlement() {
    let plist = "<key>com.apple.security.virtualization</key><true/>";
    assert!(has_virtualization_entitlement(plist));
    assert!(!has_virtualization_entitlement("<dict></dict>"));
}

#[test]
fn parses_apfs_from_mount_output() {
    let mounts = "/dev/disk3s1s1 on / (apfs, sealed, local, read-only)\n\
                  devfs on /dev (devfs, local, nobrowse)\n";
    assert_eq!(
        filesystem_for_mount_point(mounts, "/").as_deref(),
        Some("apfs")
    );
}

#[test]
fn resolver_requires_localhost_and_configured_port() {
    let directory =
        std::env::temp_dir().join(format!("vzctl-doctor-resolver-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("demo.vz.test");
    fs::write(&path, "nameserver 127.0.0.1\nport 15353\n").unwrap();
    assert!(resolver_points_to(&path, 15353));
    assert!(!resolver_points_to(&path, 15354));
    fs::remove_dir_all(directory).unwrap();
}

fn test_directory(label: &str) -> PathBuf {
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "vzctl-{label}-{fixture}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn prepare_sealed_test_image(directory: &Path, images_directory: &Path) -> PathBuf {
    fs::create_dir_all(images_directory).unwrap();
    let image = directory.join("base.raw");
    fs::write(&image, b"sealed base blocks").unwrap();
    seal_image_in_dir(
        &ImageSealOptions {
            input: image.to_string_lossy().to_string(),
            tag: Some("v1".to_string()),
            format: OutputFormat::Json,
            force: false,
        },
        &RecordingSealBackend::new("raw"),
        images_directory,
    )
    .unwrap();
    image
}

struct RecordingVmDiskBackend {
    filesystem: String,
    fail_clone: bool,
    calls: RefCell<Vec<&'static str>>,
    seeds: RefCell<Vec<(String, String, String)>>,
}

impl RecordingVmDiskBackend {
    fn new(filesystem: &str) -> Self {
        Self {
            filesystem: filesystem.to_string(),
            fail_clone: false,
            calls: RefCell::new(Vec::new()),
            seeds: RefCell::new(Vec::new()),
        }
    }

    fn failing_clone(filesystem: &str) -> Self {
        Self {
            filesystem: filesystem.to_string(),
            fail_clone: true,
            calls: RefCell::new(Vec::new()),
            seeds: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }

    fn seeds(&self) -> Vec<(String, String, String)> {
        self.seeds.borrow().clone()
    }
}

impl VmDiskBackend for RecordingVmDiskBackend {
    fn filesystem_type(&self, _path: &Path) -> Option<String> {
        Some(self.filesystem.clone())
    }

    fn clone_linked(&self, source: &Path, destination: &Path) -> Result<(), io::Error> {
        self.calls.borrow_mut().push("linked");
        if self.fail_clone {
            return Err(io::Error::other("injected clonefile failure"));
        }
        fs::copy(source, destination)?;
        let mut permissions = fs::metadata(destination)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(destination, permissions)
    }

    fn create_cloud_init_iso(
        &self,
        seed_directory: &Path,
        destination: &Path,
    ) -> Result<(), io::Error> {
        self.calls.borrow_mut().push("seed");
        self.seeds.borrow_mut().push((
            fs::read_to_string(seed_directory.join("meta-data"))?,
            fs::read_to_string(seed_directory.join("network-config"))?,
            fs::read_to_string(seed_directory.join("user-data"))?,
        ));
        fs::write(destination, b"test NoCloud ISO")
    }
}

struct RecordingSealBackend {
    image_format: String,
    calls: RefCell<Vec<&'static str>>,
}

impl RecordingSealBackend {
    fn new(image_format: &str) -> Self {
        Self {
            image_format: image_format.to_string(),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
}

impl ImageSealBackend for RecordingSealBackend {
    fn inspect_format(&self, _path: &Path) -> Result<String, SealFailure> {
        self.calls.borrow_mut().push("inspect");
        Ok(self.image_format.clone())
    }

    fn verify_preserved(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        self.calls.borrow_mut().push("preserved");
        Ok(())
    }

    fn customize(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        self.calls.borrow_mut().push("customize");
        Ok(())
    }

    fn verify_clone_safe(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        self.calls.borrow_mut().push("clone-safe");
        Ok(())
    }
}
