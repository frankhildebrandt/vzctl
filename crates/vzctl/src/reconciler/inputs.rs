use super::*;

/// Snapshot files read by apply without persisting their contents (including secrets).
/// Mounted directory contents and remote image tags are deliberately not input files.
pub(super) fn append_input_resources(
    environment: &Environment,
    config_path: &Path,
    state_dir: &Path,
    resources: &mut Vec<Resource>,
) -> Result<(), Failure> {
    let config_file = config::config_path(config_path);
    let base = config_file.parent().unwrap_or_else(|| Path::new("."));
    let base = fs::canonicalize(base)
        .map_err(|e| Failure::new(EXIT_INVALID, format!("resolve config directory: {e}")))?;
    let mut paths = BTreeSet::new();
    paths.extend(crate::kubernetes::manifest_paths(environment, &base));
    for provider in environment.spec.service_providers.values() {
        for path in [
            &provider.password_file,
            &provider.access_key_file,
            &provider.secret_key_file,
        ]
        .into_iter()
        .flatten()
        {
            paths.insert(base.join(path));
        }
    }
    for vm in environment.spec.vms.values() {
        if let Some(path) = &vm.cloud_init {
            paths.insert(base.join(path));
        }
        for compose in &vm.compose_files {
            collect_compose_inputs(&base.join(compose), &mut paths)?;
        }
    }
    if let Some(oidc) = &environment.spec.oidc {
        if oidc.enabled && oidc.mode == config::OidcMode::Embedded {
            if let Some(path) = &oidc.password_file {
                paths.insert(base.join(path));
            }
            let host_path = crate::oidc::host_uplink_path(state_dir);
            paths.insert(host_path.clone()); // Also track absent → present host defaults.
            if oidc.uplink.is_some() || host_path.exists() {
                let secret = oidc
                    .uplink
                    .as_ref()
                    .and_then(|u| u.client_secret_file.as_deref());
                let path = match secret {
                    None | Some("") | Some("host") => crate::oidc::host_secret_path(state_dir),
                    Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
                    Some(path) => {
                        crate::oidc::project_oidc_dir(state_dir, &environment.spec.project)
                            .join(path)
                    }
                };
                paths.insert(path);
            }
        }
    }
    for path in paths {
        let digest = file_fingerprint(&path)?;
        resources.push(Resource {
            kind: "input".into(),
            name: path.display().to_string(),
            labels: BTreeMap::from([
                ("managed-by".into(), "vzctl".into()),
                ("project".into(), environment.spec.project.clone()),
                ("stack_id".into(), stack_id(environment)),
                (
                    "spec".into(),
                    serde_json::to_string(&json!({"sha256": digest})).unwrap(),
                ),
            ]),
            state: "active".into(),
        });
    }
    resources.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
    Ok(())
}

pub(super) fn file_fingerprint(path: &Path) -> Result<Option<String>, Failure> {
    // Stream rather than buffering potentially large user files.
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Failure::new(
                EXIT_INVALID,
                format!("read input {}: {e}", path.display()),
            ))
        }
    };
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| {
            Failure::new(EXIT_INVALID, format!("hash input {}: {e}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(Some(format!("{:x}", digest.finalize())))
}

pub(super) fn collect_compose_inputs(
    path: &Path,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<(), Failure> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !paths.insert(path.clone()) {
        return Ok(());
    }
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    paths.insert(base.join(".env"));
    let text = fs::read_to_string(&path).map_err(|e| {
        Failure::new(
            EXIT_INVALID,
            format!("read compose {}: {e}", path.display()),
        )
    })?;
    let value: Value = serde_yaml::from_str(&text).map_err(|e| {
        Failure::new(
            EXIT_INVALID,
            format!("parse compose {}: {e}", path.display()),
        )
    })?;
    // Only inspect Compose file-bearing fields. Application values named `file`
    // or `path` in environment/labels must never be treated as filesystem reads.
    for section in ["configs", "secrets"] {
        if let Some(entries) = value[section].as_object() {
            for entry in entries.values() {
                if let Some(file) = entry["file"].as_str() {
                    paths.insert(base.join(file));
                }
            }
        }
    }
    if let Some(services) = value["services"].as_object() {
        for service in services.values() {
            let env_files = match &service["env_file"] {
                Value::Array(files) => files.clone(),
                Value::Null => Vec::new(),
                file => vec![file.clone()],
            };
            for file in env_files {
                if let Some(file) = file.as_str().or_else(|| file["path"].as_str()) {
                    paths.insert(base.join(file));
                }
            }
            if let Some(file) = service["extends"]["file"].as_str() {
                collect_compose_inputs(&base.join(file), paths)?;
            }
        }
    }
    if let Some(includes) = value["include"].as_array() {
        for include in includes {
            let files = match include.get("path").unwrap_or(include) {
                Value::Array(files) => files.clone(),
                file => vec![file.clone()],
            };
            for file in files {
                if let Some(file) = file.as_str() {
                    collect_compose_inputs(&base.join(file), paths)?;
                }
            }
        }
    }
    Ok(())
}
