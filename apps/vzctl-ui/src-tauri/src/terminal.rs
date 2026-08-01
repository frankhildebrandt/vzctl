use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const MUX_STDIN: u8 = 0x01;
const MUX_STDOUT: u8 = 0x02;
const MUX_RESIZE: u8 = 0x04;
const MUX_EXIT: u8 = 0x05;
const MUX_STDIN_EOF: u8 = 0x06;

enum SessionKind {
    Attach,
    Exec,
}

struct Session {
    kind: SessionKind,
    writer: Sender<WriterMsg>,
}

enum WriterMsg {
    Bytes(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

pub struct TerminalState {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<String, Session>>,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalDataPayload {
    session_id: String,
    data: Vec<u8>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalExitPayload {
    session_id: String,
    code: Option<u8>,
    message: Option<String>,
}

#[tauri::command]
pub fn terminal_open_attach(
    app: AppHandle,
    state: State<'_, TerminalState>,
    vm_id: String,
) -> Result<String, String> {
    let path = console_socket_path(&vm_id);
    let stream = UnixStream::connect(&path)
        .map_err(|e| format!("console socket {}: {e}", path.display()))?;
    open_session(app, &state, SessionKind::Attach, stream)
}

#[tauri::command]
pub fn terminal_open_exec(
    app: AppHandle,
    state: State<'_, TerminalState>,
    vm_id: String,
    cmd: Vec<String>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    if cmd.is_empty() {
        return Err("exec cmd is empty".into());
    }
    let socket = supervisor_socket_path();
    let result = supervisor_rpc(
        &socket,
        "vm.exec_tty",
        json!({
            "vm_id": vm_id,
            "cmd": cmd,
            "cols": cols,
            "rows": rows,
        }),
    )?;
    let path = result["socket"]
        .as_str()
        .ok_or_else(|| "vm.exec_tty missing socket path".to_string())?;
    let mut stream = UnixStream::connect(path)
        .map_err(|e| format!("exec tty socket {path}: {e}"))?;
    write_mux_frame(&mut stream, MUX_RESIZE, &resize_payload(cols, rows))?;
    open_session(app, &state, SessionKind::Exec, stream)
}

#[tauri::command]
pub fn terminal_write(
    state: State<'_, TerminalState>,
    session_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "terminal sessions lock poisoned".to_string())?;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {session_id}"))?;
    session
        .writer
        .send(WriterMsg::Bytes(data))
        .map_err(|_| "terminal session writer closed".to_string())
}

#[tauri::command]
pub fn terminal_resize(
    state: State<'_, TerminalState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "terminal sessions lock poisoned".to_string())?;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {session_id}"))?;
    if !matches!(session.kind, SessionKind::Exec) {
        return Ok(());
    }
    session
        .writer
        .send(WriterMsg::Resize { cols, rows })
        .map_err(|_| "terminal session writer closed".to_string())
}

#[tauri::command]
pub fn terminal_close(
    state: State<'_, TerminalState>,
    session_id: String,
) -> Result<(), String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "terminal sessions lock poisoned".to_string())?;
    if let Some(session) = sessions.remove(&session_id) {
        let _ = session.writer.send(WriterMsg::Close);
    }
    Ok(())
}

fn open_session(
    app: AppHandle,
    state: &TerminalState,
    kind: SessionKind,
    stream: UnixStream,
) -> Result<String, String> {
    let session_id = format!("t{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = mpsc::channel::<WriterMsg>();
    let read_stream = stream
        .try_clone()
        .map_err(|e| format!("clone terminal stream: {e}"))?;
    let write_stream = stream;

    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "terminal sessions lock poisoned".to_string())?;
        sessions.insert(
            session_id.clone(),
            Session {
                kind: match kind {
                    SessionKind::Attach => SessionKind::Attach,
                    SessionKind::Exec => SessionKind::Exec,
                },
                writer: tx,
            },
        );
    }

    let write_kind = match kind {
        SessionKind::Attach => SessionKind::Attach,
        SessionKind::Exec => SessionKind::Exec,
    };
    thread::spawn(move || {
        let mut stream = write_stream;
        while let Ok(msg) = rx.recv() {
            match msg {
                WriterMsg::Bytes(data) => {
                    let result = match write_kind {
                        SessionKind::Attach => stream
                            .write_all(&data)
                            .map_err(|e| format!("attach write: {e}")),
                        SessionKind::Exec => write_mux_frame(&mut stream, MUX_STDIN, &data),
                    };
                    if result.is_err() {
                        break;
                    }
                }
                WriterMsg::Resize { cols, rows } => {
                    if matches!(write_kind, SessionKind::Exec)
                        && write_mux_frame(&mut stream, MUX_RESIZE, &resize_payload(cols, rows))
                            .is_err()
                    {
                        break;
                    }
                }
                WriterMsg::Close => {
                    if matches!(write_kind, SessionKind::Exec) {
                        let _ = write_mux_frame(&mut stream, MUX_STDIN_EOF, &[]);
                    }
                    let _ = stream.shutdown(Shutdown::Both);
                    break;
                }
            }
        }
    });

