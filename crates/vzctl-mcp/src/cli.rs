//! Subprocess wrapper around the vzctl CLI for operations without REST coverage.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub struct VzctlCli {
    binary: PathBuf,
}

impl VzctlCli {
    pub fn from_env() -> Self {
        let binary = std::env::var_os("VZCTL_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("vzctl"));
        Self { binary }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Run `vzctl` with args; stdout must be a JSON envelope on success.
    pub fn run_json(&self, args: &[&str]) -> Result<Value, String> {
        let output = Command::new(&self.binary)
            .args(args)
            .output()
            .map_err(|error| format!("failed to spawn {}: {error}", self.binary.display()))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            return Err(format!(
                "vzctl exited {}: {}{}",
                output.status,
                stderr.trim(),
                if stdout.trim().is_empty() {
                    String::new()
                } else {
                    format!("\nstdout: {}", stdout.trim())
                }
            ));
        }
        serde_json::from_str(stdout.trim())
            .map_err(|error| format!("invalid vzctl JSON stdout: {error}"))
    }

    /// One-shot guest command via `vzctl vm exec --format json`.
    pub fn vm_exec(
        &self,
        vm_id: &str,
        command: &[String],
        timeout_ms: u64,
        cwd: Option<&str>,
        env: &[(&str, &str)],
    ) -> Result<Value, String> {
        let mut args: Vec<String> = vec![
            "vm".into(),
            "exec".into(),
            vm_id.to_string(),
            "--format".into(),
            "json".into(),
            "--timeout-ms".into(),
            timeout_ms.to_string(),
        ];
        if let Some(path) = cwd {
            args.push("--cwd".into());
            args.push(path.to_string());
        }
        for (key, value) in env {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        args.push("--".into());
        args.extend(command.iter().cloned());
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run_json(&argv)
    }

    /// Tail serial console log via `vzctl vm logs --format json`.
    pub fn vm_logs(&self, vm_id: &str, tail: Option<u32>) -> Result<Value, String> {
        let tail = tail.unwrap_or(200).to_string();
        self.run_json(&[
            "vm",
            "logs",
            vm_id,
            "--format",
            "json",
            "--tail",
            &tail,
        ])
    }
}
