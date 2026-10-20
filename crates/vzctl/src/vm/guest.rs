use super::*;

pub(super) fn exec_vm(
    id: &str,
    cmd: &[String],
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
    timeout_ms: u64,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let outcome = exec_in_vm(id, cmd, cwd, env, timeout_ms, socket_path)
        .map_err(|message| Failure::new(EXIT_SUPERVISOR, message))?;
    Ok(exec_envelope(id, cmd, &outcome))
}

pub(super) fn exec_envelope(id: &str, cmd: &[String], outcome: &ExecOutcome) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "command": "vm.exec",
        "status": if outcome.exit == 0 { "ok" } else { "fail" },
        "exit_code": outcome.exit,
        "summary": {
            "message": format!("exit {}", outcome.exit),
            "vm_id": id,
            "exit": outcome.exit,
            "truncated": outcome.truncated,
        },
        "exec": {
            "cmd": cmd,
            "exit": outcome.exit,
            "stdout": outcome.stdout,
            "stderr": outcome.stderr,
            "truncated": outcome.truncated,
        },
    })
}

pub(super) fn exec_tty_vm(
    id: &str,
    cmd: &[String],
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let (cols, rows) = terminal_winsize();
    let mut params = json!({
        "vm_id": id,
        "cmd": cmd,
        "cols": cols,
        "rows": rows,
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    params["env"] = json!(tty_exec_env(env));
    let result = rpc(socket_path, "vm.exec_tty", params)?;
    let path = result["socket"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "vm.exec_tty missing socket path"))?;
    let mut stream = UnixStream::connect(path).map_err(|error| {
        Failure::new(EXIT_SUPERVISOR, format!("exec tty socket {path}: {error}"))
    })?;
    eprintln!("attached to {id} exec tty (Ctrl-P Ctrl-Q to detach)");
    let mut resize = [0_u8; 4];
    resize[0..2].copy_from_slice(&cols.to_le_bytes());
    resize[2..4].copy_from_slice(&rows.to_le_bytes());
    write_mux_frame(&mut stream, MUX_RESIZE, &resize)?;
    let exit = mux_tty_session(&mut stream)?;
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.exec",
        "status": if exit == 0 { "ok" } else { "fail" },
        "exit_code": exit,
        "summary": {
            "message": format!("exit {exit}"),
            "vm_id": id,
            "exit": exit,
            "tty": true,
        },
        "exec": {
            "cmd": cmd,
            "exit": exit,
            "tty": true,
        },
    }))
}

pub(super) const DEFAULT_TTY_TERM: &str = "xterm-256color";

/// Guest terminfo rarely includes host types like xterm-ghostty; keep a
/// portable default unless the caller set TERM explicitly.
pub(super) fn tty_exec_env(env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = env.clone();
    let missing = out
        .get("TERM")
        .map(|value| value.is_empty())
        .unwrap_or(true);
    if missing {
        out.insert("TERM".into(), DEFAULT_TTY_TERM.into());
    }
    out
}

