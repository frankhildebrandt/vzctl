use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

mod api_proxy;
mod terminal;

struct EventBridge {
    /// Reserved for future process-backed bridges.
    _child: Mutex<Option<Child>>,
}

static EVENTS_SUBSCRIBED: AtomicBool = AtomicBool::new(false);

/// PIDs of in-flight streamed vzctl children — never reap these for lease recovery.
fn protected_pids() -> &'static Mutex<HashSet<u32>> {
    static PROTECTED_PIDS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    PROTECTED_PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

#[tauri::command]
fn api_base_url() -> Result<String, String> {
    // Kept for diagnostics; UI uses api_request invoke (WKWebView cannot fetch localhost).
    Ok(match std::env::var("VZCTL_API_LISTEN") {
        Ok(v) => v,
        Err(_) => format!(
            "unix:{}",
            terminal::vzctl_state_dir_path().join("api.sock").display()
        ),
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiRequestArgs {
    method: String,
    path: String,
    headers: Option<Vec<(String, String)>>,
    body: Option<String>,
}

#[tauri::command]
async fn api_request(args: ApiRequestArgs) -> Result<api_proxy::ApiResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let response = api_proxy::request(
            &args.method,
            &args.path,
            args.headers.as_deref().unwrap_or(&[]),
            args.body.as_deref(),
        )?;
        maybe_elevate_privileged_dns(&args.method, &args.path, args.body.as_deref(), response)
    })
    .await
    .map_err(|e| format!("api_request task failed: {e}"))?
}

#[tauri::command]
fn subscribe_events(app: AppHandle, _bridge: State<'_, EventBridge>) -> Result<(), String> {
    if EVENTS_SUBSCRIBED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let stream = api_proxy::open_sse("/v1/events?filter=apply.*,vm.state")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let mut data = String::new();
            loop {
                line.clear();
                let n = reader
                    .read_line(&mut line)
                    .map_err(|e| format!("sse read: {e}"))?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if trimmed.is_empty() {
                    if !data.is_empty() {
                        if let Ok(value) = serde_json::from_str::<Value>(&data) {
                            let _ = app.emit("vzctl-event", value);
                        }
                        data.clear();
                    }
                    continue;
                }
                if let Some(payload) = trimmed.strip_prefix("data:") {
                    let payload = payload.trim_start();
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(payload);
                }
            }
            Ok(())
        })();
        if let Err(err) = result {
            eprintln!("vzctl events sse: {err}");
        }
        EVENTS_SUBSCRIBED.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
async fn run_vzctl(
    app: AppHandle,
    path: String,
    command: String,
    force: Option<bool>,
    purge: Option<bool>,
) -> Result<String, String> {
    let force = force.unwrap_or(false);
    let purge = purge.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        run_vzctl_blocking(app, path, command, force, purge)
    })
    .await
    .map_err(|e| format!("vzctl task failed: {e}"))?
}

#[tauri::command]
async fn run_vzctl_argv(args: Vec<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || run_vzctl_argv_blocking(args))
        .await
        .map_err(|e| format!("vzctl task failed: {e}"))?
}

/// Ask macOS to restart the host (standard System Events restart dialog).
#[tauri::command]
async fn request_host_reboot() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let status = Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\" to restart",
            ])
            .status()
            .map_err(|e| format!("osascript restart failed: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "Host-Neustart abgebrochen oder fehlgeschlagen (status {status})"
            ))
        }
    })
    .await
    .map_err(|e| format!("reboot task failed: {e}"))?
}

fn run_vzctl_argv_blocking(args: Vec<String>) -> Result<String, String> {
    validate_vzctl_argv(&args)?;
    let mut owned = args;
    ensure_json_format(&mut owned);
    let vzctl = which_vzctl()?;
    let arg_refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    let output = run_args(&vzctl, &arg_refs)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        return Ok(pretty_or_raw(&stdout));
    }

    // Privileged DNS bind-helper / resolver installs need Admin.
    if needs_dns_elevation(&owned, &output) {
        let elevated_args = bind_helper_elevated_args(&owned);
        let elevated_refs: Vec<&str> = elevated_args.iter().map(String::as_str).collect();
        match run_elevated(&vzctl, &elevated_refs) {
            Ok(elevated) => {
                if elevated.trim().is_empty() {
                    return Ok(pretty_or_raw(&stdout));
                }
                return Ok(pretty_or_raw(&elevated));
            }
            Err(error) => {
                return Err(format!(
                    "Admin-Rechte nötig für `{}`.\n\
                     Passwort-Dialog abgebrochen oder fehlgeschlagen: {error}\n\n\
                     Alternativ:\n  sudo {} {}",
                    elevated_args.join(" "),
                    vzctl.display(),
                    elevated_args.join(" ")
                ));
            }
        }
    }

    // Prefer JSON envelope when present (status may be fail).
    if let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) {
        if value.get("status").and_then(|s| s.as_str()) == Some("fail")
            || value
                .get("exit_code")
                .and_then(|c| c.as_u64())
                .is_some_and(|c| c != 0)
        {
            return Ok(pretty_or_raw(&stdout));
        }
        return Ok(pretty_or_raw(&stdout));
    }
    let detail = [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "vzctl {} failed ({}){}",
        owned.join(" "),
        output.status,
        if detail.is_empty() {
            String::new()
        } else {
            format!("\n{detail}")
        }
    ))
}

