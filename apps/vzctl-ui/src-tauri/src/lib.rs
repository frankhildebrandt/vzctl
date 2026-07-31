use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

#[tauri::command]
fn open_environment(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let folder = app
        .dialog()
        .file()
        .set_title("Open vzctl Environment")
        .blocking_pick_folder();
    Ok(folder.map(|p| p.to_string()))
}

#[tauri::command]
fn run_vzctl(path: String, command: String) -> Result<String, String> {
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
    let args = match command.as_str() {
        "diff" => vec!["diff", "-C", path.as_str(), "--format", "json"],
        "up" => vec!["up", "-C", path.as_str(), "--format", "json"],
        "apply" => vec!["apply", "-C", path.as_str(), "--format", "json"],
        "down" => vec!["down", "-C", path.as_str(), "--format", "json"],
        "status" => {
            return status_bundle(&vzctl, &path);
        }
        other => return Err(format!("unsupported command: {other}")),
    };

    let output = Command::new(&vzctl)
        .args(&args)
        .output()
        .map_err(|e| format!("spawn vzctl: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        Ok(pretty_or_raw(&stdout))
    } else {
        Err(format!(
            "vzctl {} failed ({})\n{}\n{}",
            command,
            output.status,
            stdout.trim(),
            stderr.trim()
        ))
    }
}

fn status_bundle(vzctl: &PathBuf, path: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for (label, args) in [
        ("dns", vec!["dns", "status", "--format", "json"]),
        ("certs", vec!["certs", "fingerprint", "--format", "json"]),
        (
            "oidc",
            vec![
                "oidc",
                "status",
                "--format",
                "json",
            ],
        ),
        ("diff", vec!["diff", "-C", path, "--format", "json"]),
    ] {
        let output = Command::new(vzctl)
            .args(&args)
            .output()
            .map_err(|e| format!("spawn vzctl: {e}"))?;
        let body = if output.status.success() {
            pretty_or_raw(&String::from_utf8_lossy(&output.stdout))
        } else {
            format!(
                "ERROR {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
        };
        parts.push(format!("## {label}\n{body}"));
    }
    Ok(parts.join("\n\n"))
}

fn which_vzctl() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("VZCTL_BIN") {
        return Ok(PathBuf::from(path));
    }
    which("vzctl").or_else(|_| {
        let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug/vzctl");
        if debug.exists() {
            Ok(debug)
        } else {
            Err("vzctl not found on PATH; set VZCTL_BIN".into())
        }
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![open_environment, run_vzctl])
        .run(tauri::generate_context!())
        .expect("error while running vzctl-ui");
}
