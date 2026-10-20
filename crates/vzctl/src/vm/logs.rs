use super::*;

pub(super) fn logs_vm(
    id: &str,
    follow: bool,
    tail: usize,
    source: &str,
    list_sources: bool,
    q: Option<&str>,
    min_level: Option<&str>,
    group_field: Option<&str>,
    group_value: Option<&str>,
    filters: &[(String, String)],
    restart: bool,
    socket_path: &Path,
) -> Result<Value, Failure> {
    if list_sources {
        return list_log_sources(id, socket_path);
    }
    if source != "serial" {
        return guest_logs_vm(
            id,
            source,
            follow,
            tail,
            q,
            min_level,
            group_field,
            group_value,
            filters,
            restart,
            socket_path,
        );
    }
    logs_serial_vm(id, follow, tail)
}

pub(super) fn logs_serial_vm(id: &str, follow: bool, tail: usize) -> Result<Value, Failure> {
    validate_vm_id(id)?;
    let bundle = bundle_path(id);
    if !bundle.join("vm.json").is_file() {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("VM bundle not found: {}", bundle.display()),
        ));
    }
    let path = serial_log_path(id);
    if !path.is_file() {
        return Err(Failure::new(
            EXIT_SUPERVISOR,
            format!(
                "serial log missing: {} (is the VM started?)",
                path.display()
            ),
        ));
    }

    let (lines, redacted) = read_serial_tail(&path, tail)?;
    if follow {
        for line in &lines {
            println!("{line}");
        }
        follow_serial_log(&path)?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.logs",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": format!("followed serial log for {id}"),
                "vm_id": id,
                "source": "serial",
                "lines": lines.len(),
                "redacted": redacted,
            },
            "log": {
                "path": path.display().to_string(),
                "source": "serial",
            },
        }));
    }

    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.logs",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("serial log for {id}"),
            "vm_id": id,
            "source": "serial",
            "lines": lines.len(),
            "redacted": redacted,
        },
        "log": {
            "path": path.display().to_string(),
            "source": "serial",
        },
        "lines": lines,
    }))
}

pub(super) fn read_serial_tail(path: &Path, tail: usize) -> Result<(Vec<String>, usize), Failure> {
    let content = fs::read_to_string(path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot read serial log {}: {error}", path.display()),
        )
    })?;
    let all: Vec<(String, bool)> = content.lines().map(|line| redact_log_line(line)).collect();
    let start = all.len().saturating_sub(tail);
    let mut redacted = 0usize;
    let lines = all[start..]
        .iter()
        .map(|(value, hit)| {
            if *hit {
                redacted += 1;
            }
            value.clone()
        })
        .collect();
    Ok((lines, redacted))
}

pub(super) fn follow_serial_log(path: &Path) -> Result<(), Failure> {
    let mut file = File::open(path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot open serial log {}: {error}", path.display()),
        )
    })?;
    file.seek(SeekFrom::End(0)).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("cannot seek serial log {}: {error}", path.display()),
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = String::new();
    loop {
        buffer.clear();
        match reader.read_line(&mut buffer) {
            Ok(0) => {
                thread::sleep(Duration::from_millis(200));
                // Re-open if truncated (VM restart).
                let metadata = fs::metadata(path).ok();
                let pos = reader.stream_position().unwrap_or(0);
                if let Some(meta) = metadata {
                    if pos > meta.len() {
                        let mut file = File::open(path).map_err(|error| {
                            Failure::new(
                                EXIT_SUPERVISOR,
                                format!("cannot reopen serial log {}: {error}", path.display()),
                            )
                        })?;
                        file.seek(SeekFrom::Start(0)).ok();
                        reader = BufReader::new(file);
                    }
                }
            }
            Ok(_) => {
                let line = buffer.trim_end_matches(['\r', '\n']);
                let (value, _) = redact_log_line(line);
                println!("{value}");
            }
            Err(error) => {
                return Err(Failure::new(
                    EXIT_SUPERVISOR,
                    format!("serial log follow read: {error}"),
                ));
            }
        }
    }
}

pub(super) fn journald_units_for_vm(id: &str, socket_path: &Path) -> Vec<String> {
    let (project, _) = match crate::config::split_runtime_vm_id(id) {
        Some(parts) => parts,
        None => return Vec::new(),
    };
    let stacks = match rpc(socket_path, "stacks.list", json!({})) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    for stack in stacks["stacks"].as_array().into_iter().flatten() {
        let path = stack["path"].as_str().unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        if let Ok(environment) = crate::config::validate_path(Path::new(path)) {
            if environment.spec.project == project {
                return crate::config::journald_units_for_runtime_vm(&environment, id);
            }
        }
    }
    Vec::new()
}