fn needs_dns_elevation(args: &[String], output: &Output) -> bool {
    let is_privileged_dns = matches!(
        (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str)
        ),
        (
            Some("dns"),
            Some(
                "install-bind-helper"
                    | "uninstall-bind-helper"
                    | "install-resolver"
                    | "uninstall-resolver"
            )
        )
    );
    is_privileged_dns && needs_resolver_elevation(output)
}

/// osascript elevation runs as root without SUDO_UID — pass the calling user's uid.
fn bind_helper_elevated_args(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    if out.first().map(String::as_str) != Some("dns")
        || out.get(1).map(String::as_str) != Some("install-bind-helper")
    {
        return out;
    }
    if out.windows(2).any(|pair| pair[0] == "--allow-uid") {
        return out;
    }
    if let Some(uid) = current_uid() {
        out.push("--allow-uid".into());
        out.push(uid.to_string());
    }
    out
}

fn current_uid() -> Option<u32> {
    let output = Command::new("id").arg("-u").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn validate_vzctl_argv(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("vzctl argv is empty".into());
    }
    let group = args[0].as_str();
    match group {
        "doctor" => {
            // `vzctl doctor [--format …] [--min-free-gib N]` — no subcommand.
        }
        "dns" => {
            if args.len() < 2 {
                return Err("dns requires a subcommand".into());
            }
            let allowed = [
                "status",
                "query",
                "install-resolver",
                "uninstall-resolver",
                "install-bind-helper",
                "uninstall-bind-helper",
            ];
            if !allowed.contains(&args[1].as_str()) {
                return Err(format!("unsupported dns subcommand: {}", args[1]));
            }
        }
        "certs" => {
            if args.len() < 2 {
                return Err("certs requires a subcommand".into());
            }
            match args[1].as_str() {
                "fingerprint" => {}
                "ca" => {
                    let action = args.get(2).map(String::as_str);
                    if !matches!(action, Some("init" | "install")) {
                        return Err(
                            "unsupported certs ca subcommand (allowed: init|install)".into(),
                        );
                    }
                }
                other => {
                    return Err(format!("unsupported certs subcommand: {other}"));
                }
            }
        }
        "vm" | "image" => {
            if args.len() < 2 {
                return Err(format!("{group} requires a subcommand"));
            }
            let sub = args[1].as_str();
            let allowed_vm = [
                "list", "start", "stop", "delete", "create", "modify", "inspect", "mount",
                "unmount", "mounts", "logs", "exec", "services", "ps", "transfer",
            ];
            let allowed_image = ["list", "pull", "bake", "seal"];
            let allowed = if group == "vm" {
                &allowed_vm[..]
            } else {
                &allowed_image[..]
            };
            if !allowed.contains(&sub) {
                return Err(format!("unsupported {group} subcommand: {sub}"));
            }
        }
        "docker" => {
            if args.len() < 2 {
                return Err("docker requires a subcommand".into());
            }
            let allowed = ["ps", "inspect", "start", "stop", "restart", "run"];
            if !allowed.contains(&args[1].as_str()) {
                return Err(format!("unsupported docker subcommand: {}", args[1]));
            }
        }
        other => return Err(format!("unsupported argv group: {other}")),
    }
    // Block interactive / streaming modes — use the terminal bridge instead.
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "attach" | "-it" | "-i" | "--interactive" | "-t" | "--tty" | "-f" | "--follow" => {
                return Err(format!("interactive/streaming flag not allowed via argv: {arg}"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn ensure_json_format(args: &mut Vec<String>) {
    let has_format = args.windows(2).any(|pair| pair[0] == "--format");
    if has_format {
        return;
    }
    // Keep UI-appended `--format json` before a passthrough `--` so it is not
    // swallowed as container/command args (e.g. `docker run … -- cmd`).
    if let Some(idx) = args.iter().position(|arg| arg == "--") {
        args.insert(idx, "json".into());
        args.insert(idx, "--format".into());
    } else {
        args.push("--format".into());
        args.push("json".into());
    }
}

fn run_vzctl_blocking(
    app: AppHandle,
    path: String,
    command: String,
    force: bool,
    purge: bool,
) -> Result<String, String> {
    let config = PathBuf::from(&path);
    if !config.join("hypernetwork.config.yaml").is_file()
        && !config
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
    {
        return Err(
            "directory must contain hypernetwork.config.yaml (or pass a config file)".into(),
        );
    }

    let vzctl = which_vzctl()?;
    let mut owned_args: Vec<String> = match command.as_str() {
        "diff" => vec![
            "diff".into(),
            "-C".into(),
            path.clone(),
            "--format".into(),
            "json".into(),
        ],
        "up" => vec![
            "up".into(),
            "-C".into(),
            path.clone(),
            "--format".into(),
            "json".into(),
        ],
        "apply" => vec![
            "apply".into(),
            "-C".into(),
            path.clone(),
            "--format".into(),
            "json".into(),
        ],
        "down" => vec![
            "down".into(),
            "-C".into(),
            path.clone(),
            "--format".into(),
            "json".into(),
        ],
        "validate" => vec![
            "validate".into(),
            "-C".into(),
            path.clone(),
            "--format".into(),
            "json".into(),
        ],
        "status" => {
            return status_bundle(&vzctl, &path);
        }
        other => return Err(format!("unsupported command: {other}")),
    };
    if force && matches!(command.as_str(), "up" | "apply") {
        owned_args.insert(1, "--force".into());
    }
    if purge && command == "down" {
        owned_args.insert(1, "--purge".into());
    }
    let args: Vec<&str> = owned_args.iter().map(String::as_str).collect();

    let stream = matches!(command.as_str(), "up" | "apply" | "down");
    if stream {
        emit_console(&app, "cmd", &format!("$ vzctl {}", args.join(" ")));
    }

    let output = if stream {
        run_args_streaming(&app, &vzctl, &args)?
    } else {
        run_args(&vzctl, &args)?
    };
    if output.status.success() {
        if stream {
            emit_console(&app, "ok", &format!("vzctl {command} ok"));
        }
        return Ok(pretty_or_raw(&String::from_utf8_lossy(&output.stdout)));
    }

    if matches!(command.as_str(), "up" | "apply" | "down") && is_recoverable(&output) {
        emit_console(&app, "warn", "recoverable failure — retry…");
        return recover_and_retry(&app, &vzctl, &path, &command, &args, &output, 0);
    }

    let message = format_failure(&command, &output);
    if stream {
        emit_console(&app, "fail", &message);
    }
    Err(message)
}

fn is_recoverable(output: &Output) -> bool {
    is_incomplete_journal(output) || is_lease_held(output) || needs_resolver_elevation(output)
}

fn recover_and_retry(
    app: &AppHandle,
    vzctl: &Path,
    path: &str,
    command: &str,
    args: &[&str],
    first: &Output,
    depth: u8,
) -> Result<String, String> {
    if depth > 1 {
        let message = format_failure(command, first);
        emit_console(app, "fail", &message);
        return Err(message);
    }

    if is_incomplete_journal(first) || is_lease_held(first) {
        emit_console(app, "warn", "clear lease / abort journal…");
        clear_lease(vzctl, path, first);
    }

    ensure_resolver(vzctl, path, command, first)?;

    emit_console(app, "info", "retry…");
    let retry = run_args_streaming(app, vzctl, args)?;
    if retry.status.success() {
        emit_console(app, "ok", &format!("vzctl {command} ok (retry)"));
        return Ok(pretty_or_raw(&String::from_utf8_lossy(&retry.stdout)));
    }

    if is_lease_held(&retry) {
        emit_console(app, "warn", "lease still held — reap + abort…");
        clear_lease(vzctl, path, &retry);
        let again = run_args_streaming(app, vzctl, args)?;
        if again.status.success() {
            emit_console(app, "ok", &format!("vzctl {command} ok (retry2)"));
            return Ok(pretty_or_raw(&String::from_utf8_lossy(&again.stdout)));
        }
        let message = format!(
            "Stack-Lease noch belegt.\n\
             Warte bis zum Lease-Ende oder im Terminal:\n\
               {} apply --abort -C {}\n\
               {} {} -C {}\n\n{}",
            vzctl.display(),
            path,
            vzctl.display(),
            command,
            path,
            format_failure(command, &again)
        );
        emit_console(app, "fail", &message);
        return Err(message);
    }

    if is_recoverable(&retry) && !is_lease_held(&retry) {
        return recover_and_retry(app, vzctl, path, command, args, &retry, depth + 1);
    }

    let message = format_failure(command, &retry);
    emit_console(app, "fail", &message);
    Err(message)
}

fn clear_lease(vzctl: &Path, path: &str, output: &Output) {
    if abort_journal(vzctl, path).is_ok() {
        return;
    }

    // A previous UI/CLI up can hang past the lease TTL while the PID stays
    // alive — abort then refuses. Reap only local up/apply holders (never
    // events subscribe / image seal / the current process), then abort again.
    if let Some(pid) = holder_pid(output) {
        if should_reap_lease_holder(pid) {
            emit_reap(pid);
            let _ = Command::new("kill").args([pid.to_string()]).status();
            thread::sleep(Duration::from_millis(400));
            // Also stop orphaned seal children left by the killed up.
            reap_orphaned_seals();
            if abort_journal(vzctl, path).is_ok() {
                return;
            }
        }
    }

    if let Some(wait) = lease_wait(output) {
        thread::sleep(wait);
        if let Some(pid) = holder_pid(output) {
            if should_reap_lease_holder(pid) {
                emit_reap(pid);
                let _ = Command::new("kill").args([pid.to_string()]).status();
                thread::sleep(Duration::from_millis(400));
                reap_orphaned_seals();
            }
        }
        let _ = abort_journal(vzctl, path);
    } else if is_lease_held(output) {
        thread::sleep(Duration::from_secs(2));
        let _ = abort_journal(vzctl, path);
    }
}

fn emit_reap(pid: i32) {
    // Best-effort stderr for tauri console when AppHandle isn't in scope.
    eprintln!("reaping hung lease holder pid={pid}");
}

fn should_reap_lease_holder(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    if pid as u32 == std::process::id() {
        return false;
    }
    if let Ok(guard) = protected_pids().lock() {
        if guard.contains(&(pid as u32)) {
            return false;
        }
    }
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output();
    let Ok(output) = output else {
        return false;
    };
    let cmd = String::from_utf8_lossy(&output.stdout);
    // Only reap stack holders — never event subscribers or bake/seal workers.
    let is_stack = cmd.contains("vzctl up")
        || cmd.contains("vzctl apply")
        || cmd.contains("vzctl down");
    is_stack && !cmd.contains("events subscribe")
}

fn reap_orphaned_seals() {
    let output = Command::new("pgrep")
        .args(["-f", r"vzctl image seal "])
        .output();
    let Ok(output) = output else {
        return;
    };
    for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        let Ok(pid) = pid.parse::<i32>() else {
            continue;
        };
        if pid as u32 == std::process::id() {
            continue;
        }
        let _ = Command::new("kill").args([pid.to_string()]).status();
    }
}

fn holder_pid(output: &Output) -> Option<i32> {
    let text = combined_text(output);
    let marker = "localhost:";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn ensure_resolver(
    vzctl: &Path,
    path: &str,
    command: &str,
    first: &Output,
) -> Result<(), String> {
    if !(needs_resolver_elevation(first)
        || is_incomplete_journal(first)
        || is_lease_held(first))
    {
        return Ok(());
    }

    let resolver_args: &[&str] = match command {
        "down" => &[
            "dns",
            "uninstall-resolver",
            "--config",
            path,
            "--format",
            "json",
        ],
        _ => &[
            "dns",
            "install-resolver",
            "--config",
            path,
            "--format",
            "json",
        ],
    };

    let resolver = run_args(vzctl, resolver_args)?;
    if resolver.status.success() {
        return Ok(());
    }
    if !needs_resolver_elevation(&resolver) {
        if needs_resolver_elevation(first) {
            return Err(format_failure("dns", &resolver));
        }
        return Ok(());
    }

    run_elevated(vzctl, resolver_args).map_err(|error| {
        format!(
            "Host-Resolver braucht Admin-Rechte (/etc/resolver).\n\
             Passwort-Dialog abgebrochen oder fehlgeschlagen: {error}\n\n\
             Alternativ:\n  sudo {} {}",
            vzctl.display(),
            resolver_args.join(" ")
        )
    })?;
    Ok(())
}

fn abort_journal(vzctl: &Path, path: &str) -> Result<(), String> {
    let output = run_args(
        vzctl,
        &["apply", "--abort", "-C", path, "--format", "json"],
    )?;
    if output.status.success()
        || combined_text(&output).contains("no incomplete")
    {
        Ok(())
    } else if is_lease_held(&output) {
        Err(format_failure("apply --abort", &output))
    } else {
        Ok(())
    }
}

fn run_args(vzctl: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new(vzctl)
        .args(args)
        .output()
        .map_err(|e| format!("spawn vzctl {}: {e}", args.join(" ")))
}

fn emit_console(app: &AppHandle, stream: &str, line: &str) {
    for part in line.split('\n') {
        let trimmed = part.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let _ = app.emit(
            "vzctl-console",
            json!({
                "stream": stream,
                "line": trimmed,
            }),
        );
    }
}

fn run_args_streaming(app: &AppHandle, vzctl: &Path, args: &[&str]) -> Result<Output, String> {
    let mut child = Command::new(vzctl)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn vzctl {}: {e}", args.join(" ")))?;

    let child_pid = child.id();
    protect_pid(child_pid);

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "missing stderr".to_string())?;

    let app_out = app.clone();
    let out_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            // JSON envelope stays in buffer only; surface short diagnostics.
            if !line.trim_start().starts_with('{') {
                emit_console(&app_out, "out", &line);
            }
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
        }
        buf
    });

    let app_err = app.clone();
    let err_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            emit_console(&app_err, "err", &line);
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
        }
        buf
    });

    let status = child
        .wait()
        .map_err(|e| format!("wait vzctl {}: {e}", args.join(" ")))?;
    unprotect_pid(child_pid);
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn protect_pid(pid: u32) {
    if let Ok(mut guard) = protected_pids().lock() {
        guard.insert(pid);
    }
}

