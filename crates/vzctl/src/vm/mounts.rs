use super::*;

pub(super) fn guest_mount_unsupported(failure: &Failure) -> bool {
    let message = failure.message.to_ascii_lowercase();
    message.contains("unsupported") && message.contains("method")
}

/// Bind a virtiofs share into PID 1's mount namespace (visible to Docker).
///
/// After a live share swap, previous guest mounts may show `//deleted` in
/// mountinfo until cleared. Always clear + remount in the init mount ns.
pub(super) fn guest_virtiofs_bind_mount(
    id: &str,
    mount: &crate::mounts::ResolvedMount,
    socket_path: &Path,
) -> Result<(), Failure> {
    // Deploy the current helper (PrivateTmp hides /tmp from PID 1) then bind.
    let script = crate::guest_utils::VIRTIOFS_BIND_SCRIPT;
    let deploy_and_bind = format!(
        r#"set -eu
helper=/usr/local/lib/vzctl/virtiofs-bind
cat >"$helper.new" <<'VZCTL_VIRTIOFS_BIND_EOF'
{script}
VZCTL_VIRTIOFS_BIND_EOF
chmod 0755 "$helper.new"
mv -f "$helper.new" "$helper"
nsenter -t 1 -m -- "$helper" mount {name} {target}{mode_arg}
"#,
        script = script,
        name = sh_escape(&mount.name),
        target = sh_escape(&mount.target),
        mode_arg = if mount.read_only { " ro" } else { "" },
    );
    let args = vec![
        "sudo".into(),
        "-n".into(),
        "sh".into(),
        "-c".into(),
        deploy_and_bind,
    ];
    let result = exec_vm(id, &args, None, &BTreeMap::new(), 60_000, socket_path)?;
    let exit = result["exit_code"].as_u64().unwrap_or(1);
    if exit != 0 {
        let stderr = result
            .pointer("/exec/stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let stdout = result
            .pointer("/exec/stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "virtiofs-bind failed"
        };
        return Err(Failure::new(
            EXIT_GUEST,
            format!("guest virtiofs bind {}: {detail}", mount.target),
        ));
    }
    Ok(())
}

pub(super) fn sh_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(super) fn mount_vm(
    id: &str,
    source: &Path,
    target: &str,
    name: Option<&str>,
    read_only: bool,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let _ = read_manifest(&bundle)?;
    let mut flag = format!("source={},target={}", source.display(), target);
    if let Some(name) = name {
        flag = format!("tag={name},{flag}");
    }
    if read_only {
        flag.push_str(",ro");
    }
    let mount = crate::mounts::parse_mount_flag(&flag)
        .map_err(|message| Failure::new(EXIT_INVALID, message))?;
    let mut mounts = crate::mounts::read_manifest_mounts(&bundle)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;
    if mounts
        .iter()
        .any(|existing| existing.name == mount.name && existing.target != mount.target)
    {
        return Err(Failure::new(
            EXIT_INVALID,
            format!(
                "mount name {} already maps to {}",
                mount.name,
                mounts
                    .iter()
                    .find(|existing| existing.name == mount.name)
                    .map(|existing| existing.target.as_str())
                    .unwrap_or("?")
            ),
        ));
    }
    if mounts
        .iter()
        .any(|existing| existing.target == mount.target && existing.name != mount.name)
    {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("mount target {} is already in use", mount.target),
        ));
    }
    mounts.retain(|existing| existing.name != mount.name);
    mounts.push(mount.clone());
    crate::mounts::write_manifest_mounts(&bundle, &mounts)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;

    if helper_is_running(id, socket_path)? {
        let rpc_result = rpc(
            socket_path,
            "vm.mount.add",
            json!({
                "vm_id": id,
                "name": mount.name,
                "source": mount.source,
                "target": mount.target,
                "read_only": mount.read_only,
            }),
        );
        let result = match rpc_result {
            Ok(value) => value,
            Err(failure) if guest_mount_unsupported(&failure) => {
                // Helper applies the share before agent fs.mount; old agents
                // lack fs.mount — guest_virtiofs_bind_mount below still binds.
                json!({ "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>() })
            }
            Err(failure) => return Err(failure),
        };
        // Agent may run under PrivateTmp (own mount ns). Docker uses PID 1's
        // namespace — always ensure the bind there (clears //deleted after share swap).
        guest_virtiofs_bind_mount(id, &mount, socket_path)?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.mount",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": "mount added (live)",
                "vm_id": id,
                "live": true,
            },
            "vm_id": id,
            "mount": mount.to_json(),
            "mounts": result.get("mounts").cloned().unwrap_or(json!([])),
            "live": true,
        }));
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.mount",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": "mount recorded (VM stopped; applied on next start)",
            "vm_id": id,
            "live": false,
        },
        "vm_id": id,
        "mount": mount.to_json(),
        "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "live": false,
    }))
}

pub(super) fn unmount_vm(
    id: &str,
    target: Option<&str>,
    name: Option<&str>,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let _ = read_manifest(&bundle)?;
    let mut mounts = crate::mounts::read_manifest_mounts(&bundle)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;
    let before = mounts.len();
    mounts.retain(|mount| {
        if let Some(name) = name {
            if mount.name == name {
                return false;
            }
        }
        if let Some(target) = target {
            if mount.target == target {
                return false;
            }
        }
        true
    });
    if mounts.len() == before {
        return Err(Failure::new(EXIT_INVALID, "mount not found"));
    }
    crate::mounts::write_manifest_mounts(&bundle, &mounts)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;

    if helper_is_running(id, socket_path)? {
        let mut params = json!({ "vm_id": id });
        if let Some(name) = name {
            params["name"] = json!(name);
        }
        if let Some(target) = target {
            params["target"] = json!(target);
        }
        let result = rpc(socket_path, "vm.mount.remove", params)?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.unmount",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": "mount removed (live)",
                "vm_id": id,
                "live": true,
            },
            "vm_id": id,
            "mounts": result.get("mounts").cloned().unwrap_or(json!([])),
            "live": true,
        }));
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.unmount",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": "mount removed from manifest",
            "vm_id": id,
            "live": false,
        },
        "vm_id": id,
        "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "live": false,
    }))
}

pub(super) fn list_vm_mounts(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let bundle = bundle_path(id);
    let _ = read_manifest(&bundle)?;
    let mounts = crate::mounts::read_manifest_mounts(&bundle)
        .map_err(|message| Failure::new(EXIT_VM_DISK, message))?;
    let live = if helper_is_running(id, socket_path)? {
        rpc(socket_path, "vm.mount.list", json!({ "vm_id": id })).ok()
    } else {
        None
    };
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.mounts",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("{} mount(s)", mounts.len()),
            "vm_id": id,
        },
        "vm_id": id,
        "mounts": mounts.iter().map(crate::mounts::ResolvedMount::to_json).collect::<Vec<_>>(),
        "runtime": live,
    }))
}
