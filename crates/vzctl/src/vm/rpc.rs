use super::*;

pub(super) fn rpc_timeout_secs(method: &str, params: &Value) -> u64 {
    if method == "vm.start" {
        // Supervisor permits 5s old-helper release + 5s new-helper claim.
        // Leave transport/scheduling headroom so its result reaches the caller.
        20
    } else if method == "vm.exec" || method.starts_with("vm.agent.") {
        params
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(|ms| (ms / 1000).saturating_add(10).max(15))
            .unwrap_or(40)
    } else {
        5
    }
}

pub(super) fn rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, Failure> {
    let timeout_secs = rpc_timeout_secs(method, &params);
    rpc_with_timeout(socket_path, method, params, timeout_secs)
}

pub(super) fn rpc_with_timeout(
    socket_path: &Path,
    method: &str,
    params: Value,
    timeout_secs: u64,
) -> Result<Value, Failure> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("supervisor socket {}: {error}", socket_path.display()),
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout_secs)))
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(timeout_secs.min(10)))))
        .map_err(|error| {
            Failure::new(
                EXIT_SUPERVISOR,
                format!("supervisor timeout setup: {error}"),
            )
        })?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    writeln!(stream, "{request}")
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor request: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("supervisor response: {error}")))?;
    let response: Value = serde_json::from_str(&line).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("invalid supervisor response: {error}"),
        )
    })?;
    if let Some(error) = response.get("error") {
        let rpc_code = error["code"].as_i64().unwrap_or(-32031);
        let code = if rpc_code == -32602 {
            EXIT_INVALID
        } else if method.starts_with("net.") {
            EXIT_NETWORK
        } else if method == "vm.exec" || method.starts_with("vm.agent.") {
            EXIT_GUEST
        } else {
            EXIT_VM_OP
        };
        return Err(Failure::new(code, {
            let message = error["message"]
                .as_str()
                .unwrap_or("VM operation failed")
                .to_string();
            if message == "Method not found"
                && (method == "vm.exec" || method.starts_with("vm.agent."))
            {
                format!(
                    "helper does not support {method}; restart the VM \
                         (`vzctl vm stop <id> && vzctl vm start <id>`) after upgrading vz-helper"
                )
            } else {
                message
            }
        }));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| Failure::new(EXIT_SUPERVISOR, "supervisor response has no result"))
}