pub(super) fn transfer_vm(
    id: &str,
    src: &TransferPath,
    dst: &TransferPath,
    socket_path: &Path,
) -> Result<Value, Failure> {
    match (src, dst) {
        (TransferPath::Host(host), TransferPath::Guest(guest)) => {
            let bytes = fs::read(host).map_err(|error| {
                Failure::new(EXIT_INVALID, format!("read {}: {error}", host.display()))
            })?;
            if bytes.len() > TRANSFER_MAX_BYTES {
                return Err(Failure::new(
                    EXIT_UNAVAILABLE,
                    format!(
                        "transfer exceeds {} KiB agent limit; use virtiofs for larger files",
                        TRANSFER_MAX_BYTES / 1024
                    ),
                ));
            }
            let encoded =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            let result = rpc(
                socket_path,
                "vm.exec",
                json!({
                    "vm_id": id,
                    "cmd": ["tee", guest],
                    "timeout_ms": 30_000,
                    "stdin_b64": encoded,
                }),
            )?;
            if result["exit"].as_u64().unwrap_or(1) != 0
                || result["truncated"].as_bool().unwrap_or(false)
            {
                return Err(Failure::new(
                    EXIT_GUEST,
                    format!(
                        "push failed: {}",
                        result["stderr"].as_str().unwrap_or("guest exec error")
                    ),
                ));
            }
            Ok(transfer_envelope(
                id,
                "push",
                &host.display().to_string(),
                guest,
                bytes.len(),
            ))
        }
        (TransferPath::Guest(guest), TransferPath::Host(host)) => {
            let result = rpc(
                socket_path,
                "vm.exec",
                json!({
                    "vm_id": id,
                    "cmd": ["base64", "-w0", guest],
                    "timeout_ms": 30_000,
                }),
            )?;
            if result["exit"].as_u64().unwrap_or(1) != 0 {
                return Err(Failure::new(
                    EXIT_GUEST,
                    format!(
                        "pull failed: {}",
                        result["stderr"].as_str().unwrap_or("guest exec error")
                    ),
                ));
            }
            if result["truncated"].as_bool().unwrap_or(false) {
                return Err(Failure::new(
                    EXIT_UNAVAILABLE,
                    "pull truncated by agent limits; use virtiofs for larger files",
                ));
            }
            let encoded = result["stdout"].as_str().unwrap_or("").trim();
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                .map_err(|error| {
                    Failure::new(EXIT_GUEST, format!("invalid guest base64: {error}"))
                })?;
            if bytes.len() > TRANSFER_MAX_BYTES {
                return Err(Failure::new(
                    EXIT_UNAVAILABLE,
                    format!(
                        "transfer exceeds {} KiB agent limit; use virtiofs for larger files",
                        TRANSFER_MAX_BYTES / 1024
                    ),
                ));
            }
            if let Some(parent) = host.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).map_err(|error| {
                        Failure::new(
                            EXIT_INVALID,
                            format!("create {}: {error}", parent.display()),
                        )
                    })?;
                }
            }
            fs::write(host, &bytes).map_err(|error| {
                Failure::new(EXIT_INVALID, format!("write {}: {error}", host.display()))
            })?;
            Ok(transfer_envelope(
                id,
                "pull",
                guest,
                &host.display().to_string(),
                bytes.len(),
            ))
        }
        _ => Err(Failure::new(
            EXIT_INVALID,
            "vm transfer requires one host path and one <id>:<guest-path>",
        )),
    }
}

pub(super) fn transfer_envelope(
    id: &str,
    direction: &str,
    src: &str,
    dst: &str,
    bytes: usize,
) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "command": "vm.transfer",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("copied {bytes} bytes ({direction})"),
            "vm_id": id,
            "change": "copied",
            "bytes": bytes,
            "direction": direction,
        },
        "transfer": {
            "src": src,
            "dst": dst,
            "bytes": bytes,
            "direction": direction,
            "truncated": false,
        },
    })
}

pub(super) fn services_vm(
    id: &str,
    action: &ServicesAction,
    unit_type: &str,
    all: bool,
    socket_path: &Path,
) -> Result<Value, Failure> {
    match services_vm_agent(id, action, unit_type, all, socket_path) {
        Ok(envelope) => Ok(envelope),
        Err(agent_failure) => {
            services_vm_exec_fallback(id, action, unit_type, all, socket_path, agent_failure)
        }
    }
}

pub(super) fn services_vm_agent(
    id: &str,
    action: &ServicesAction,
    unit_type: &str,
    all: bool,
    socket_path: &Path,
) -> Result<Value, Failure> {
    let vm_id = json!({ "vm_id": id });
    let status = rpc(socket_path, "vm.agent.systemd.status", vm_id.clone())?;
    let available = status
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !available {
        return Err(Failure::new(
            EXIT_UNAVAILABLE,
            "systemd is not available on this guest",
        ));
    }
    let mut out = json!({
        "apiVersion": API_VERSION,
        "command": "vm.services",
        "status": "ok",
        "exit_code": 0,
        "systemd": {
            "available": true,
            "version": status.get("version").cloned().unwrap_or(Value::Null),
        },
    });
    match action {
        ServicesAction::List => {
            let mut params = vm_id;
            if let Some(object) = params.as_object_mut() {
                object.insert("type".into(), json!(unit_type));
                if all {
                    object.insert("all".into(), json!(true));
                }
            }
            let result = rpc(socket_path, "vm.agent.systemd.list", params)?;
            out["units"] = result.get("units").cloned().unwrap_or_else(|| json!([]));
        }
        ServicesAction::Status(unit) => {
            validate_systemd_unit(unit)?;
            let result = rpc(
                socket_path,
                "vm.agent.systemd.show",
                json!({ "vm_id": id, "unit": unit }),
            )?;
            out["unit"] = result.get("unit").cloned().unwrap_or_else(|| json!({}));
        }
        ServicesAction::Start(unit)
        | ServicesAction::Stop(unit)
        | ServicesAction::Restart(unit) => {
            let verb = services_action_verb(action);
            validate_systemd_unit(unit)?;
            rpc(
                socket_path,
                "vm.agent.systemd.control",
                json!({ "vm_id": id, "unit": unit, "action": verb }),
            )?;
            out["control"] = json!({
                "unit": unit,
                "action": verb,
                "ok": true,
            });
        }
    }
    Ok(out)
}