fn unprotect_pid(pid: u32) {
    if let Ok(mut guard) = protected_pids().lock() {
        guard.remove(&pid);
    }
}

fn is_incomplete_journal(output: &Output) -> bool {
    output.status.code() == Some(5) || combined_text(output).contains("incomplete journal")
}

fn is_lease_held(output: &Output) -> bool {
    if output.status.code() == Some(6) {
        return true;
    }
    let text = combined_text(output);
    text.contains("stack lease held") || text.contains("\"exit_code\":6")
}

fn needs_resolver_elevation(output: &Output) -> bool {
    if output.status.code() == Some(19) {
        return true;
    }
    if output.status.success() {
        return false;
    }
    let text = combined_text(output);
    text_needs_dns_elevation(&text)
}

fn text_needs_dns_elevation(text: &str) -> bool {
    text.contains("Permission denied")
        || text.contains("run this command with sudo")
        || text.contains("os error 13")
        || text.contains("/Library/LaunchDaemons")
        || text.contains("launchctl bootstrap failed")
        || text.contains("/usr/local/libexec/vzctl")
        || text.contains("\"exit_code\":19")
        || text.contains("\"exit_code\": 19")
}

/// Privileged DNS REST routes that the LaunchAgent cannot complete without Admin.
fn is_privileged_dns_route(method: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    match (method.to_ascii_uppercase().as_str(), path) {
        ("POST" | "DELETE", "/v1/dns/resolver") => true,
        ("POST", "/v1/dns/bind-helper") => true,
        _ => false,
    }
}

