//! Shared controller contract. Runtime bindings keep credentials separate from status.
mod config;
mod validation;
pub(crate) use validation::validate_config;
mod bindings;
mod edge;
pub(crate) mod kubernetes_claims;
mod postgres;
mod runtime;
mod s3;
pub(crate) mod storage;
pub(crate) use config::*;
pub(crate) use runtime::{reconcile, run};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) struct ControllerError {
    pub code: &'static str,
    pub message: String,
}
impl ControllerError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Debug for ControllerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::fmt::Display for ControllerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ControllerError {}
pub(crate) type ControllerResult<T> = Result<T, ControllerError>;

pub(crate) struct ExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}
pub(crate) trait GuestExec {
    fn check_permission(&self) -> ControllerResult<()> {
        Ok(())
    }
    fn run(
        &self,
        vm: &str,
        argv: &[String],
        env: &BTreeMap<String, String>,
        stdin: &[u8],
        timeout_ms: u64,
    ) -> ControllerResult<ExecOutput>;
}
pub(crate) trait SecretStore {
    fn get_or_create(&self, key: &str) -> ControllerResult<String>;
}
pub(crate) struct Binding {
    pub public: PublicBinding,
    /// Internal only: never serialize this map in status, events, or logs.
    pub secrets: BTreeMap<String, String>,
}
pub(crate) struct ReconcileResult {
    pub changed: bool,
    pub binding: Binding,
    pub resource: serde_json::Value,
}
pub(crate) struct ControllerContext<'a> {
    pub project: &'a str,
    pub claim_name: &'a str,
    pub provider: &'a ProviderConfig,
    pub advertised_endpoint: &'a str,
    pub claim: &'a ClaimConfig,
    pub config_dir: &'a Path,
    pub guest: &'a dyn GuestExec,
    pub secrets: &'a dyn SecretStore,
}
impl ControllerContext<'_> {
    pub fn owner(&self) -> String {
        format!("vzctl:{}/{}", self.project, self.claim_name)
    }
    pub fn provider_vm(&self) -> ControllerResult<String> {
        self.provider
            .via
            .as_ref()
            .map(|vm| format!("{}/{}", self.project, vm))
            .ok_or_else(|| ControllerError::new("InvalidProvider", "provider requires via VM"))
    }
    pub fn read_secret(&self, path: &str) -> ControllerResult<String> {
        let file = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.config_dir.join(path)
        };
        use std::io::Read;
        let mut value = String::new();
        use std::os::unix::fs::OpenOptionsExt;
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(file)
            .map_err(|_| {
                ControllerError::new(
                    "SecretUnavailable",
                    "provider credential file cannot be read",
                )
            })?;
        if !handle
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() <= 65536)
        {
            return Err(ControllerError::new(
                "InvalidSecret",
                "provider credential must be a bounded regular file",
            ));
        }
        handle.take(65537).read_to_string(&mut value).map_err(|_| {
            ControllerError::new(
                "SecretUnavailable",
                "provider credential file cannot be read",
            )
        })?;
        if value.len() > 65536 || value.trim().is_empty() {
            return Err(ControllerError::new(
                "InvalidSecret",
                "provider credential must be nonempty and bounded",
            ));
        }
        Ok(value.trim_end_matches(['\n', '\r']).to_string())
    }
}
pub(crate) trait ServiceController {
    fn reconcile(&self, context: &ControllerContext<'_>) -> ControllerResult<ReconcileResult>;
}