pub(super) fn services_vm_exec_fallback(
    id: &str,
    action: &ServicesAction,
    unit_type: &str,
    all: bool,
    socket_path: &Path,
    agent_failure: Failure,
) -> Result<Value, Failure> {
    if matches!(action, ServicesAction::Status(_)) {
        return Err(agent_failure);
    }
    let cmd = match action {
        ServicesAction::List => {
            let mut argv = vec![
                "systemctl".to_string(),
                "list-units".to_string(),
                format!("--type={unit_type}"),
                "--no-pager".to_string(),
                "--plain".to_string(),
            ];
            if all {
                argv.push("--all".to_string());
            }
            argv
        }
        ServicesAction::Status(_) => unreachable!(),
        ServicesAction::Start(unit) => {
            validate_systemd_unit(unit)?;
            vec!["systemctl".to_string(), "start".to_string(), unit.clone()]
        }
        ServicesAction::Stop(unit) => {
            validate_systemd_unit(unit)?;
            vec!["systemctl".to_string(), "stop".to_string(), unit.clone()]
        }
        ServicesAction::Restart(unit) => {
            validate_systemd_unit(unit)?;
            vec!["systemctl".to_string(), "restart".to_string(), unit.clone()]
        }
    };
    let envelope = exec_vm(id, &cmd, None, &BTreeMap::new(), 30_000, socket_path)?;
    let mut out = envelope;
    out["command"] = json!("vm.services");
    out["systemd"] = json!({ "available": false });
    if let Some(exec) = out.get("exec") {
        if exec.get("exit").and_then(Value::as_u64) == Some(0) {
            if let Some(obj) = out.get_mut("systemd") {
                if let Some(map) = obj.as_object_mut() {
                    map.insert("available".into(), json!(true));
                }
            }
        }
    }
    out["services"] = out["exec"].clone();
    Ok(out)
}

pub(super) fn services_action_verb(action: &ServicesAction) -> &'static str {
    match action {
        ServicesAction::Start(_) => "start",
        ServicesAction::Stop(_) => "stop",
        ServicesAction::Restart(_) => "restart",
        ServicesAction::List | ServicesAction::Status(_) => "list",
    }
}

pub(super) fn validate_systemd_unit(unit: &str) -> Result<(), Failure> {
    if unit.is_empty() || unit.len() > 256 {
        return Err(Failure::new(EXIT_INVALID, "invalid systemd unit name"));
    }
    if unit.contains(';') || unit.contains("..") {
        return Err(Failure::new(EXIT_INVALID, "invalid systemd unit name"));
    }
    Ok(())
}

pub(super) fn guest_ps_vm(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    // `|| true` keeps agent one-shot exec from mapping non-zero `ps` to exec_failed.
    let cmd = [
        "sh",
        "-c",
        "ps -A -o pid= -o user= -o pcpu= -o pmem= -o args= 2>/dev/null || true",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let envelope = exec_vm(id, &cmd, None, &BTreeMap::new(), 30_000, socket_path)?;
    let stdout = envelope["exec"]["stdout"].as_str().unwrap_or("");
    let processes = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() >= 5 {
                json!({
                    "pid": parts[0].parse::<u64>().unwrap_or(0),
                    "user": parts[1],
                    "pcpu": parts[2],
                    "pmem": parts[3],
                    "args": parts[4..].join(" "),
                })
            } else {
                json!({ "raw": line })
            }
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.ps",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("{} process(es)", processes.len()),
            "vm_id": id,
            "processes": processes.len(),
        },
        "processes": processes,
        "exec": envelope["exec"].clone(),
    }))
}