fn api_response_needs_elevation(response: &api_proxy::ApiResponse) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(response.body.trim()) {
        let exit = value.get("exit_code").and_then(|c| c.as_u64());
        let status = value.get("status").and_then(|s| s.as_str());
        if status == Some("ok") || exit == Some(0) {
            return false;
        }
        if exit == Some(19) {
            return true;
        }
        if let Some(msg) = value
            .pointer("/summary/message")
            .and_then(|m| m.as_str())
            .or_else(|| value.get("message").and_then(|m| m.as_str()))
        {
            if text_needs_dns_elevation(msg) {
                return true;
            }
        }
        if let Some(err) = value.pointer("/error/message").and_then(|m| m.as_str()) {
            if text_needs_dns_elevation(err) {
                return true;
            }
        }
        if status == Some("fail") && text_needs_dns_elevation(&response.body) {
            return true;
        }
    }
    if !(response.status == 403 || response.status >= 400) {
        // HTTP 200 fail-envelope already handled above; bare 200 ok → no elevate.
        return text_needs_dns_elevation(&response.body)
            && response.body.contains("\"status\":\"fail\"");
    }
    text_needs_dns_elevation(&response.body)
}

fn maybe_elevate_privileged_dns(
    method: &str,
    path: &str,
    body: Option<&str>,
    response: api_proxy::ApiResponse,
) -> Result<api_proxy::ApiResponse, String> {
    if !is_privileged_dns_route(method, path) || !api_response_needs_elevation(&response) {
        return Ok(response);
    }
    let vzctl = which_vzctl()?;
    let args = elevated_dns_args_from_rest(method, path, body)?;
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_elevated(&vzctl, &arg_refs) {
        Ok(elevated) => {
            let body = if elevated.trim().is_empty() {
                response.body
            } else {
                elevated
            };
            Ok(api_proxy::ApiResponse {
                status: 200,
                body,
                content_type: Some("application/json".into()),
            })
        }
        Err(error) => Err(format!(
            "Admin-Rechte nötig für `{}`.\n\
             Passwort-Dialog abgebrochen oder fehlgeschlagen: {error}\n\n\
             Alternativ:\n  sudo {} {}",
            args.join(" "),
            vzctl.display(),
            args.join(" ")
        )),
    }
}