pub(super) fn merge_log_sources(id: &str, socket_path: &Path) -> Result<Vec<Value>, Failure> {
    validate_vm_id(id)?;
    let mut sources = Vec::new();
    for unit in journald_units_for_vm(id, socket_path) {
        sources.push(json!({
            "name": unit,
            "kind": "journald",
        }));
    }
    let result = rpc(
        socket_path,
        "vm.agent.services.list",
        json!({ "vm_id": id }),
    )?;
    for service in result["services"].as_array().into_iter().flatten() {
        let name = service["name"].as_str().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let kind = service["kind"].as_str().unwrap_or("iwatch");
        if kind != "iwatch" {
            continue;
        }
        if sources
            .iter()
            .any(|entry| entry["name"].as_str() == Some(name))
        {
            continue;
        }
        let mut entry = service.clone();
        if entry.get("kind").is_none() {
            entry["kind"] = json!("iwatch");
        }
        sources.push(entry);
    }
    Ok(sources)
}

pub(super) fn log_source_kind(id: &str, source: &str, socket_path: &Path) -> Option<String> {
    if journald_units_for_vm(id, socket_path)
        .iter()
        .any(|unit| unit == source)
    {
        return Some("journald".to_string());
    }
    let result = rpc(
        socket_path,
        "vm.agent.services.list",
        json!({ "vm_id": id }),
    );
    if let Ok(value) = result {
        for service in value["services"].as_array().into_iter().flatten() {
            if service["name"].as_str() == Some(source) {
                return Some(service["kind"].as_str().unwrap_or("iwatch").to_string());
            }
        }
    }
    if crate::config::is_valid_systemd_unit(source) {
        Some("journald".to_string())
    } else {
        None
    }
}

pub(super) fn list_log_sources(id: &str, socket_path: &Path) -> Result<Value, Failure> {
    let sources = merge_log_sources(id, socket_path)?;
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.logs",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("log sources for {id}"),
            "vm_id": id,
        },
        "sources": sources,
    }))
}

/// POST iwatch `/api/restart` through the published guest service.
pub(super) fn restart_guest_log_source(
    id: &str,
    source: &str,
    socket_path: &Path,
) -> Result<(), Failure> {
    let result = rpc(
        socket_path,
        "vm.agent.services.http",
        json!({
            "vm_id": id,
            "name": source,
            "method": "POST",
            "path": "/api/restart",
        }),
    )?;
    let status = result["status"].as_u64().unwrap_or(0);
    if status == 404 {
        return Err(Failure::new(
            EXIT_GUEST,
            format!("log source {source} not found"),
        ));
    }
    if !(200..300).contains(&status) {
        let body = decode_services_body(&result).unwrap_or_default();
        let detail = String::from_utf8_lossy(&body);
        let detail = detail.trim();
        if detail.is_empty() {
            return Err(Failure::new(
                EXIT_GUEST,
                format!("restart {source} failed (HTTP {status})"),
            ));
        }
        return Err(Failure::new(
            EXIT_GUEST,
            format!("restart {source} failed (HTTP {status}): {detail}"),
        ));
    }
    Ok(())
}

pub(super) fn guest_logs_vm(
    id: &str,
    source: &str,
    follow: bool,
    tail: usize,
    q: Option<&str>,
    min_level: Option<&str>,
    group_field: Option<&str>,
    group_value: Option<&str>,
    filters: &[(String, String)],
    restart: bool,
    socket_path: &Path,
) -> Result<Value, Failure> {
    validate_vm_id(id)?;
    if restart {
        if log_source_kind(id, source, socket_path).as_deref() == Some("journald") {
            return Err(Failure::new(
                EXIT_INVALID,
                format!("cannot restart journald source {source}"),
            ));
        }
        restart_guest_log_source(id, source, socket_path)?;
        if !follow {
            return Ok(json!({
                "apiVersion": API_VERSION,
                "command": "vm.logs",
                "status": "ok",
                "exit_code": 0,
                "summary": {
                    "message": format!("restarted {source} on {id}"),
                    "vm_id": id,
                    "source": source,
                    "restarted": true,
                },
                "source": source,
                "restarted": true,
            }));
        }
    }
    if follow {
        follow_guest_logs(
            id,
            source,
            tail,
            q,
            min_level,
            group_field,
            group_value,
            filters,
            socket_path,
        )?;
        return Ok(json!({
            "apiVersion": API_VERSION,
            "command": "vm.logs",
            "status": "ok",
            "exit_code": 0,
            "summary": {
                "message": format!("followed {source} logs for {id}"),
                "vm_id": id,
                "source": source,
            },
        }));
    }

    let path = iwatch_logs_path(false, tail, q, min_level, group_field, group_value, filters);
    let result = rpc(
        socket_path,
        "vm.agent.services.http",
        json!({
            "vm_id": id,
            "name": source,
            "method": "GET",
            "path": path,
        }),
    )?;
    let status = result["status"].as_u64().unwrap_or(0);
    if status == 404 {
        return Err(Failure::new(
            EXIT_GUEST,
            format!("log source {source} not found"),
        ));
    }
    let body = decode_services_body(&result)?;
    let parsed: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let lines = parsed
        .get("lines")
        .cloned()
        .or_else(|| parsed.get("items").cloned())
        .unwrap_or(parsed);
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.logs",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("{source} logs for {id}"),
            "vm_id": id,
            "source": source,
        },
        "source": source,
        "lines": lines,
    }))
}

