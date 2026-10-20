//! Independent claim reconciliation with private durable state and redacted status.
use super::storage::{digest, read_private, write_atomic, FileSecretStore, Lock};
use super::*;
use base64::Engine;
use serde_json::{json, Value};
use std::process::ExitCode;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Registration {
    project: String,
    generation: String,
    config_path: String,
}
pub(super) struct RpcGuest<'a> {
    socket: &'a Path,
    registration: Option<&'a Registration>,
}
impl RpcGuest<'_> {
    fn check_registration(&self) -> ControllerResult<()> {
        if let Some(registration) = self.registration {
            let value = crate::vm::supervisor_rpc_deadline(
                self.socket,
                "controller.registration",
                json!({"project":registration.project}),
                5,
            )
            .map_err(|_| {
                ControllerError::new(
                    "RegistrationUnavailable",
                    "controller registration cannot be verified",
                )
            })?;
            if value["enabled"] != true
                || value["generation"] != registration.generation
                || value["config_path"] != registration.config_path
            {
                return Err(ControllerError::new(
                    "RegistrationRevoked",
                    "controller registration was disabled or replaced",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
impl GuestExec for RpcGuest<'_> {
    fn check_permission(&self) -> ControllerResult<()> {
        self.check_registration()
    }
    fn run(
        &self,
        vm: &str,
        argv: &[String],
        env: &BTreeMap<String, String>,
        stdin: &[u8],
        timeout_ms: u64,
    ) -> ControllerResult<ExecOutput> {
        self.check_registration()?;
        if stdin.len() > 256 * 1024 || timeout_ms == 0 || timeout_ms > 120_000 {
            return Err(ControllerError::new(
                "InvalidOperation",
                "controller operation exceeds limits",
            ));
        }
        let value=crate::vm::supervisor_rpc_deadline(self.socket,"vm.exec",json!({
            "vm_id":vm,"cmd":argv,"env":env,
            "stdin_b64":base64::engine::general_purpose::STANDARD.encode(stdin),"timeout_ms":timeout_ms,
        }),timeout_ms.div_ceil(1000)+10).map_err(|_|ControllerError::new("GuestUnavailable","guest execution failed or timed out"))?;
        Ok(ExecOutput {
            exit_code: value["exit"].as_i64().unwrap_or(-1) as i32,
            stdout: value["stdout"].as_str().unwrap_or("").to_string(),
            stderr: value["stderr"].as_str().unwrap_or("").to_string(),
            truncated: value["truncated"].as_bool().unwrap_or(false),
        })
    }
}
fn directory(state: &Path, project: &str) -> PathBuf {
    state.join("controllers").join(digest(project))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn provider<'a>(
    environment: &'a crate::config::Environment,
    claim: &ClaimConfig,
) -> ControllerResult<&'a ProviderConfig> {
    if let Some(name) = &claim.provider {
        return environment
            .spec
            .service_providers
            .get(name)
            .filter(|p| p.kind == claim.kind)
            .ok_or_else(|| {
                ControllerError::new("InvalidProvider", "provider missing or incompatible")
            });
    }
    let mut candidates = environment
        .spec
        .service_providers
        .values()
        .filter(|p| p.kind == claim.kind);
    let selected = candidates
        .next()
        .ok_or_else(|| ControllerError::new("InvalidProvider", "no compatible provider"))?;
    if candidates.next().is_some() {
        return Err(ControllerError::new(
            "AmbiguousProvider",
            "multiple compatible providers",
        ));
    }
    Ok(selected)
}

fn desired_hash(
    environment: &crate::config::Environment,
    claim: &ClaimConfig,
) -> ControllerResult<String> {
    let selected = provider(environment, claim);
    let config = serde_json::to_string(&(
        claim,
        selected.as_ref().ok(),
        &environment.spec.ingress,
        &environment.spec.oidc,
        &environment.spec.kubernetes,
    ))
    .map_err(|_| ControllerError::new("InvalidClaim", "claim cannot be encoded"))?;
    Ok(digest(&config))
}

pub(crate) fn reconcile(
    environment: &crate::config::Environment,
    config_file: &Path,
    socket: &Path,
    state: &Path,
) -> ControllerResult<Value> {
    reconcile_guarded(environment, config_file, socket, state, None)
}
fn reconcile_guarded(
    environment: &crate::config::Environment,
    config_file: &Path,
    socket: &Path,
    state: &Path,
    registration: Option<&Registration>,
) -> ControllerResult<Value> {
    let directory = directory(state, &environment.spec.project);
    let _lock = Lock::acquire(&directory)?;
    let secret_store = FileSecretStore::new(directory.join("secrets"))?;
    let guest = RpcGuest {
        socket,
        registration,
    };
    guest.check_registration()?;
    let stack_hash = digest(
        &serde_json::to_string(environment)
            .map_err(|_| ControllerError::new("InvalidConfig", "stack cannot be encoded"))?,
    );
    let discovered = super::kubernetes_claims::discover(environment, &guest)?;
    let environment = &discovered.environment;
    let mut warnings = serde_json::to_value(&discovered.warnings)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    guest.check_registration()?;
    let (mut edge_results, edge_warnings) = super::edge::reconcile(
        environment,
        config_file,
        socket,
        state,
        &guest,
        discovered.warnings.is_empty()
            && !discovered
                .sources
                .iter()
                .any(|source| source.validation_error.is_some() && source.affects_edge()),
    );
    warnings.extend(
        edge_warnings
            .into_iter()
            .map(|error| json!({"code":error.code,"message":error.message})),
    );
    let mut claims = Vec::new();
    for (name, claim) in &environment.spec.service_claims {
        guest.check_registration()?;
        let source = discovered
            .sources
            .iter()
            .find(|source| source.claim_name == *name);
        let record_path = directory.join(format!("{}.json", digest(name)));
        let previous = if record_path.exists() {
            serde_json::from_slice::<Value>(&read_private(&record_path)?)
                .map_err(|_| ControllerError::new("InvalidState", "controller status is invalid"))?
        } else {
            Value::Null
        };
        let selected = provider(environment, claim);
        let desired = desired_hash(environment, claim)?;
        let generation = source.map(|source| source.generation).unwrap_or_else(|| {
            previous["observedGeneration"].as_u64().unwrap_or(0)
                + u64::from(previous["desiredHash"] != desired)
        });
        let result=if matches!(claim.kind,ServiceKind::Ingress|ServiceKind::Oidc) {
            edge_results.remove(name).unwrap_or_else(||Err(ControllerError::new("InvalidClaim","edge claim result missing")))
        } else { selected.and_then(|provider|{
            let context=ControllerContext{project:&environment.spec.project,claim_name:name,provider,
                advertised_endpoint:provider.advertise_endpoint.as_deref().or(provider.endpoint.as_deref()).unwrap_or(""),
                claim,config_dir:config_file.parent().unwrap_or_else(||Path::new(".")),guest:&guest,secrets:&secret_store};
            match claim.kind {
                ServiceKind::Postgres=>super::postgres::PostgresController.reconcile(&context),
                ServiceKind::S3=>super::s3::S3Controller.reconcile(&context),
                ServiceKind::Ingress|ServiceKind::Oidc=>unreachable!("edge claims handled above"),
            }
        }) }.and_then(|result|{
            // Private binding first: a failed delivery can be retried without rotating credentials.
            let bytes=serde_json::to_vec(&json!({"owner":format!("vzctl:{}/{name}",environment.spec.project),"public":result.binding.public,"secrets":result.binding.secrets}))
                .map_err(|_|ControllerError::new("InvalidBinding","binding cannot be encoded"))?;
            write_atomic(&directory.join("bindings").join(format!("{}.json",digest(name))),&bytes)?;
            super::bindings::deliver(environment,name,claim,&result.binding,&guest)?;
            Ok(result)
        });
        let observed = now();
        let mut record = json!({"name":name,"kind":claim.kind,"desiredHash":desired,"observedGeneration":generation,
            "observedAt":observed,"lastSuccess":previous["lastSuccess"],"state":"Degraded","resource":previous["resource"],"binding":previous["binding"]});
        match result {
            Ok(result) => {
                record["state"] = json!("Ready");
                record["lastSuccess"] = json!(observed);
                record["changed"] = json!(result.changed || previous["desiredHash"] != desired);
                record["binding"] = json!(result.binding.public);
                record["resource"] = result.resource;
                record["conditions"] =
                    json!([{"type":"Ready","status":"True","reason":"Reconciled"}]);
            }
            Err(error) => {
                record["state"] = json!(if error.code.contains("Conflict") {
                    "Conflict"
                } else {
                    "Degraded"
                });
                record["conditions"] = json!([{"type":"Ready","status":"False","reason":error.code,"message":error.message}]);
            }
        }
        if let Some(source) = source {
            record["source"] = json!({"type":"kubernetes","cluster":source.cluster,"namespace":source.namespace,"name":source.name,"uid":source.uid});
            if let Err(error) = super::kubernetes_claims::update_status(source, &record, &guest) {
                warnings.push(json!({"claim":name,"code":error.code,"message":error.message}));
            }
        }
        write_atomic(
            &record_path,
            &serde_json::to_vec(&record)
                .map_err(|_| ControllerError::new("InvalidState", "status cannot be encoded"))?,
        )?;
        claims.push(record);
    }
    for source in &discovered.sources {
        if let Some((code, message)) = &source.validation_error {
            let record = json!({"name":source.claim_name,"state":"Degraded","observedGeneration":source.generation,"observedAt":now(),
                "source":{"type":"kubernetes","cluster":source.cluster,"namespace":source.namespace,"name":source.name,"uid":source.uid},
                "conditions":[{"type":"Ready","status":"False","reason":code,"message":message}]});
            if let Err(error) = super::kubernetes_claims::update_status(source, &record, &guest) {
                warnings.push(
                    json!({"claim":source.claim_name,"code":error.code,"message":error.message}),
                );
            }
            claims.push(record);
        }
    }
    let snapshot_path = directory.join("snapshot.json");
    if snapshot_path.exists() {
        let previous: Value = serde_json::from_slice(&read_private(&snapshot_path)?)
            .map_err(|_| ControllerError::new("InvalidState", "controller snapshot is invalid"))?;
        if let Some(previous) = previous["claims"].as_array() {
            for old in previous.iter().take(512) {
                if claims.iter().any(|claim| claim["name"] == old["name"]) {
                    continue;
                }
                let mut retained = old.clone();
                if warnings.is_empty() || old["source"]["type"] != "kubernetes" {
                    retained["state"] = json!("Retained");
                    retained["conditions"] = json!([{"type":"Ready","status":"False","reason":"ClaimRemoved","message":"provisioned data and credentials retained"}]);
                } else {
                    retained["state"] = json!("Degraded");
                    retained["conditions"] = json!([{"type":"Ready","status":"Unknown","reason":"DiscoveryUnavailable"}]);
                }
                claims.push(retained);
            }
        }
    }
    let snapshot = json!({"project":environment.spec.project,"stackHash":stack_hash,"claims":claims,"warnings":warnings,"observedAt":now()});
    write_atomic(
        &snapshot_path,
        &serde_json::to_vec(&snapshot).map_err(|_| {
            ControllerError::new("InvalidState", "controller snapshot cannot be encoded")
        })?,
    )?;
    Ok(snapshot)
}
fn previous_stack_changed(
    environment: &crate::config::Environment,
    previous: &Value,
) -> ControllerResult<bool> {
    Ok(*previous
        != digest(
            &serde_json::to_string(environment)
                .map_err(|_| ControllerError::new("InvalidConfig", "stack cannot be encoded"))?,
        ))
}

fn status(environment: &crate::config::Environment, state: &Path) -> ControllerResult<Value> {
    let directory = directory(state, &environment.spec.project);
    let mut claims = Vec::new();
    for (name, claim) in &environment.spec.service_claims {
        let path = directory.join(format!("{}.json", digest(name)));
        if path.exists() {
            let mut record =
                serde_json::from_slice::<Value>(&read_private(&path)?).map_err(|_| {
                    ControllerError::new("InvalidState", "controller status is invalid")
                })?;
            if record["desiredHash"] != desired_hash(environment, claim)? {
                record["state"] = json!("Pending");
                record["conditions"] = json!([{"type":"Ready","status":"False","reason":"DesiredStateChanged","message":"configuration changed since last reconcile"}]);
            }
            claims.push(record);
        } else {
            claims.push(
                json!({"name":name,"kind":claim.kind,"state":"Pending","observedGeneration":0}),
            );
        }
    }
    let snapshot_path = directory.join("snapshot.json");
    let previous: Value = if snapshot_path.exists() {
        serde_json::from_slice(&read_private(&snapshot_path)?)
            .map_err(|_| ControllerError::new("InvalidState", "controller snapshot is invalid"))?
    } else {
        Value::Null
    };
    let previous_snapshot_hash = previous["stackHash"].clone();
    if let Some(previous) = previous["claims"].as_array() {
        for old in previous.iter().take(512) {
            if claims.iter().any(|claim| claim["name"] == old["name"]) {
                continue;
            }
            let mut record = old.clone();
            if record["source"]["type"] != "kubernetes" {
                record["state"] = json!("Retained");
                record["conditions"] =
                    json!([{"type":"Ready","status":"False","reason":"ClaimRemoved"}]);
            } else if previous_stack_changed(environment, &previous_snapshot_hash)?
                && record["state"] != "Retained"
            {
                record["state"] = json!("Pending");
                record["conditions"] = json!([{"type":"Ready","status":"False","reason":"DesiredStateChanged","message":"stack provider or cluster configuration changed since last reconcile"}]);
            }
            claims.push(record);
        }
    }
    Ok(
        json!({"project":environment.spec.project,"claims":claims,"warnings":previous["warnings"],"source":"last-reconcile","observedAt":previous["observedAt"]}),
    )
}

fn with_supervisor_status(mut data: Value, project: &str, socket: &Path) -> Value {
    if !data["warnings"].is_array() {
        data["warnings"] = json!([]);
    }
    match crate::vm::supervisor_rpc_deadline(
        socket,
        "controller.registration",
        json!({"project":project}),
        5,
    ) {
        Ok(registration) => {
            data["supervisor"] = json!({"enabled":registration["enabled"],"generation":registration["generation"],
                "lastStatus":registration["lastStatus"],"lastAttemptAt":registration["lastAttemptAt"]});
            if registration["enabled"] == true && registration["lastStatus"]["state"] == "Degraded"
            {
                data["warnings"].as_array_mut().unwrap().push(json!({"code":"SupervisorReconcileFailed","message":"last automatic reconcile failed; inspect supervisor.lastStatus"}));
            }
        }
        Err(_) => {
            data["supervisor"] = json!({"state":"Unavailable"});
            data["warnings"].as_array_mut().unwrap().push(json!({"code":"SupervisorUnavailable","message":"automatic reconciliation state could not be verified"}));
        }
    }
    data
}

pub(crate) fn run(args: impl Iterator<Item = String>, socket: &Path) -> ExitCode {
    let args: Vec<_> = args.collect();
    let json_output = args.windows(2).any(|v| v == ["--format", "json"]);
    let action = args.first().map(String::as_str).unwrap_or("");
    let command = if ["status", "reconcile", "watch"].contains(&action) {
        format!("controller.{action}")
    } else {
        "controller".into()
    };
    let mut config = PathBuf::from(".");
    let mut interval = 30u64;
    let mut registered = None;
    let parsed = (|| {
        if !["status", "reconcile", "watch"].contains(&action) {
            return Err("expected status|reconcile|watch");
        }
        let mut index = 1;
        while index < args.len() {
            let value = args.get(index + 1).ok_or("option needs a value")?;
            match args[index].as_str() {
                "-C" | "--config" => config = PathBuf::from(value),
                "--format" if ["human", "json"].contains(&value.as_str()) => {}
                "--registered"
                    if action == "reconcile" && !value.is_empty() && value.len() <= 128 =>
                {
                    registered = Some(value.clone())
                }
                "--interval" if action == "watch" => {
                    interval = value
                        .parse()
                        .ok()
                        .filter(|v| (5..=3600).contains(v))
                        .ok_or("interval must be 5..3600 seconds")?
                }
                _ => return Err("unknown controller option"),
            }
            index += 2;
        }
        Ok(())
    })();
    let emit = |status: &str, code: u8, data: Value| {
        let output = json!({"apiVersion":"vzctl.dev/v1","command":command,"status":status,"exit_code":code,"summary":{"message":if code==0{"controller state available"}else{"controller requires attention"},"claims":data["claims"].as_array().map_or(0,Vec::len)},"data":data});
        if json_output {
            println!("{output}");
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
    };
    if let Err(message) = parsed {
        emit("fail", 2, json!({"code":"Usage","message":message}));
        return ExitCode::from(2);
    }
    let stop = Arc::new(AtomicBool::new(false));
    if action == "watch" {
        let flag = stop.clone();
        if ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).is_err() {
            emit("fail", 24, json!({"code":"SignalHandlerUnavailable"}));
            return ExitCode::from(24);
        }
    }
    loop {
        let file = crate::config::config_path(&config);
        let result = crate::config::validate_path(&file)
            .map_err(|_| {
                ControllerError::new(
                    "InvalidConfig",
                    "stack config validation failed; run vzctl validate for details",
                )
            })
            .and_then(|environment| {
                if action == "status" {
                    status(&environment, &crate::state_dir())
                        .map(|data| with_supervisor_status(data, &environment.spec.project, socket))
                } else {
                    let registration = registered.as_ref().map(|generation| Registration {
                        project: environment.spec.project.clone(),
                        generation: generation.clone(),
                        config_path: std::fs::canonicalize(&file)
                            .unwrap_or_else(|_| file.clone())
                            .display()
                            .to_string(),
                    });
                    reconcile_guarded(
                        &environment,
                        &file,
                        socket,
                        &crate::state_dir(),
                        registration.as_ref(),
                    )
                }
            });
        let code = match result {
            Ok(data) => {
                let ready = data["claims"].as_array().is_some_and(|claims| {
                    claims
                        .iter()
                        .all(|claim| claim["state"] == "Ready" || claim["state"] == "Retained")
                }) && data["warnings"].as_array().is_none_or(Vec::is_empty);
                emit(
                    if ready { "ok" } else { "warn" },
                    if ready { 0 } else { 24 },
                    data,
                );
                if ready {
                    0
                } else {
                    24
                }
            }
            Err(error) => {
                emit(
                    "fail",
                    24,
                    json!({"code":error.code,"message":error.message}),
                );
                24
            }
        };
        if action != "watch" {
            return ExitCode::from(code);
        }
        for _ in 0..interval * 10 {
            if stop.load(Ordering::SeqCst) {
                return ExitCode::SUCCESS;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