fn elevated_dns_args_from_rest(
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<Vec<String>, String> {
    let path_only = path.split('?').next().unwrap_or(path);
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let method = method.to_ascii_uppercase();

    if path_only == "/v1/dns/bind-helper" && method == "POST" {
        return Ok(bind_helper_elevated_args(&[
            "dns".into(),
            "install-bind-helper".into(),
            "--format".into(),
            "json".into(),
        ]));
    }

    if path_only == "/v1/dns/resolver" && matches!(method.as_str(), "POST" | "DELETE") {
        let install = method == "POST";
        let mut args = vec![
            "dns".into(),
            if install {
                "install-resolver".into()
            } else {
                "uninstall-resolver".into()
            },
            "--format".into(),
            "json".into(),
        ];
        let (config, project) = resolver_scope_from_rest(body, query);
        if let Some(config) = config {
            args.push("--config".into());
            args.push(config);
        }
        if let Some(project) = project {
            args.push("--project".into());
            args.push(project);
        }
        if !args.windows(2).any(|w| w[0] == "--config" || w[0] == "--project") {
            return Err("dns resolver elevation needs config or project".into());
        }
        return Ok(args);
    }

    Err(format!("unsupported privileged DNS route {method} {path}"))
}

fn resolver_scope_from_rest(body: Option<&str>, query: &str) -> (Option<String>, Option<String>) {
    let mut config = None;
    let mut project = None;
    if let Some(body) = body {
        if let Ok(value) = serde_json::from_str::<Value>(body) {
            config = value
                .get("config")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            project = value
                .get("project")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let raw = parts.next().unwrap_or("");
        let value = urlencoding_decode(raw);
        if value.is_empty() {
            continue;
        }
        match key {
            "config" if config.is_none() => config = Some(value),
            "project" if project.is_none() => project = Some(value),
            _ => {}
        }
    }
    (config, project)
}

fn urlencoding_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &input[i + 1..i + 3];
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn lease_wait(output: &Output) -> Option<Duration> {
    let text = combined_text(output);
    let marker = "until ";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '"' || c == '}')
        .unwrap_or(rest.len());
    let stamp = rest[..end].trim_matches(|c| c == '"' || c == '\'');
    let expires = parse_rfc3339(stamp)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    if expires <= now {
        return Some(Duration::from_secs(1));
    }
    let wait = (expires - now + 1).min(90);
    Some(Duration::from_secs(wait))
}

fn parse_rfc3339(stamp: &str) -> Option<u64> {
    if stamp.len() < 20 || !stamp.ends_with('Z') {
        return None;
    }
    let year: u64 = stamp.get(0..4)?.parse().ok()?;
    let month: u64 = stamp.get(5..7)?.parse().ok()?;
    let day: u64 = stamp.get(8..10)?.parse().ok()?;
    let hour: u64 = stamp.get(11..13)?.parse().ok()?;
    let min: u64 = stamp.get(14..16)?.parse().ok()?;
    let sec: u64 = stamp.get(17..19)?.parse().ok()?;
    let days = days_from_civil(year as i64, month as i32, day as i32)?;
    Some((days as u64) * 86400 + hour * 3600 + min * 60 + sec)
}

fn days_from_civil(y: i64, m: i32, d: i32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe as i64 - 719468)
}

