//! Minimal Supervisor REST client (UDS default, optional TCP).

use serde_json::Value;
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

enum UpstreamStream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for UpstreamStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(stream) => stream.read(buf),
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for UpstreamStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(stream) => stream.write(buf),
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Clone)]
pub struct ApiClient {
    upstream: Upstream,
}

impl ApiClient {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            upstream: resolve_upstream()?,
        })
    }

    pub fn get(&self, path: &str) -> Result<ApiResponse, String> {
        self.request("GET", path, None)
    }

    pub fn post(&self, path: &str, body: Option<&Value>) -> Result<ApiResponse, String> {
        let payload = body.map(|value| value.to_string());
        self.request("POST", path, payload.as_deref())
    }

    pub fn delete(&self, path: &str) -> Result<ApiResponse, String> {
        self.request("DELETE", path, None)
    }

    pub fn get_json(&self, path: &str) -> Result<Value, String> {
        let response = self.get(path)?;
        parse_json_response(&response)
    }

    pub fn post_json(&self, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let response = self.post(path, body)?;
        parse_json_response(&response)
    }

    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<ApiResponse, String> {
        let mut stream = connect_upstream(&self.upstream)?;
        let host = match &self.upstream {
            Upstream::Unix(_) => "localhost".to_string(),
            Upstream::Tcp(address) => address
                .split(':')
                .next()
                .unwrap_or("127.0.0.1")
                .to_string(),
        };

        let body_bytes = body.map(str::as_bytes).unwrap_or(b"");
        let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
        if body_bytes.is_empty() {
            if matches!(method, "POST" | "PUT" | "PATCH") {
                request.push_str("Content-Length: 0\r\n");
            }
        } else {
            request.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
            request.push_str("Content-Type: application/json\r\n");
        }
        request.push_str("\r\n");

        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("api write headers: {error}"))?;
        if !body_bytes.is_empty() {
            stream
                .write_all(body_bytes)
                .map_err(|error| format!("api write body: {error}"))?;
        }
        let _ = stream.flush();

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|error| format!("api read: {error}"))?;
        parse_http_response(&raw)
    }
}

/// URL-encode a vzctl resource id (`project/vm` → `project%2Fvm`).
pub fn encode_id(id: &str) -> String {
    id.replace('/', "%2F")
}

pub fn vzctl_state_dir() -> PathBuf {
    if let Ok(path) = std::env::var("VZCTL_STATE_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs_fallback_state_dir()
}

fn dirs_fallback_state_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    home.join("Library/Application Support/vzctl")
}

fn resolve_upstream() -> Result<Upstream, String> {
    if let Ok(raw) = std::env::var("VZCTL_API_LISTEN") {
        return parse_listen(&raw);
    }
    Ok(Upstream::Unix(vzctl_state_dir().join("api.sock")))
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
        let (host, _port) = rest
            .rsplit_once(':')
            .ok_or_else(|| "VZCTL_API_LISTEN tcp needs host:port".to_string())?;
        let host = if host == "localhost" { "127.0.0.1" } else { host };
        return Ok(Upstream::Tcp(format!("{host}:{_port}")));
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
                    Err(error) => {
                        last = Some(error.to_string());
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
            Err(format!(
                "Supervisor REST not reachable ({}): {}. Is vz-supervisor serve active?",
                path.display(),
                last.unwrap_or_else(|| "failed".into())
            ))
        }
        Upstream::Tcp(address) => {
            let stream = TcpStream::connect(address).map_err(|error| {
                format!(
                    "Supervisor REST not reachable ({address}): {error}. Is vz-supervisor serve active?"
                )
            })?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
            Ok(UpstreamStream::Tcp(stream))
        }
    }
}

fn parse_http_response(raw: &[u8]) -> Result<ApiResponse, String> {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_string())?;
    let header_bytes = &raw[..separator];
    let body_bytes = &raw[separator + 4..];
    let header_text = std::str::from_utf8(header_bytes)
        .map_err(|error| format!("invalid HTTP header utf-8: {error}"))?;
    let status_line = header_text
        .lines()
        .next()
        .ok_or_else(|| "missing HTTP status line".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid HTTP status line: {status_line}"))?;
    let body = String::from_utf8_lossy(body_bytes).into_owned();
    Ok(ApiResponse { status, body })
}

fn parse_json_response(response: &ApiResponse) -> Result<Value, String> {
    if response.status >= 400 {
        return Err(format!(
            "HTTP {}: {}",
            response.status,
            response.body.trim()
        ));
    }
    serde_json::from_str(&response.body)
        .map_err(|error| format!("invalid JSON body: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn encode_id_percent_encodes_slashes() {
        assert_eq!(encode_id("edge-dmz/web"), "edge-dmz%2Fweb");
        assert_eq!(encode_id("web"), "web");
    }

    #[test]
    fn parse_listen_accepts_unix_and_tcp() {
        assert!(matches!(
            parse_listen("unix:/tmp/api.sock").unwrap(),
            Upstream::Unix(path) if path == Path::new("/tmp/api.sock")
        ));
        assert!(matches!(
            parse_listen("tcp:127.0.0.1:8080").unwrap(),
            Upstream::Tcp(addr) if addr == "127.0.0.1:8080"
        ));
    }
}
