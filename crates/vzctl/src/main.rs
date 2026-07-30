use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("help") | Some("-h") | Some("--help") => {
            println!(
                "\
vzctl — Environments-as-Code for macOS Virtualization (Alpha stub)

Commands:
  doctor              Check host baseline and supervisor health
  version
  apply [--resume|--abort]   (stub — see ADR 0003)
  help"
            );
            ExitCode::SUCCESS
        }
        Some("version") => {
            println!("vzctl {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("doctor") => doctor(),
        Some("apply") => {
            let flags: Vec<String> = args.collect();
            eprintln!("apply stub: flags={flags:?} — journal/reconcile not wired yet (ADR 0003)");
            ExitCode::from(2)
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            ExitCode::from(2)
        }
    }
}

fn doctor() -> ExitCode {
    let ver = macos_major().unwrap_or(0);
    println!("vzctl doctor");
    println!("  host macOS major: {ver}");
    if ver < 26 {
        eprintln!("  FAIL: macOS 26+ required (ADR 0001)");
        return ExitCode::from(11);
    }
    println!("  OK: macOS baseline");
    println!("  OK: ADR 0002 ownership / 0003 apply specs present in docs/adr/");
    check_supervisor()
}

fn check_supervisor() -> ExitCode {
    let path = supervisor_socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            println!("  WARN: supervisor not running ({})", path.display());
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("  FAIL: supervisor socket {}: {error}", path.display());
            return ExitCode::from(10);
        }
    };

    let timeout = Some(Duration::from_secs(2));
    if let Err(error) = stream.set_read_timeout(timeout) {
        eprintln!("  FAIL: supervisor read timeout setup: {error}");
        return ExitCode::from(10);
    }
    if let Err(error) = stream.set_write_timeout(timeout) {
        eprintln!("  FAIL: supervisor write timeout setup: {error}");
        return ExitCode::from(10);
    }

    let request = json!({
        "jsonrpc": "2.0",
        "method": "daemon.health",
        "id": 1
    });
    if let Err(error) = writeln!(stream, "{request}") {
        eprintln!("  FAIL: supervisor health request: {error}");
        return ExitCode::from(10);
    }

    let mut response = String::new();
    if let Err(error) = BufReader::new(stream).read_line(&mut response) {
        eprintln!("  FAIL: supervisor health response: {error}");
        return ExitCode::from(10);
    }
    let value: Value = match serde_json::from_str(&response) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("  FAIL: invalid supervisor health JSON: {error}");
            return ExitCode::from(10);
        }
    };
    let result = &value["result"];
    if result["ok"] != true || result["db_ok"] != true {
        eprintln!("  FAIL: supervisor health is not ok: {value}");
        return ExitCode::from(10);
    }

    println!(
        "  OK: supervisor {} (pid {}, db ok)",
        result["version"].as_str().unwrap_or("unknown"),
        result["pid"]
    );
    ExitCode::SUCCESS
}

fn supervisor_socket_path() -> PathBuf {
    if let Some(directory) = std::env::var_os("VZCTL_STATE_DIR") {
        return PathBuf::from(directory).join("vz.sock");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("vzctl")
        .join("vz.sock")
}

fn macos_major() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let out = Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        s.trim().split('.').next()?.parse().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