fn combined_text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn run_elevated(vzctl: &Path, args: &[&str]) -> Result<String, String> {
    let shell = shell_join(vzctl, args);
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        escape_applescript(&shell)
    );
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("osascript: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("exit {}", output.status)
        } else {
            stderr
        })
    }
}

fn shell_join(vzctl: &Path, args: &[&str]) -> String {
    std::iter::once(vzctl.to_string_lossy().as_ref())
        .chain(args.iter().copied())
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
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

fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_failure(command: &str, output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "vzctl {} failed ({})\n{}\n{}",
        command,
        output.status,
        stdout.trim(),
        stderr.trim()
    )
}

fn status_bundle(vzctl: &PathBuf, path: &str) -> Result<String, String> {
    let mut sections = serde_json::Map::new();
    for (label, args) in [
        ("dns", vec!["dns", "status", "--format", "json"]),
        ("certs", vec!["certs", "fingerprint", "--format", "json"]),
        ("oidc", vec!["oidc", "status", "--format", "json"]),
        ("diff", vec!["diff", "-C", path, "--format", "json"]),
    ] {
        let output = Command::new(vzctl)
            .args(&args)
            .output()
            .map_err(|e| format!("spawn vzctl: {e}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let parsed = serde_json::from_str::<Value>(stdout.trim()).ok();
        sections.insert(
            label.to_string(),
            json!({
                "ok": output.status.success(),
                "exit_code": output.status.code(),
                "data": parsed,
                "stderr": stderr.trim(),
            }),
        );
    }

    let config_path = resolve_config_path(path);
    let desired = config_path
        .as_ref()
        .map(|p| desired_vm_ids_from_config(p))
        .unwrap_or_default();
    let vm_list = Command::new(vzctl)
        .args(["vm", "list", "--format", "json"])
        .output()
        .ok();
    let vms_value = vm_list
        .as_ref()
        .and_then(|o| serde_json::from_str::<Value>(String::from_utf8_lossy(&o.stdout).trim()).ok());
    let all_vms = vms_value
        .as_ref()
        .and_then(|v| v.get("vms"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let project_name = config_path
        .as_ref()
        .and_then(|p| read_project_name(p));

    let mut items = Vec::new();
    let mut running = 0u32;
    let mut starting = 0u32;
    let mut stopping = 0u32;
    let mut stopped = 0u32;
    let mut missing = 0u32;
    let mut other = 0u32;

    for short_id in &desired {
        let runtime_id = project_name
            .as_ref()
            .map(|project| format!("{project}/{short_id}"))
            .unwrap_or_else(|| short_id.clone());
        let found = find_listed_vm(&all_vms, &runtime_id, short_id);
        let resolved_id = found
            .and_then(|vm| vm.get("id").and_then(|v| v.as_str()))
            .unwrap_or(runtime_id.as_str())
            .to_string();
        let state = found
            .and_then(|vm| vm.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("missing");
        match state {
            "running" => running += 1,
            "starting" => starting += 1,
            "stopping" => stopping += 1,
            "stopped" => stopped += 1,
            "missing" => missing += 1,
            _ => other += 1,
        }
        items.push(json!({
            "id": resolved_id,
            "name": short_id,
            "state": state,
            "present": found.is_some(),
        }));
    }

    let desired_n = desired.len() as u32;
    let phase = inventory_phase(desired_n, running, starting, stopping, stopped, missing, other);

    sections.insert(
        "stack".to_string(),
        json!({
            "ok": true,
            "data": {
                "phase": phase,
                "label": phase_label(phase),
                "stack_id": sections
                    .get("diff")
                    .and_then(|d| d.get("data"))
                    .and_then(|d| d.get("stack_id"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "project": project_name
                    .map(Value::String)
                    .unwrap_or(Value::Null),
                "vms": {
                    "desired": desired_n,
                    "running": running,
                    "starting": starting,
                    "stopping": stopping,
                    "stopped": stopped,
                    "missing": missing,
                    "other": other,
                },
                "items": items,
            }
        }),
    );

    let ingress = config_path
        .as_ref()
        .map(|p| ingress_from_config(p))
        .unwrap_or_else(|| json!({ "enabled": false, "routes": [] }));
    sections.insert(
        "ingress".to_string(),
        json!({
            "ok": ingress.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
            "data": ingress,
        }),
    );

    Ok(pretty_or_raw(
        &serde_json::to_string(&json!({
            "apiVersion": "vzctl.dev/v1",
            "command": "status.bundle",
            "status": "ok",
            "sections": sections,
        }))
        .unwrap_or_else(|_| "{}".into()),
    ))
}

fn resolve_config_path(path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(path);
    if p.is_file() {
        return Some(p);
    }
    let candidate = p.join("hypernetwork.config.yaml");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

fn find_listed_vm<'a>(
    all_vms: &'a [Value],
    runtime_id: &str,
    short_id: &str,
) -> Option<&'a Value> {
    if let Some(vm) = all_vms
        .iter()
        .find(|vm| vm.get("id").and_then(|v| v.as_str()) == Some(runtime_id))
    {
        return Some(vm);
    }
    if runtime_id != short_id {
        if let Some(vm) = all_vms
            .iter()
            .find(|vm| vm.get("id").and_then(|v| v.as_str()) == Some(short_id))
        {
            return Some(vm);
        }
    }
    let suffix = format!("/{short_id}");
    all_vms.iter().find(|vm| {
        vm.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id.ends_with(&suffix))
    })
}

fn desired_vm_ids_from_config(config_path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    let mut in_vms = false;
    let mut vms_indent: Option<usize> = None;
    for line in text.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let content = trimmed_end.trim_start();
        if content.starts_with('#') {
            continue;
        }
        if !in_vms {
            if content == "vms:" || content.starts_with("vms:") {
                in_vms = true;
                vms_indent = Some(indent);
            }
            continue;
        }
        let base = vms_indent.unwrap_or(0);
        if indent <= base {
            break;
        }
        // Direct children of `vms:` look like `  router:`.
        if indent == base + 2 && content.ends_with(':') && !content.starts_with('-') {
            let key = content.trim_end_matches(':').trim();
            if !key.is_empty() && !key.contains(' ') && !key.contains('{') {
                ids.push(key.to_string());
            }
        }
    }
    ids
}

fn read_project_name(config_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config_path).ok()?;
    for line in text.lines() {
        let content = line.trim();
        if let Some(rest) = content.strip_prefix("project:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn ingress_from_config(config_path: &Path) -> Value {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return json!({ "enabled": false, "routes": [] });
    };
    let mut in_ingress = false;
    let mut ingress_indent: Option<usize> = None;
    let mut in_routes = false;
    let mut routes_indent: Option<usize> = None;
    let mut enabled = true;
    let mut host_aliases = true;
    let mut https_port: u16 = 443;
    let mut routes: Vec<Value> = Vec::new();
    let mut current: Option<serde_json::Map<String, Value>> = None;

    let flush = |routes: &mut Vec<Value>, current: &mut Option<serde_json::Map<String, Value>>| {
        if let Some(map) = current.take() {
            if map.get("host").and_then(|v| v.as_str()).is_some_and(|h| !h.is_empty()) {
                routes.push(Value::Object(map));
            }
        }
    };

    for line in text.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let content = trimmed_end.trim_start();
        if content.starts_with('#') {
            continue;
        }

        if !in_ingress {
            if content == "ingress:" || content.starts_with("ingress:") {
                in_ingress = true;
                ingress_indent = Some(indent);
            }
            continue;
        }

        let base = ingress_indent.unwrap_or(0);
        if indent <= base {
            break;
        }

        if in_routes {
            let rbase = routes_indent.unwrap_or(base + 2);
            if indent <= rbase {
                flush(&mut routes, &mut current);
                in_routes = false;
                // fall through to handle sibling keys under ingress
            } else if content.starts_with("- ") {
                flush(&mut routes, &mut current);
                let mut map = serde_json::Map::new();
                let rest = content.trim_start_matches("- ").trim();
                if let Some((k, v)) = rest.split_once(':') {
                    let key = k.trim();
                    let value = yaml_scalar(v.trim());
                    if key == "host" || key == "to" {
                        map.insert(key.to_string(), Value::String(value));
                    } else if key == "requires" {
                        map.insert("requires".into(), parse_requires_inline(v.trim()));
                    }
                }
                current = Some(map);
                continue;
            } else if let Some(map) = current.as_mut() {
                if let Some((k, v)) = content.split_once(':') {
                    let key = k.trim();
                    let value = yaml_scalar(v.trim());
                    if key == "host" || key == "to" {
                        map.insert(key.to_string(), Value::String(value));
                    } else if key == "requires" {
                        map.insert("requires".into(), parse_requires_inline(v.trim()));
                    }
                }
                continue;
            }
        }

        if content == "routes:" || content.starts_with("routes:") {
            flush(&mut routes, &mut current);
            in_routes = true;
            routes_indent = Some(indent);
            continue;
        }
        if let Some((k, v)) = content.split_once(':') {
            match k.trim() {
                "enabled" => enabled = yaml_bool(v.trim(), true),
                "hostAliases" => host_aliases = yaml_bool(v.trim(), true),
                "httpsPort" => {
                    if let Ok(port) = v.trim().parse::<u16>() {
                        https_port = port;
                    }
                }
                _ => {}
            }
        }
    }
    flush(&mut routes, &mut current);

    let mut enriched = Vec::new();
    for route in routes {
        let host = route
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if host.is_empty() {
            continue;
        }
        let to = route
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let requires = route
            .get("requires")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![]));
        let url = https_url(&host, https_port);
        let alias = if host_aliases {
            host.split('.').next().map(|short| {
                let alias_host = format!("{short}.localhost");
                json!({
                    "host": alias_host,
                    "url": https_url(&format!("{short}.localhost"), https_port),
                })
            })
        } else {
            None
        };
        enriched.push(json!({
            "host": host,
            "url": url,
            "to": to,
            "requires": requires,
            "alias": alias,
        }));
    }

    json!({
        "enabled": enabled,
        "https_port": https_port,
        "host_aliases": host_aliases,
        "routes": enriched,
    })
}

fn https_url(host: &str, port: u16) -> String {
    if port == 443 {
        format!("https://{host}")
    } else {
        format!("https://{host}:{port}")
    }
}

fn yaml_scalar(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn yaml_bool(raw: &str, default: bool) -> bool {
    match yaml_scalar(raw).to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => true,
        "false" | "no" | "off" => false,
        _ => default,
    }
}

fn parse_requires_inline(raw: &str) -> Value {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let items = inner
            .split(',')
            .map(|part| yaml_scalar(part.trim()))
            .filter(|s| !s.is_empty())
            .map(Value::String)
            .collect::<Vec<_>>();
        return Value::Array(items);
    }
    let one = yaml_scalar(trimmed);
    if one.is_empty() {
        Value::Array(vec![])
    } else {
        Value::Array(vec![Value::String(one)])
    }
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs are allowed".into());
    }
    let status = Command::new("open")
        .arg(&url)
        .status()
        .map_err(|e| format!("open url: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open failed ({status})"))
    }
}

