use super::*;

pub(super) fn adopt_report(
    stack_id: &str,
    environment: &Environment,
    desired: &[Resource],
    actual: &[Resource],
    socket_path: &Path,
) -> Result<Value, Failure> {
    let mut candidates = BTreeSet::new();
    for resource in desired.iter().chain(actual.iter()) {
        if resource.kind == "vm" {
            candidates.insert(resource.name.clone());
        }
    }
    let project = environment.spec.project.as_str();
    for (vm_id, manifest_path) in discover_vm_manifests(&crate::state_dir().join("vms")) {
        let Ok(raw) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if manifest["managed-by"] != "vzctl" {
            continue;
        }
        let labels = &manifest["labels"];
        let matches_stack = labels["stack_id"].as_str() == Some(stack_id)
            || (labels["project"].as_str() == Some(project) && candidates.contains(&vm_id));
        if matches_stack || candidates.contains(&vm_id) {
            candidates.insert(vm_id);
        }
    }

    let runtime = match rpc(socket_path, "vm.list", json!({})) {
        Ok(records) => records,
        Err(failure) if failure.code == EXIT_SUPERVISOR => json!([]),
        Err(failure) => return Err(failure),
    };
    let live: BTreeSet<String> = runtime
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| matches!(record["state"].as_str(), Some("starting") | Some("running")))
        .filter_map(|record| record["vm_id"].as_str().map(str::to_string))
        .collect();

    let mut actions = Vec::new();
    for vm_id in &candidates {
        if live.contains(vm_id) {
            continue;
        }
        let lock_path = helper_lock_path(vm_id);
        if !lock_path.is_file() {
            continue;
        }
        if !is_safe_stale_helper_lock(&lock_path) {
            continue;
        }
        actions.push(json!({
            "action": "report",
            "kind": "helper-lock",
            "name": vm_id,
            "breaking": false,
            "reason": "stale helper lock (pid dead or flock free)",
        }));
    }

    let changed = !actions.is_empty();
    let message = if changed {
        format!("{} stale helper lock(s) reported", actions.len())
    } else {
        "no lockfile-only resources adopted".to_string()
    };
    Ok(json!({
        "message": message,
        "stack_id": stack_id,
        "actions": actions,
        "changed": changed,
        "minimal": !changed,
    }))
}

pub(super) fn helper_lock_path(vm_id: &str) -> PathBuf {
    crate::state_dir()
        .join("helpers")
        .join(format!("{}.lock", state_file_component(vm_id)))
}

pub(super) fn discover_vm_manifests(vms_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(vms_dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let manifest = path.join("vm.json");
        if manifest.is_file() {
            found.push((name, manifest));
            continue;
        }
        // Nested project/vm bundles: vms/{project}/{vm}/vm.json
        let Ok(children) = fs::read_dir(&path) else {
            continue;
        };
        for child in children.flatten() {
            let child_path = child.path();
            if !child_path.is_dir() {
                continue;
            }
            let child_name = child.file_name().to_string_lossy().to_string();
            let nested_manifest = child_path.join("vm.json");
            if nested_manifest.is_file() {
                found.push((format!("{name}/{child_name}"), nested_manifest));
            }
        }
    }
    found
}

pub(super) fn state_file_component(value: &str) -> String {
    let prefix: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let hash = value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{prefix}-{hash:x}")
}

pub(super) fn is_safe_stale_helper_lock(path: &Path) -> bool {
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let fd = file.as_raw_fd();
    let locked = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if locked == 0 {
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        return true;
    }
    // Lock held — only treat as stale if recorded PID is dead.
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<i32>() else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return false;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}