    let read_app = app.clone();
    let read_session = session_id.clone();
    let read_kind = match kind {
        SessionKind::Attach => SessionKind::Attach,
        SessionKind::Exec => SessionKind::Exec,
    };
    thread::spawn(move || {
        let mut stream = read_stream;
        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
        match read_kind {
            SessionKind::Attach => {
                let mut buf = [0_u8; 4096];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let _ = read_app.emit(
                                "terminal-data",
                                TerminalDataPayload {
                                    session_id: read_session.clone(),
                                    data: buf[..n].to_vec(),
                                },
                            );
                        }
                        Err(err)
                            if err.kind() == std::io::ErrorKind::WouldBlock
                                || err.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    }
                }
                let _ = read_app.emit(
                    "terminal-exit",
                    TerminalExitPayload {
                        session_id: read_session,
                        code: None,
                        message: Some("detached".into()),
                    },
                );
            }
            SessionKind::Exec => {
                let mut exit_code = None;
                loop {
                    match read_mux_frame(&mut stream) {
                        Ok(Some((MUX_STDOUT, payload))) => {
                            let _ = read_app.emit(
                                "terminal-data",
                                TerminalDataPayload {
                                    session_id: read_session.clone(),
                                    data: payload,
                                },
                            );
                        }
                        Ok(Some((MUX_EXIT, payload))) => {
                            exit_code = payload.first().copied();
                            break;
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                let _ = read_app.emit(
                    "terminal-exit",
                    TerminalExitPayload {
                        session_id: read_session,
                        code: exit_code,
                        message: None,
                    },
                );
            }
        }
    });

    Ok(session_id)
}

fn resize_payload(cols: u16, rows: u16) -> [u8; 4] {
    let mut payload = [0_u8; 4];
    payload[0..2].copy_from_slice(&cols.to_le_bytes());
    payload[2..4].copy_from_slice(&rows.to_le_bytes());
    payload
}

fn write_mux_frame(stream: &mut UnixStream, frame_type: u8, payload: &[u8]) -> Result<(), String> {
    if payload.len() > 1_048_576 {
        return Err("mux frame exceeds 1 MiB".into());
    }
    let mut header = [0_u8; 5];
    header[0] = frame_type;
    header[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    stream
        .write_all(&header)
        .and_then(|_| {
            if payload.is_empty() {
                Ok(())
            } else {
                stream.write_all(payload)
            }
        })
        .map_err(|e| format!("mux write: {e}"))
}

fn read_mux_frame(stream: &mut UnixStream) -> Result<Option<(u8, Vec<u8>)>, String> {
    let mut header = [0_u8; 5];
    match read_exact_timeout(stream, &mut header) {
        Ok(()) => {}
        Err(err) if err == "eof" => return Ok(None),
        Err(err) => return Err(err),
    }
    let frame_type = header[0];
    let len = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
    if len > 1_048_576 {
        return Err("mux frame exceeds 1 MiB".into());
    }
    let mut payload = vec![0_u8; len];
    if len > 0 {
        read_exact_timeout(stream, &mut payload)?;
    }
    Ok(Some((frame_type, payload)))
}

fn read_exact_timeout(stream: &mut UnixStream, buf: &mut [u8]) -> Result<(), String> {
    let mut offset = 0;
    while offset < buf.len() {
        match stream.read(&mut buf[offset..]) {
            Ok(0) => return Err("eof".into()),
            Ok(n) => offset += n,
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(())
}

fn supervisor_rpc(socket_path: &Path, method: &str, params: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| format!("supervisor socket {}: {e}", socket_path.display()))?;
    let timeout = Some(Duration::from_secs(5));
    stream
        .set_read_timeout(timeout)
        .and_then(|_| stream.set_write_timeout(timeout))
        .map_err(|e| format!("supervisor timeout setup: {e}"))?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    writeln!(stream, "{request}").map_err(|e| format!("supervisor request: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("supervisor response: {e}"))?;
    let response: Value = serde_json::from_str(&line)
        .map_err(|e| format!("invalid supervisor response: {e}"))?;
    if let Some(error) = response.get("error") {
        return Err(error["message"]
            .as_str()
            .unwrap_or("VM operation failed")
            .to_string());
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| "supervisor response missing result".to_string())
}

fn state_dir() -> PathBuf {
    if let Some(directory) = std::env::var_os("VZCTL_STATE_DIR") {
        return PathBuf::from(directory);
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("vzctl")
}

pub(crate) fn vzctl_state_dir_string() -> String {
    state_dir().display().to_string()
}

fn supervisor_socket_path() -> PathBuf {
    state_dir().join("vz.sock")
}

fn console_socket_path(id: &str) -> PathBuf {
    state_dir()
        .join("helpers")
        .join(format!("{}.console.sock", state_file_component(id)))
}

fn state_file_component(value: &str) -> String {
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