fn inventory_phase(
    desired: u32,
    running: u32,
    starting: u32,
    stopping: u32,
    _stopped: u32,
    _missing: u32,
    other: u32,
) -> &'static str {
    if desired == 0 {
        return "unknown";
    }
    if starting > 0 {
        return "starting";
    }
    if stopping > 0 {
        return "stopping";
    }
    if running == desired && other == 0 && _missing == 0 {
        return "running";
    }
    if running == 0 {
        return "down";
    }
    "partial"
}

fn phase_label(phase: &str) -> &'static str {
    match phase {
        "down" => "Down",
        "starting" => "Starting",
        "stopping" => "Stopping",
        "reconciling" => "Up (Reconciling)",
        "running" => "Up (Running)",
        "partial" => "Up (Partial)",
        "failed" => "Failed",
        _ => "Unknown",
    }
}

fn which_vzctl() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("VZCTL_BIN") {
        return Ok(PathBuf::from(path));
    }
    which("vzctl").or_else(|_| {
        let candidates = [
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/vzctl"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/release/vzctl"),
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/bin/vzctl"),
        ];
        candidates
            .into_iter()
            .find(|path| path.exists())
            .ok_or_else(|| "vzctl not found on PATH; set VZCTL_BIN".into())
    })
}

fn which(bin: &str) -> Result<PathBuf, String> {
    let output = Command::new("which")
        .arg(bin)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(PathBuf::from(
            String::from_utf8_lossy(&output.stdout).trim(),
        ))
    } else {
        Err(format!("{bin} not found"))
    }
}

