use super::*;

/// Retry `run_self` while `retryable` matches, until `budget` elapses.
pub(super) fn run_self_retrying(
    args: &[&str],
    budget: Duration,
    retryable: impl Fn(&Failure) -> bool,
    progress: &mut ProgressReporter,
) -> Result<Value, Failure> {
    let deadline = Instant::now() + budget;
    loop {
        match run_self(args, progress) {
            Ok(value) => return Ok(value),
            Err(failure) if retryable(&failure) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(failure) => return Err(failure),
        }
    }
}

pub(super) fn checkpoint(
    socket_path: &Path,
    id: &str,
    stack_id: &str,
    holder: &str,
    step: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), Failure> {
    rpc(
        socket_path,
        "stack.step",
        json!({
            "id": id,
            "stack_id": stack_id,
            "holder": holder,
            "step": step,
            "status": status,
            "error": error,
        }),
    )
    .map(|_| ())
}

/// Retry supervisor RPCs while transient edge-reconcile failures occur.
pub(super) fn rpc_retrying(
    socket_path: &Path,
    method: &str,
    params: Value,
    guard: &dyn Fn() -> Result<(), Failure>,
) -> Result<Value, Failure> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        guard()?;
        match rpc(socket_path, method, params.clone()) {
            Ok(value) => return Ok(value),
            Err(failure)
                if rpc_transient_failure(&failure.message) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(failure) => return Err(failure),
        }
    }
}

pub(super) fn rpc_transient_failure(message: &str) -> bool {
    message.contains("os error 35")
        || message.contains("WouldBlock")
        || message.contains("Timed out")
        || message.contains("timeout")
        || message.contains("Connection reset")
        || message.contains("Connection refused")
        || message.contains("vz-edge")
        || message.contains("dns-bind")
        || message.contains("did not respond")
        || message.contains("Resource temporarily unavailable")
}

pub(super) fn edge_reconcile_rpc_timeout_secs(method: &str) -> Option<u64> {
    match method {
        "ingress.ensure"
        | "ingress.purge"
        | "oidc.ensure"
        | "oidc.purge"
        | "dns.host_services.ensure"
        | "dns.records.ensure"
        | "dns.records.purge"
        | "port.ensure"
        | "port.purge" => Some(90),
        _ => None,
    }
}

pub(super) fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("connect {}: {error}", socket_path.display()),
        )
    })?;
    let timeout_secs = rpc_timeout_secs(method, &params);
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout_secs)))
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(timeout_secs.min(10))))
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    let request = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
    writeln!(stream, "{request}")
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("{method}: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("{method}: {error}")))?;
    let response: Value = serde_json::from_str(&line)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("{method}: {error}")))?;
    if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
        let code = match error["code"].as_i64() {
            Some(5) => EXIT_INCOMPLETE,
            Some(6) => EXIT_LEASE,
            _ => EXIT_STEP,
        };
        return Err(Failure::new(
            code,
            error["message"].as_str().unwrap_or("supervisor error"),
        ));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, format!("{method}: missing result")))
}

pub(super) fn rpc_timeout_secs(method: &str, params: &Value) -> u64 {
    if method == "vm.start" {
        // Same bounded lock handoff as direct vm start/restart: 5s + 5s,
        // plus transport/scheduling headroom.
        20
    } else if method == "vm.exec" || method.starts_with("vm.agent.") {
        params
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(|ms| (ms / 1000).saturating_add(10).max(15))
            .unwrap_or(40)
    } else if let Some(timeout) = edge_reconcile_rpc_timeout_secs(method) {
        timeout
    } else {
        10
    }
}

