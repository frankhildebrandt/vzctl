//! Supervisor REST client (UDS default or TCP) for Tauri invoke.
//! WebView `fetch` to localhost fails under WKWebView ("Load failed");
//! all UI API traffic goes through `request` / `open_sse` instead.

use serde::Serialize;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

#[derive(Clone)]
enum Upstream {
    Unix(PathBuf),
    Tcp(String),
}

pub(crate) enum UpstreamStream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for UpstreamStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for UpstreamStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

// Safety: each stream is used on a single thread after handoff.
unsafe impl Send for UpstreamStream {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse {
    pub status: u16,
    pub body: String,
    pub content_type: Option<String>,
}

pub fn request(
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<ApiResponse, String> {
    let upstream = resolve_upstream()?;
    let mut stream = connect_upstream(&upstream)?;

    let host = match &upstream {
        Upstream::Unix(_) => "localhost".to_string(),
        Upstream::Tcp(addr) => addr.split(':').next().unwrap_or("127.0.0.1").to_string(),
    };

    let body_bytes = body.map(str::as_bytes).unwrap_or(b"");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    let mut has_content_type = false;
    let mut has_content_length = false;
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == "host" || lower == "connection" {
            continue;
        }
        if lower == "content-type" {
            has_content_type = true;
        }
        if lower == "content-length" {
            has_content_length = true;
        }
        req.push_str(name);
        req.push_str(": ");
        req.push_str(value);
        req.push_str("\r\n");
    }
    if !body_bytes.is_empty() {
        if !has_content_length {
            req.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
        }
        if !has_content_type {
            req.push_str("Content-Type: application/json\r\n");
        }
    } else if matches!(method, "POST" | "PUT" | "PATCH") && !has_content_length {
        req.push_str("Content-Length: 0\r\n");
    }
    req.push_str("\r\n");

    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("api write headers: {e}"))?;
    if !body_bytes.is_empty() {
        stream
            .write_all(body_bytes)
            .map_err(|e| format!("api write body: {e}"))?;
    }
    let _ = stream.flush();

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("api read: {e}"))?;
    parse_http_response(&raw)
}

/// Open SSE stream after HTTP headers (body is event-stream).
pub fn open_sse(path_and_query: &str) -> Result<UpstreamStream, String> {
    let upstream = resolve_upstream()?;
    let mut stream = connect_upstream(&upstream)?;
    let host = match &upstream {
        Upstream::Unix(_) => "localhost",
        Upstream::Tcp(addr) => addr.split(':').next().unwrap_or("127.0.0.1"),
    };
    let req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {host}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("sse write: {e}"))?;
    let _ = stream.flush();

    let mut buf = Vec::new();
    let mut tmp = [0u8; 1];
    loop {
        let n = stream
            .read(&mut tmp)
            .map_err(|e| format!("sse header read: {e}"))?;
        if n == 0 {
            return Err("sse closed during headers".into());
        }
        buf.push(tmp[0]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64_000 {
            return Err("sse headers too large".into());
        }
    }
    Ok(stream)
}

fn resolve_upstream() -> Result<Upstream, String> {
    if let Ok(raw) = std::env::var("VZCTL_API_LISTEN") {
        return parse_listen(&raw);
    }
    let state = crate::terminal::vzctl_state_dir_path();
    Ok(Upstream::Unix(state.join("api.sock")))
}

fn parse_listen(raw: &str) -> Result<Upstream, String> {
    let trimmed = raw.trim();
    if let Some(path) = trimmed.strip_prefix("unix:") {
        if path.is_empty() {
            return Err("VZCTL_API_LISTEN unix path empty".into());
        }
        return Ok(Upstream::Unix(PathBuf::from(path)));
    }
    if let Some(rest) = trimmed.strip_prefix("tcp:") {
        let (host, port) = rest
            .rsplit_once(':')
            .ok_or_else(|| "VZCTL_API_LISTEN tcp needs host:port".to_string())?;
        let host = if host == "localhost" {
            "127.0.0.1"
        } else {
            host
        };
        return Ok(Upstream::Tcp(format!("{host}:{port}")));
    }
    Err(format!(
        "VZCTL_API_LISTEN must be unix:<path> or tcp:host:port, got {trimmed}"
    ))
}

fn connect_upstream(upstream: &Upstream) -> Result<UpstreamStream, String> {
    match upstream {
        Upstream::Unix(path) => {
            let mut last = None;
            for _ in 0..20 {
                match UnixStream::connect(path) {
                    Ok(stream) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
                        return Ok(UpstreamStream::Unix(stream));
                    }
                    Err(err) => {
                        last = Some(err.to_string());
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
            Err(format!(
                "Supervisor REST nicht erreichbar ({}): {}. Ist `vz-supervisor serve` aktiv?",
                path.display(),
                last.unwrap_or_else(|| "failed".into())
            ))
        }
        Upstream::Tcp(addr) => {
            let stream = TcpStream::connect(addr).map_err(|e| {
                format!(
                    "Supervisor REST nicht erreichbar ({addr}): {e}. Ist `vz-supervisor serve` aktiv?"
                )
            })?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
            Ok(UpstreamStream::Tcp(stream))
        }
    }
}

fn parse_http_response(raw: &[u8]) -> Result<ApiResponse, String> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "api response missing header terminator".to_string())?;
    let header_bytes = &raw[..sep];
    let body_bytes = &raw[sep + 4..];
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "api response missing status line".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("api bad status line: {status_line}"))?;

    let mut content_type = None;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-type" {
            content_type = Some(value.to_string());
        } else if name == "content-length" {
            content_length = value.parse().ok();
        }
    }

    let body = if let Some(len) = content_length {
        String::from_utf8_lossy(&body_bytes[..len.min(body_bytes.len())]).into_owned()
    } else {
        String::from_utf8_lossy(body_bytes).into_owned()
    };

    Ok(ApiResponse {
        status,
        body,
        content_type,
    })
}
