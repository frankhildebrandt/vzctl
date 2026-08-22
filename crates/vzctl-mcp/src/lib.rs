mod api;
mod cli;
mod extensions;
mod util;

pub use api::{encode_id, ApiClient};
pub use cli::VzctlCli;

use rmcp::{
    handler::server::wrapper::Parameters,
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::util::{api_json, api_post_json, json_pretty};

/// MCP server exposing vzctl control, debug, guest exec, and guest-service access.
#[derive(Clone)]
pub struct VzctlMcp {
    api: ApiClient,
    cli: VzctlCli,
}

impl VzctlMcp {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            api: ApiClient::from_env()?,
            cli: VzctlCli::from_env(),
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct VmIdParams {
    /// Runtime VM id (`project/vm` or flat id).
    vm_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StackIdParams {
    /// Stack registry id (usually directory basename).
    stack_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JobIdParams {
  job_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct VmExecParams {
    vm_id: String,
    /// argv passed to the guest process (no shell).
    command: Vec<String>,
    #[serde(default = "default_exec_timeout")]
    timeout_ms: u64,
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<String>,
}

fn default_exec_timeout() -> u64 {
    30_000
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct VmLogsParams {
    vm_id: String,
    #[serde(default = "default_log_tail")]
    tail: u32,
}

fn default_log_tail() -> u32 {
    200
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GuestServiceRequestParams {
    vm_id: String,
    /// Guest publisher name from `guest_services_list`.
    service: String,
    /// Root-relative API path, e.g. `/api/status`.
    path: String,
    #[serde(default)]
    method: String,
    #[serde(default)]
    body: Option<Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SystemdUnitParams {
    vm_id: String,
    unit: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SystemdListParams {
    vm_id: String,
    #[serde(default)]
    unit_type: Option<String>,
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProjectParams {
    project: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StackApplyParams {
    stack_id: String,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    resume: bool,
    #[serde(default)]
    abort: bool,
}

fn parse_env_pairs(pairs: &[String]) -> Result<Vec<(&str, &str)>, String> {
    let mut out = Vec::new();
    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .filter(|(key, _)| !key.is_empty())
            .ok_or_else(|| format!("env entry must be KEY=VALUE, got {pair:?}"))?;
        out.push((key, value));
    }
    Ok(out)
}

#[tool_router]
impl VzctlMcp {
    // --- Debug / health ---

    #[tool(description = "Run vzctl doctor health report (host + stack checks).")]
    fn doctor(&self) -> Result<String, String> {
        api_json(&self.api, "/v1/doctor")
    }

    #[tool(description = "Supervisor health including network resilience state.")]
    fn health(&self) -> Result<String, String> {
        api_json(&self.api, "/v1/health")
    }

    #[tool(description = "DNS subsystem status (vz-edge listener, bind helper).")]
    fn dns_status(&self) -> Result<String, String> {
        api_json(&self.api, "/v1/dns/status")
    }

    #[tool(description = "List active host port forwards (127.0.0.1).")]
    fn port_list(&self) -> Result<String, String> {
        api_json(&self.api, "/v1/ports")
    }

    // --- VM control ---

    #[tool(description = "List runtime VMs with state, IPs, and attachments.")]
    fn vm_list(&self) -> Result<String, String> {
        api_json(&self.api, "/v1/vms")
    }

    #[tool(description = "Inspect a VM (config, runtime, agent, serial log meta).")]
    fn vm_inspect(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}", encode_id(&vm_id));
        api_json(&self.api, &path)
    }

    #[tool(description = "Start a stopped VM.")]
    fn vm_start(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}/start", encode_id(&vm_id));
        api_post_json(&self.api, &path, None)
    }

    #[tool(description = "Stop a running VM.")]
    fn vm_stop(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}/stop", encode_id(&vm_id));
        api_post_json(&self.api, &path, None)
    }

    #[tool(description = "Restart a VM (graceful stop + start).")]
    fn vm_restart(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}/restart", encode_id(&vm_id));
        api_post_json(&self.api, &path, None)
    }

    #[tool(description = "Guest-agent CPU/RAM/IOPS stats for a VM.")]
    fn vm_stats(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}/stats", encode_id(&vm_id));
        api_json(&self.api, &path)
    }

    // --- Stacks ---

    #[tool(description = "List registered hypernetwork stacks.")]
    fn stack_list(&self) -> Result<String, String> {
        api_json(&self.api, "/v1/stacks")
    }

    #[tool(description = "Stack status bundle (desired vs actual, VMs, jobs).")]
    fn stack_status(
        &self,
        Parameters(StackIdParams { stack_id }): Parameters<StackIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/stacks/{}/status", encode_id(&stack_id));
        api_json(&self.api, &path)
    }

    #[tool(description = "Validate stack hypernetwork.config.yaml.")]
    fn stack_validate(
        &self,
        Parameters(StackIdParams { stack_id }): Parameters<StackIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/stacks/{}/validate", encode_id(&stack_id));
        api_post_json(&self.api, &path, None)
    }

    #[tool(description = "Apply stack desired state. Returns job id — poll with job_status.")]
    fn stack_apply(
        &self,
        Parameters(StackApplyParams {
            stack_id,
            force,
            resume,
            abort,
        }): Parameters<StackApplyParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/stacks/{}/apply", encode_id(&stack_id));
        let body = json!({
            "force": force,
            "resume": resume,
            "abort": abort,
        });
        api_post_json(&self.api, &path, Some(&body))
    }

    #[tool(description = "Poll long-running supervisor job status and log lines.")]
    fn job_status(
        &self,
        Parameters(JobIdParams { job_id }): Parameters<JobIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/jobs/{}", encode_id(&job_id));
        api_json(&self.api, &path)
    }

    // --- Guest shell / logs ---

    #[tool(
        description = "Run a one-shot command in a VM via guest agent (argv array, no shell)."
    )]
    fn vm_exec(
        &self,
        Parameters(VmExecParams {
            vm_id,
            command,
            timeout_ms,
            cwd,
            env,
        }): Parameters<VmExecParams>,
    ) -> Result<String, String> {
        if command.is_empty() {
            return Err("command must not be empty".into());
        }
        let env_pairs = parse_env_pairs(&env)?;
        let result = self.cli.vm_exec(
            &vm_id,
            &command,
            timeout_ms,
            cwd.as_deref(),
            &env_pairs,
        )?;
        Ok(json_pretty(&result))
    }

    #[tool(description = "Tail VM serial console log.")]
    fn vm_logs(
        &self,
        Parameters(VmLogsParams { vm_id, tail }): Parameters<VmLogsParams>,
    ) -> Result<String, String> {
        let result = self.cli.vm_logs(&vm_id, Some(tail))?;
        Ok(json_pretty(&result))
    }

    // --- Guest services / systemd ---

    #[tool(description = "List guest HTTP publishers exposed by the agent (iwatch, etc.).")]
    fn guest_services_list(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}/guest-services", encode_id(&vm_id));
        api_json(&self.api, &path)
    }

    #[tool(
        description = "HTTP request to a guest publisher API (proxied via supervisor REST)."
    )]
    fn guest_service_request(
        &self,
        Parameters(GuestServiceRequestParams {
            vm_id,
            service,
            path,
            method,
            body,
        }): Parameters<GuestServiceRequestParams>,
    ) -> Result<String, String> {
        if !path.starts_with('/') {
            return Err("path must be root-relative, e.g. /api/status".into());
        }
        let method = if method.is_empty() {
            "GET".to_string()
        } else {
            method.to_ascii_uppercase()
        };
        let api_path = format!(
            "/v1/vms/{}/guest-services/{}{}",
            encode_id(&vm_id),
            encode_id(&service),
            path
        );
        let response = match method.as_str() {
            "GET" => self.api.get(&api_path)?,
            "POST" => self.api.post(&api_path, body.as_ref())?,
            "DELETE" => self.api.delete(&api_path)?,
            other => return Err(format!("unsupported method {other}; use GET, POST, or DELETE")),
        };
        Ok(format!(
            "HTTP {}\n{}",
            response.status,
            response.body.trim()
        ))
    }

    #[tool(description = "Guest systemd capability and availability.")]
    fn systemd_status(
        &self,
        Parameters(VmIdParams { vm_id }): Parameters<VmIdParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/vms/{}/systemd", encode_id(&vm_id));
        api_json(&self.api, &path)
    }

    #[tool(description = "List systemd units in a VM.")]
    fn systemd_list_units(
        &self,
        Parameters(SystemdListParams {
            vm_id,
            unit_type,
            all,
        }): Parameters<SystemdListParams>,
    ) -> Result<String, String> {
        let mut path = format!("/v1/vms/{}/systemd/units", encode_id(&vm_id));
        let mut query = Vec::new();
        if let Some(value) = unit_type {
            query.push(format!("type={}", encode_id(&value)));
        }
        if let Some(value) = all {
            query.push(format!("all={value}"));
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
        api_json(&self.api, &path)
    }

    #[tool(description = "Start a systemd unit in a VM.")]
    fn systemd_start_unit(
        &self,
        Parameters(SystemdUnitParams { vm_id, unit }): Parameters<SystemdUnitParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/v1/vms/{}/systemd/units/{}/start",
            encode_id(&vm_id),
            encode_id(&unit)
        );
        api_post_json(&self.api, &path, None)
    }

    #[tool(description = "Stop a systemd unit in a VM.")]
    fn systemd_stop_unit(
        &self,
        Parameters(SystemdUnitParams { vm_id, unit }): Parameters<SystemdUnitParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/v1/vms/{}/systemd/units/{}/stop",
            encode_id(&vm_id),
            encode_id(&unit)
        );
        api_post_json(&self.api, &path, None)
    }

    #[tool(description = "Restart a systemd unit in a VM.")]
    fn systemd_restart_unit(
        &self,
        Parameters(SystemdUnitParams { vm_id, unit }): Parameters<SystemdUnitParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/v1/vms/{}/systemd/units/{}/restart",
            encode_id(&vm_id),
            encode_id(&unit)
        );
        api_post_json(&self.api, &path, None)
    }

    // --- Docker ---

    #[tool(description = "List Docker containers on the project's docker-role VM.")]
    fn docker_ps(
        &self,
        Parameters(ProjectParams { project }): Parameters<ProjectParams>,
    ) -> Result<String, String> {
        let path = format!("/v1/projects/{}/containers", encode_id(&project));
        api_json(&self.api, &path)
    }
}

#[tool_handler(
    name = "vzctl",
    version = "0.0.1",
    instructions = "Control vzctl hypercontainers on macOS: VMs, stacks, guest exec, systemd, guest HTTP services, Docker, and debug/doctor. VM ids use project/vm form. Long stack apply jobs return jobId — poll job_status. Extend with viewer modules (NATS, Postgres) as new tools land."
)]
impl ServerHandler for VzctlMcp {}
