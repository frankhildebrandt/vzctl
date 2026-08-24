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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DebugStackPathParams {
    stack_path: String,
    #[serde(default)]
    source_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DebugSourceParams {
    stack_path: String,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DebugSqlQueryParams {
    stack_path: String,
    sql: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    max_rows: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DebugNatsSubParams {
    stack_path: String,
    subject: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DebugNatsKvGetParams {
    stack_path: String,
    bucket: String,
    key: String,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DebugNatsKvWatchParams {
    stack_path: String,
    bucket: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
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

    // --- Debug data sources ---

    #[tool(description = "List observability.dataSources for a stack directory.")]
    fn debug_sources_list(
        &self,
        Parameters(DebugStackPathParams {
            stack_path,
            source_type,
        }): Parameters<DebugStackPathParams>,
    ) -> Result<String, String> {
        let mut args = vec![
            "debug".to_string(),
            "sources".to_string(),
            "list".to_string(),
            "-C".to_string(),
            stack_path,
            "--format".to_string(),
            "json".to_string(),
        ];
        if let Some(kind) = source_type {
            args.push("--type".into());
            args.push(kind);
        }
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(json_pretty(&self.cli.run_json(&argv)?))
    }

    #[tool(description = "Check debug via-VM prerequisites (psql/mysql/nats in PATH).")]
    fn debug_doctor(
        &self,
        Parameters(DebugStackPathParams { stack_path, .. }): Parameters<DebugStackPathParams>,
    ) -> Result<String, String> {
        let args = [
            "debug",
            "doctor",
            "-C",
            stack_path.as_str(),
            "--format",
            "json",
        ];
        Ok(json_pretty(&self.cli.run_json(&args)?))
    }

    #[tool(description = "Read-only Postgres query via configured via-VM.")]
    fn debug_postgres_query(
        &self,
        Parameters(DebugSqlQueryParams {
            stack_path,
            sql,
            source,
            max_rows,
            timeout_ms,
        }): Parameters<DebugSqlQueryParams>,
    ) -> Result<String, String> {
        extensions::run_sql_query(
            &self.cli,
            "postgres",
            &stack_path,
            &sql,
            &source,
            max_rows,
            timeout_ms,
        )
    }

    #[tool(description = "Read-only MySQL query via configured via-VM.")]
    fn debug_mysql_query(
        &self,
        Parameters(DebugSqlQueryParams {
            stack_path,
            sql,
            source,
            max_rows,
            timeout_ms,
        }): Parameters<DebugSqlQueryParams>,
    ) -> Result<String, String> {
        extensions::run_sql_query(
            &self.cli,
            "mysql",
            &stack_path,
            &sql,
            &source,
            max_rows,
            timeout_ms,
        )
    }

    #[tool(description = "List NATS streams (read-only) via configured via-VM.")]
    fn debug_nats_streams(
        &self,
        Parameters(DebugSourceParams { stack_path, source }): Parameters<DebugSourceParams>,
    ) -> Result<String, String> {
        let mut args = vec![
            "debug".into(),
            "nats".into(),
            "streams".into(),
            "-C".into(),
            stack_path,
            "--format".into(),
            "json".into(),
        ];
        extensions::push_source(&mut args, &source);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(json_pretty(&self.cli.run_json(&argv)?))
    }

    #[tool(description = "Batch-subscribe to a NATS subject (read-only).")]
    fn debug_nats_subscribe(
        &self,
        Parameters(DebugNatsSubParams {
            stack_path,
            subject,
            source,
            count,
            timeout_ms,
        }): Parameters<DebugNatsSubParams>,
    ) -> Result<String, String> {
        let mut args = vec![
            "debug".into(),
            "nats".into(),
            "sub".into(),
            "-C".into(),
            stack_path,
            "--format".into(),
            "json".into(),
            "--subject".into(),
            subject,
        ];
        extensions::push_source(&mut args, &source);
        if let Some(value) = count {
            args.push("--count".into());
            args.push(value.to_string());
        }
        extensions::push_timeout_ms(&mut args, &timeout_ms);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(json_pretty(&self.cli.run_json(&argv)?))
    }

    #[tool(description = "Read a NATS KV entry (read-only).")]
    fn debug_nats_kv_get(
        &self,
        Parameters(DebugNatsKvGetParams {
            stack_path,
            bucket,
            key,
            source,
        }): Parameters<DebugNatsKvGetParams>,
    ) -> Result<String, String> {
        let mut args = vec![
            "debug".into(),
            "nats".into(),
            "kv".into(),
            "get".into(),
            "-C".into(),
            stack_path,
            "--format".into(),
            "json".into(),
            "--bucket".into(),
            bucket,
            "--key".into(),
            key,
        ];
        extensions::push_source(&mut args, &source);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(json_pretty(&self.cli.run_json(&argv)?))
    }

    #[tool(description = "Watch NATS KV changes until timeout (read-only).")]
    fn debug_nats_kv_watch(
        &self,
        Parameters(DebugNatsKvWatchParams {
            stack_path,
            bucket,
            key,
            source,
            timeout_ms,
        }): Parameters<DebugNatsKvWatchParams>,
    ) -> Result<String, String> {
        let mut args = vec![
            "debug".into(),
            "nats".into(),
            "kv".into(),
            "watch".into(),
            "-C".into(),
            stack_path,
            "--format".into(),
            "json".into(),
            "--bucket".into(),
            bucket,
        ];
        if let Some(key) = key {
            args.push("--key".into());
            args.push(key);
        }
        extensions::push_source(&mut args, &source);
        extensions::push_timeout_ms(&mut args, &timeout_ms);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(json_pretty(&self.cli.run_json(&argv)?))
    }
}

#[tool_handler(
    name = "vzctl",
    version = "0.0.1",
    instructions = "Control vzctl hypercontainers on macOS: VMs, stacks, guest exec, systemd, guest HTTP services, Docker, and debug/doctor. VM ids use project/vm form. Long stack apply jobs return jobId — poll job_status. Extend with viewer modules (NATS, Postgres) as new tools land."
)]
impl ServerHandler for VzctlMcp {}