pub(super) fn run_self(args: &[&str], progress: &mut ProgressReporter) -> Result<Value, Failure> {
    let executable = std::env::current_exe()
        .map_err(|error| Failure::new(EXIT_STEP, format!("resolve vzctl executable: {error}")))?;
    let pipe_stderr = progress.pipe_subprocess_stderr();
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if pipe_stderr {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .map_err(|error| {
            Failure::new(EXIT_STEP, format!("run vzctl {}: {error}", args.join(" ")))
        })?;
    let stderr_thread = if pipe_stderr {
        child.stderr.take().map(|stderr| {
            let tx = progress.log_sender();
            let job_id = progress.current_job_id();
            let job_path = progress.current_job_path();
            let plain_percent = progress.plain_percent();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(tx) = &tx {
                        let _ = tx.send(ProgressMessage::Log {
                            job_id: job_id.clone(),
                            line,
                        });
                    } else if let Some(percent) = plain_percent {
                        progress::print_plain_subprocess_line(percent, job_path.as_deref(), &line);
                    }
                }
            })
        })
    } else {
        None
    };
    let mut stdout = Vec::new();
    child
        .stdout
        .as_mut()
        .ok_or_else(|| {
            Failure::new(
                EXIT_STEP,
                format!("vzctl {}: missing stdout", args.join(" ")),
            )
        })?
        .read_to_end(&mut stdout)
        .map_err(|error| {
            Failure::new(
                EXIT_STEP,
                format!("read vzctl {} stdout: {error}", args.join(" ")),
            )
        })?;
    let status = child.wait().map_err(|error| {
        Failure::new(EXIT_STEP, format!("wait vzctl {}: {error}", args.join(" ")))
    })?;
    if let Some(handle) = stderr_thread {
        let _ = handle.join();
    }
    if !status.success() {
        let message = serde_json::from_slice::<Value>(&stdout)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/summary/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("vzctl {} failed", args.join(" ")));
        return Err(Failure::new(
            status.code().unwrap_or(EXIT_STEP as i32) as u8,
            message,
        ));
    }
    serde_json::from_slice(&stdout).map_err(|error| {
        Failure::new(
            EXIT_STEP,
            format!("vzctl {} returned invalid JSON: {error}", args.join(" ")),
        )
    })
}

/// Like `run_self`, but retries via macOS Admin dialog when `/etc/resolver` needs root.
pub(super) fn run_self_privileged(
    args: &[&str],
    progress: &mut ProgressReporter,
) -> Result<Value, Failure> {
    match run_self(args, progress) {
        Ok(value) => Ok(value),
        Err(failure) if failure_needs_dns_elevation(&failure) => run_self_elevated(args),
        Err(failure) => Err(failure),
    }
}

pub(super) fn failure_needs_dns_elevation(failure: &Failure) -> bool {
    failure.code == crate::dns::EXIT_RESOLVER
        || failure.message.contains("Permission denied")
        || failure.message.contains("run this command with sudo")
        || failure.message.contains("os error 13")
}

pub(super) fn run_self_elevated(args: &[&str]) -> Result<Value, Failure> {
    let executable = std::env::current_exe()
        .map_err(|error| Failure::new(EXIT_STEP, format!("resolve vzctl executable: {error}")))?;
    let shell = std::iter::once(executable.to_string_lossy().as_ref())
        .chain(args.iter().copied())
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        escape_applescript(&shell)
    );
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| Failure::new(EXIT_STEP, format!("osascript elevate: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Failure::new(
            crate::dns::EXIT_RESOLVER,
            if stderr.is_empty() {
                format!(
                    "Admin elevation failed or cancelled for: vzctl {}",
                    args.join(" ")
                )
            } else {
                stderr
            },
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.get("status").and_then(|s| s.as_str()) == Some("fail")
            || value
                .get("exit_code")
                .and_then(|c| c.as_u64())
                .is_some_and(|c| c != 0)
        {
            let message = value
                .pointer("/summary/message")
                .and_then(Value::as_str)
                .unwrap_or("elevated dns command failed")
                .to_string();
            return Err(Failure::new(crate::dns::EXIT_RESOLVER, message));
        }
        return Ok(value);
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start..=end]) {
                    return Ok(value);
                }
            }
        }
    }
    Ok(json!({
        "apiVersion": API_VERSION,
        "status": "ok",
        "exit_code": 0,
        "summary": {"message": if trimmed.is_empty() { "elevated ok" } else { trimmed }},
    }))
}

pub(super) fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".into();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:@+=".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

pub(super) fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
