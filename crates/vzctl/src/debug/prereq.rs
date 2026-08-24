use crate::config::DataSourcesConfig;
use crate::debug::sources::{configured_types, LoadedStack, SourceType};
use crate::vm::ExecOutcome;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

pub(crate) fn doctor_json(stack: &LoadedStack, socket_path: &Path) -> Result<Value, String> {
    let mut checks = Vec::new();
    let mut ok = true;
    for source_type in configured_types(&stack.environment.spec.observability.data_sources) {
        let type_config = match type_config_for(stack, source_type) {
            Ok(value) => value,
            Err(message) => {
                ok = false;
                checks.push(json!({
                    "type": source_type.as_str(),
                    "status": "fail",
                    "message": message,
                }));
                continue;
            }
        };
        let via_key = type_config.via.clone();
        let vm_id = crate::runtime_vm_id(&stack.environment.spec.project, &via_key);
        let tool = source_type.tool_binary();
        let which = crate::vm::exec_in_vm(
            &vm_id,
            &["which".into(), tool.into()],
            None,
            &Default::default(),
            10_000,
            socket_path,
        );
        match which {
            Ok(outcome) if outcome.exit == 0 => {
                checks.push(json!({
                    "type": source_type.as_str(),
                    "status": "ok",
                    "via": via_key,
                    "vmId": vm_id,
                    "tool": tool,
                    "path": outcome.stdout.trim(),
                }));
            }
            Ok(outcome) => {
                ok = false;
                checks.push(json!({
                    "type": source_type.as_str(),
                    "status": "fail",
                    "via": via_key,
                    "vmId": vm_id,
                    "tool": tool,
                    "message": format!("{} not found in via VM PATH", tool),
                    "stderr": outcome.stderr.trim(),
                }));
            }
            Err(message) => {
                ok = false;
                checks.push(json!({
                    "type": source_type.as_str(),
                    "status": "fail",
                    "via": via_key,
                    "vmId": vm_id,
                    "tool": tool,
                    "message": message,
                }));
            }
        }
    }
    Ok(json!({
        "checks": checks,
        "ok": ok,
    }))
}

fn type_config_for(stack: &LoadedStack, source_type: SourceType) -> Result<TypeVia, String> {
    let data_sources = &stack.environment.spec.observability.data_sources;
    Ok(match source_type {
        SourceType::Postgres => TypeVia {
            via: data_sources
                .postgres
                .as_ref()
                .ok_or_else(|| "postgres not configured".to_string())?
                .via
                .clone(),
        },
        SourceType::Mysql => TypeVia {
            via: data_sources
                .mysql
                .as_ref()
                .ok_or_else(|| "mysql not configured".to_string())?
                .via
                .clone(),
        },
        SourceType::Nats => TypeVia {
            via: data_sources
                .nats
                .as_ref()
                .ok_or_else(|| "nats not configured".to_string())?
                .via
                .clone(),
        },
    })
}

struct TypeVia {
    via: String,
}

pub(crate) fn exec_with_timing(
    vm_id: &str,
    cmd: &[String],
    env: &std::collections::BTreeMap<String, String>,
    timeout_ms: u64,
    socket_path: &Path,
) -> Result<(ExecOutcome, u64), String> {
    let started = Instant::now();
    let outcome = crate::vm::exec_in_vm(vm_id, cmd, None, env, timeout_ms, socket_path)?;
    Ok((outcome, started.elapsed().as_millis() as u64))
}

pub(crate) fn exec_json(outcome: &ExecOutcome, duration_ms: u64) -> Value {
    json!({
        "exit": outcome.exit,
        "stdout": outcome.stdout,
        "stderr": outcome.stderr,
        "truncated": outcome.truncated,
        "durationMs": duration_ms,
    })
}

pub(crate) fn empty_data_sources_message() -> String {
    "observability.dataSources is empty; configure postgres, mysql, or nats sources".into()
}

pub(crate) fn ensure_data_sources(config: &DataSourcesConfig) -> Result<(), String> {
    if config.is_empty() {
        Err(empty_data_sources_message())
    } else {
        Ok(())
    }
}
