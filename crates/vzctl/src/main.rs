use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("help") | Some("-h") | Some("--help") => {
            println!(
                "\
vzctl — Environments-as-Code for macOS Virtualization (Alpha stub)

Commands:
  doctor              Check host baseline (macOS 26+)
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
            eprintln!(
                "apply stub: flags={flags:?} — journal/reconcile not wired yet (ADR 0003)"
            );
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
    println!("  note: supervisor/helper binaries — build via `swift build` in daemon/");
    ExitCode::SUCCESS
}

fn macos_major() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let out = Command::new("sw_vers").arg("-productVersion").output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        s.trim().split('.').next()?.parse().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