fn pretty_or_raw(raw: &str) -> String {
    match serde_json::from_str::<Value>(raw.trim()) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_edge_dmz_ingress_routes() {
        let dir = std::env::temp_dir().join(format!("vzctl-ui-ingress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hypernetwork.config.yaml");
        let mut file = std::fs::File::create(&path).unwrap();
        write!(
            file,
            r#"
apiVersion: hypernetwork/v1
spec:
  project: edge-dmz
  ingress:
    enabled: true
    hostAliases: true
    routes:
      - host: web.svc.edge-dmz.vz.test
        to: "web:80"
        requires: [oidc]
      - host: auth.svc.edge-dmz.vz.test
        to: "oidc:5556"
"#
        )
        .unwrap();
        let ingress = ingress_from_config(&path);
        assert_eq!(ingress["enabled"], true);
        let routes = ingress["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0]["host"], "web.svc.edge-dmz.vz.test");
        assert_eq!(routes[0]["url"], "https://web.svc.edge-dmz.vz.test");
        assert_eq!(routes[0]["alias"]["host"], "web.localhost");
        assert_eq!(routes[0]["alias"]["url"], "https://web.localhost");
        assert_eq!(routes[1]["host"], "auth.svc.edge-dmz.vz.test");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))
}

#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    if let Some(parent) = Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&path, contents).map_err(|e| format!("write {path}: {e}"))
}

#[tauri::command]
fn write_secret_file(path: String, contents: String) -> Result<(), String> {
    write_text_file(path.clone(), contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[tauri::command]
fn ensure_dir(path: String) -> Result<(), String> {
    std::fs::create_dir_all(&path).map_err(|e| format!("mkdir {path}: {e}"))
}

#[tauri::command]
fn path_exists(path: String) -> Result<bool, String> {
    Ok(Path::new(&path).exists())
}

#[tauri::command]
fn vzctl_state_dir() -> Result<String, String> {
    Ok(terminal::vzctl_state_dir_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(EventBridge {
            _child: Mutex::new(None),
        })
        .manage(terminal::TerminalState::new())
        .invoke_handler(tauri::generate_handler![
            api_base_url,
            api_request,
            run_vzctl,
            run_vzctl_argv,
            request_host_reboot,
            subscribe_events,
            open_url,
            read_text_file,
            write_text_file,
            write_secret_file,
            ensure_dir,
            path_exists,
            vzctl_state_dir,
            terminal::terminal_open_attach,
            terminal::terminal_open_exec,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running vzctl-ui");
}