pub(super) fn follow_guest_logs(
    id: &str,
    source: &str,
    tail: usize,
    q: Option<&str>,
    min_level: Option<&str>,
    group_field: Option<&str>,
    group_value: Option<&str>,
    filters: &[(String, String)],
    socket_path: &Path,
) -> Result<(), Failure> {
    let path = iwatch_logs_path(true, tail, q, min_level, group_field, group_value, filters);
    let result = rpc(
        socket_path,
        "vm.agent.services.stream",
        json!({
            "vm_id": id,
            "name": source,
            "method": "GET",
            "path": path,
        }),
    )?;
    let helper_socket = result["socket"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_GUEST, "guest log stream did not return a socket"))?;
    let stream = UnixStream::connect(helper_socket).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!("guest log stream {helper_socket}: {error}"),
        )
    })?;
    let _ = stream.set_read_timeout(None);
    let mut reader = BufReader::new(stream);
    let mut event = String::new();
    let mut data = String::new();
    loop {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| Failure::new(EXIT_GUEST, format!("log stream read: {error}")))?;
        if bytes == 0 {
            return Ok(());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if event == "line" || event.is_empty() {
                print_iwatch_line(&data);
            }
            event.clear();
            data.clear();
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("event:") {
            event = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        }
    }
}

pub(super) fn print_iwatch_line(data: &str) {
    if data.is_empty() {
        return;
    }
    if let Ok(value) = serde_json::from_str::<Value>(data) {
        println!("{}", format_iwatch_line(&value));
        return;
    }
    println!("{data}");
}

pub(super) fn format_iwatch_line(line: &Value) -> String {
    if let Some(text) = line.as_str() {
        return text.to_string();
    }
    let text = line["text"].as_str().unwrap_or("");
    let source = line["source"].as_str().unwrap_or("");
    let level = line["level"].as_str().unwrap_or("");
    match (source.is_empty(), level.is_empty()) {
        (true, true) => text.to_string(),
        (false, true) => format!("source={source} {text}"),
        (true, false) => format!("level={level} {text}"),
        (false, false) => format!("source={source} level={level} {text}"),
    }
}

pub(super) fn decode_services_body(result: &Value) -> Result<Vec<u8>, Failure> {
    let encoded = result["body_b64"].as_str().unwrap_or("");
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|error| Failure::new(EXIT_GUEST, format!("invalid guest log body: {error}")))
}

pub(super) fn iwatch_logs_path(
    sse: bool,
    tail: usize,
    q: Option<&str>,
    min_level: Option<&str>,
    group_field: Option<&str>,
    group_value: Option<&str>,
    filters: &[(String, String)],
) -> String {
    let mut path = if sse {
        "/api/logs/sse".to_string()
    } else {
        "/api/logs".to_string()
    };
    let mut query = Vec::new();
    if sse {
        query.push(format!("tail={tail}"));
    } else {
        query.push(format!("limit={tail}"));
    }
    if let Some(value) = q {
        query.push(format!("q={}", urlencoding_plus(value)));
    }
    if let Some(value) = min_level {
        query.push(format!("minLevel={}", urlencoding_plus(value)));
    }
    if let Some(value) = group_field {
        query.push(format!("groupField={}", urlencoding_plus(value)));
    }
    if let Some(value) = group_value {
        query.push(format!("groupValue={}", urlencoding_plus(value)));
    }
    for (key, value) in filters {
        query.push(format!(
            "filter.{}={}",
            urlencoding_plus(key),
            urlencoding_plus(value)
        ));
    }
    path.push('?');
    path.push_str(&query.join("&"));
    path
}

pub(super) fn urlencoding_plus(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub(super) fn redact_log_line(line: &str) -> (String, bool) {
    let lower = line.to_ascii_lowercase();
    if lower.contains("password") || lower.contains("chpasswd") || lower.contains("root_password") {
        ("[redacted]".to_string(), true)
    } else {
        (line.to_string(), false)
    }
}

pub(super) fn serial_log_path(id: &str) -> PathBuf {
    logs_dir().join(format!("{}.serial.log", state_file_component(id)))
}

pub(super) fn logs_dir() -> PathBuf {
    if let Some(directory) = std::env::var_os("VZCTL_LOGS_DIR") {
        return PathBuf::from(directory);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Logs/vzctl")
}
