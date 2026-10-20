//! Explicit live contract: only a fresh, exclusively created test namespace.
//! Opt-in and private plan are supplied by the reviewed offline runner.
use super::{controller, manifests};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

fn require(ok: bool, code: &'static str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(code.into())
    }
}
fn identity(value: &Value) -> Result<(String, String), String> {
    let uid = value["metadata"]["uid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("contract_uid_missing")?;
    let rv = value["metadata"]["resourceVersion"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("contract_rv_missing")?;
    Ok((uid.into(), rv.into()))
}
fn bytes(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|_| "contract_json".into())
}

fn resource(name: &str) -> (&'static str, &'static str) {
    if name == "service-default" {
        ("Service", "services")
    } else {
        ("ConfigMap", "configmaps")
    }
}
fn expected_error(result: &Result<(), String>, case: &str) -> bool {
    let Err(error) = result else {
        return false;
    };
    let lower = error.to_lowercase();
    match case {
        "foreign_fields" => {
            lower.contains("conflict")
                && error.contains("f47-foreign")
                && error.contains(".data.value")
        }
        "foreign_owner" => {
            error.starts_with("ownership conflict:")
                && error.contains("already exists outside this cluster controller")
        }
        "uid" => {
            error.contains("resource was replaced during apply")
                || error.contains("resource identity changed during field migration")
                || (lower.contains("uid in precondition:") && lower.contains("uid in object meta:"))
                || (lower.contains("metadata.uid") && lower.contains("field is immutable"))
        }
        _ => false,
    }
}
fn persist(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let next = path.with_extension("next");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&next)
        .map_err(|_| "contract_ledger_create")?;
    file.write_all(&bytes(value)?)
        .map_err(|_| "contract_ledger_write")?;
    file.sync_all().map_err(|_| "contract_ledger_sync")?;
    fs::rename(next, path).map_err(|_| "contract_ledger_commit".into())
}

fn diagnostic_error(error: &str, name: &str, namespace: &str) -> Value {
    let fixed = [
        "ambiguous desired list key",
        "ambiguous managed list element",
        "field migration requires UID",
        "field migration requires resourceVersion",
        "incompatible atomic/structured field difference",
        "incompatible atomic/structured field intersection",
        "incompatible atomic/structured managed field ownership",
        "invalid desired list",
        "invalid desired set",
        "invalid field difference",
        "invalid field difference child",
        "invalid field intersection",
        "invalid field intersection child",
        "invalid intent list fields",
        "invalid intent list key",
        "invalid managed field set",
        "invalid managed intent child",
        "invalid managed intent fields",
        "invalid managed list key",
        "invalid managedFields inventory",
        "managed field missing from current object",
        "managed field projection exceeds depth limit",
        "managed field structure does not match current object",
        "managed intent exceeds depth limit",
        "managed keyed list is not a map",
        "managed list element missing from current object",
        "managedFields inventory exceeds migration limit",
        "overlapping managed list element selectors",
        "unsupported managed intent encoding",
        "unsupported managed list field encoding",
        "unsupported managed list key value",
        "unsupported managed map field",
        "unsupported own managed field format",
        "resource values changed during field ownership bridge",
        "field ownership bridge unexpectedly removed Create ownership",
        "field ownership bridge did not establish complete Apply ownership",
        "field ownership migration refuses an empty managedFields reset",
        "field ownership migration did not preserve resource and foreign managers",
        "ownership conflict: resource disappeared during field migration",
        "ownership conflict: resource identity changed during field migration",
    ];
    let lower = error.to_lowercase();
    let sensitive = [
        "secret",
        "password",
        "token",
        "kubeconfig",
        "authorization",
        "private key",
        "client-key",
        "certificate-data",
    ]
    .iter()
    .any(|word| lower.contains(word));
    let allowed = error.len() <= 8192
        && !sensitive
        && (fixed.contains(&error)
            || failure_code(error) != "contract_unclassified_failure"
            || (error.starts_with("K3s node server: ")
                && error.contains(name)
                && error.contains(namespace)));
    json!({"class":if allowed {"synthetic_error"} else {"unclassified_error_redacted"},
        "sha256":format!("{:x}",Sha256::digest(error.as_bytes())),"bytes":error.len(),
        "message":if allowed {Some(error)} else {None}})
}

fn diagnostic_object(value: &Value, name: &str, namespace: &str) -> Value {
    let object =
        if value["kind"] == "List" && value["items"].as_array().is_some_and(|a| a.len() == 1) {
            &value["items"][0]
        } else {
            value
        };
    let permitted_data = resource(name).0 == "Service"
        || object["data"].as_object().is_some_and(|data| {
            data.len() == 1
                && matches!(
                    data.get("value").and_then(Value::as_str),
                    Some("old" | "new" | "foreign" | "replacement")
                )
        });
    if object["kind"] != resource(name).0
        || object["metadata"]["name"] != name
        || object["metadata"]["namespace"] != namespace
        || !permitted_data
        || identity(object).is_err()
    {
        return json!({"class":"unexpected_object_redacted"});
    }
    let projection = json!({"kind":object["kind"],"name":name,"namespace":namespace,
        "uid":object["metadata"]["uid"],"resourceVersion":object["metadata"]["resourceVersion"],
        "managedFields":object["metadata"]["managedFields"]});
    let raw = serde_json::to_vec(&projection).unwrap_or_default();
    if raw.len() > 16384 {
        json!({"class":"managed_fields_limit","sha256":format!("{:x}",Sha256::digest(raw))})
    } else {
        projection
    }
}

struct Harness<'a> {
    guest: controller::Guest<'a>,
    cluster: super::ClusterConfig,
    namespace: String,
    namespace_uid: String,
    created: BTreeMap<String, Vec<String>>,
    automatic: BTreeMap<String, Value>,
    phase: &'static str,
    migrations: usize,
    ledger: std::path::PathBuf,
    diagnostics: Vec<Value>,
    uid_replacement: Option<Value>,
}
impl Harness<'_> {
    fn diagnostic(&mut self, event: Value) -> Result<(), String> {
        require(self.diagnostics.len() < 512, "contract_diagnostic_limit")?;
        self.diagnostics.push(event);
        persist(
            &self.ledger.with_file_name("synthetic-diagnostics.json"),
            &json!({"schema":"vzctl.f47-synthetic-diagnostics/v1","phase":self.phase,"events":self.diagnostics}),
        )
    }
    fn persist(&self) -> Result<(), String> {
        persist(
            &self.ledger,
            &json!({"namespace":self.namespace,"namespace_uid":self.namespace_uid,"created_uids":self.created,"automatic":self.automatic,"phase":self.phase}),
        )
    }
    fn call(&self, args: &[&str], input: &[u8]) -> Result<String, String> {
        // This is the existing product guest transport; no shell, kubeconfig export or service mutation.
        self.guest
            .kubectl(&self.cluster, args, input, 20)
            .map_err(|_| "contract_kubectl_failed".into())
    }
    fn get(&self, name: &str) -> Result<Value, String> {
        serde_json::from_str(&self.call(
            &[
                "get",
                resource(name).1,
                name,
                "--show-managed-fields=true",
                "-o",
                "json",
            ],
            &[],
        )?)
        .map_err(|_| "contract_inventory".into())
    }
    fn remember(&mut self, name: &str, value: &Value) -> Result<(), String> {
        require(
            value["kind"] == resource(name).0
                && value["metadata"]["namespace"] == self.namespace
                && value["metadata"]["name"] == name,
            "contract_created_identity",
        )?;
        let (uid, _) = identity(value)?;
        self.created.entry(name.into()).or_default().push(uid);
        self.persist()
    }
    fn create(&mut self, name: &str, value: &Value, manager: &str) -> Result<Value, String> {
        require(
            ["vzctl-kubernetes", "f47-foreign"].contains(&manager),
            "contract_fixture_manager",
        )?;
        let flag = format!("--field-manager={manager}");
        let result = self.call(
            &[
                "create",
                &flag,
                "--show-managed-fields=true",
                "-f",
                "-",
                "-o",
                "json",
            ],
            &bytes(value)?,
        )?;
        let created: Value =
            serde_json::from_str(&result).map_err(|_| "contract_create_inventory")?;
        self.remember(name, &created)?;
        Ok(created)
    }
    fn desired(&self, name: &str, value: &str) -> Result<Value, String> {
        manifests::owned(
            json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":name,"namespace":self.namespace,"labels":{"vzctl.dev/f47-contract":self.namespace}},"data":{"value":value}}),
            "controller-lab",
            "dev",
        )
    }
    fn delete_exact(&self, name: &str, uid: &str, rv: &str) -> Result<(), String> {
        let path = format!(
            "/api/v1/namespaces/{}/{}/{name}",
            self.namespace,
            resource(name).1
        );
        let options = json!({"apiVersion":"v1","kind":"DeleteOptions","preconditions":{"uid":uid,"resourceVersion":rv}});
        self.call(&["delete", "--raw", &path, "-f", "-"], &bytes(&options)?)
            .map(|_| ())
    }
    fn apply(
        &mut self,
        name: &str,
        desired: &Value,
        hook: &str,
    ) -> Result<(Result<(), String>, usize, bool), String> {
        require(["none", "uid", "rv"].contains(&hook), "contract_hook")?;
        let mut fired = false;
        let mut applies = 0usize;
        let mut operation_count = 0usize;
        let namespace = self.namespace.clone();
        let cluster = self.cluster.clone();
        let result = controller::apply_owned_object(
            "controller-lab",
            "dev",
            &cluster,
            desired,
            |args, input, seconds| {
                operation_count += 1;
                let operation = args.first().copied().unwrap_or("invalid");
                require(
                    ["get", "create", "apply", "patch"].contains(&operation),
                    "contract_command_scope",
                )?;
                self.diagnostic(json!({"stage":"before","operation":operation,"sequence":operation_count,"name":name}))?;
                if seconds > 30 || args.iter().any(|a| a.starts_with("--force")) {
                    return Err("contract_forbidden_command".into());
                }
                let payload: Value =
                    serde_json::from_slice(input).map_err(|_| "contract_payload")?;
                if payload["kind"] != resource(name).0
                    || payload["metadata"]["name"] != name
                    || payload["metadata"]["namespace"] != namespace
                {
                    return Err("contract_foreign_target".into());
                }
                if !["get", "create", "apply", "patch"].contains(&args[0]) {
                    return Err("contract_command_scope".into());
                }
                if args[0] == "apply" {
                    applies += 1;
                    if !fired && hook != "none" {
                        fired = true;
                        let current = self.get(name)?;
                        let (uid, rv) = identity(&current)?;
                        if hook == "uid" {
                            self.delete_exact(name, &uid, &rv)?;
                            // Identical complete desired intent (including manifest hash) avoids a
                            // foreign field-value conflict masking the immutable UID precondition.
                            let replacement = desired.clone();
                            let created = self.create(name, &replacement, "f47-foreign")?;
                            require(
                                identity(&created)?.0 != uid
                                    && manifests::matches_desired(desired, &created),
                                "contract_uid_fixture",
                            )?;
                            self.uid_replacement = Some(created);
                        } else {
                            let patch = json!([{"op":"test","path":"/metadata/uid","value":uid},{"op":"test","path":"/metadata/resourceVersion","value":rv},{"op":"add","path":"/metadata/annotations/f47-race","value":"preserve"}]);
                            let text =
                                serde_json::to_string(&patch).map_err(|_| "contract_patch")?;
                            self.call(
                                &[
                                    "patch",
                                    "configmap",
                                    name,
                                    "--type=json",
                                    "--field-manager=f47-racer",
                                    "--patch",
                                    &text,
                                ],
                                &[],
                            )?;
                        }
                    }
                }
                let before_patch = if args[0] == "patch" {
                    require(
                        args.contains(&"--field-manager=vzctl-kubernetes-migration"),
                        "contract_patch_manager",
                    )?;
                    Some(self.get(name)?)
                } else {
                    None
                };
                // Preserve real guest diagnostics only inside product classification; never print them.
                if let Some(before) = &before_patch {
                    self.diagnostic(json!({"stage":"before_patch_inventory","name":name,
                        "object":diagnostic_object(before,name,&namespace)}))?;
                }
                let output = match self.guest.kubectl(&cluster, args, input, seconds) {
                    Ok(output) => output,
                    Err(error) => {
                        self.diagnostic(json!({"stage":"guest_error","operation":operation,"sequence":operation_count,
                            "error":diagnostic_error(&error,name,&namespace)}))?;
                        return Err(error);
                    }
                };
                self.diagnostic(
                    json!({"stage":"completed","operation":operation,"sequence":operation_count}),
                )?;
                if operation == "get" {
                    let parsed = serde_json::from_str::<Value>(&output).unwrap_or(Value::Null);
                    self.diagnostic(json!({"stage":"get_inventory","name":name,
                        "object":diagnostic_object(&parsed,name,&namespace)}))?;
                }
                if let Some(before) = before_patch {
                    let after = self.get(name)?;
                    self.diagnostic(json!({"stage":"after_patch_inventory","name":name,
                        "object":diagnostic_object(&after,name,&namespace)}))?;
                    let foreign = |v: &Value| -> Result<Vec<Value>, String> {
                        Ok(v["metadata"]["managedFields"]
                            .as_array()
                            .ok_or("contract_managed_fields")?
                            .iter()
                            .filter(|e| e["manager"] != "vzctl-kubernetes")
                            .cloned()
                            .collect())
                    };
                    require(
                        foreign(&before)? == foreign(&after)?
                            && super::managed_fields::same_values(&before, &after),
                        "contract_patch_inventory_changed",
                    )?;
                    require(
                        !after["metadata"]["managedFields"]
                            .as_array()
                            .ok_or("contract_managed_fields")?
                            .iter()
                            .any(|e| e["manager"] == "vzctl-kubernetes-migration"),
                        "contract_migration_manager_added",
                    )?;
                    self.migrations += 1;
                }
                if args[0] == "create" {
                    let created: Value =
                        serde_json::from_str(&output).map_err(|_| "contract_create_inventory")?;
                    self.remember(name, &created)?;
                }
                Ok(output)
            },
        );
        if let Err(error) = &result {
            self.diagnostic(json!({"stage":"product_error","name":name,
                "error":diagnostic_error(error,name,&namespace)}))?;
        }
        Ok((result, applies, fired))
    }
    fn inventory(&self) -> Result<Vec<Value>, String> {
        let resources = self.call(
            &[
                "api-resources",
                "--verbs=list",
                "--namespaced=true",
                "--cached=false",
                "-o",
                "name",
            ],
            &[],
        )?;
        let names: std::collections::BTreeSet<_> = resources
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        require(
            !names.is_empty()
                && names.len() <= 128
                && names.iter().all(|s| {
                    s.len() < 256
                        && s.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
                        })
                }),
            "contract_api_inventory",
        )?;
        let names = names.into_iter().collect::<Vec<_>>().join(",");
        let raw = self.call(&["get", &names, "-o", "json"], &[])?;
        let values: Value = serde_json::from_str(&raw).map_err(|_| "contract_full_inventory")?;
        let items = values["items"]
            .as_array()
            .ok_or("contract_full_inventory")?;
        require(
            items.len() <= 32
                && items
                    .iter()
                    .all(|v| v["metadata"]["namespace"] == self.namespace),
            "contract_inventory_scope",
        )?;
        Ok(items.clone())
    }
    fn capture_automatic(&mut self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let items = self.inventory()?;
            let mut known = BTreeMap::new();
            for item in items {
                let key = format!(
                    "{}/{}",
                    item["kind"].as_str().unwrap_or("?"),
                    item["metadata"]["name"].as_str().unwrap_or("?")
                );
                require(
                    ["ConfigMap/kube-root-ca.crt", "ServiceAccount/default"]
                        .contains(&key.as_str()),
                    "contract_namespace_not_exclusive",
                )?;
                identity(&item)?;
                if key == "ConfigMap/kube-root-ca.crt" {
                    require(
                        item["data"].as_object().is_some_and(|v| {
                            v.len() == 1 && v.get("ca.crt").is_some_and(Value::is_string)
                        }),
                        "contract_automatic_ca",
                    )?;
                } else {
                    require(
                        item["secrets"].as_array().is_none_or(Vec::is_empty)
                            && item["imagePullSecrets"]
                                .as_array()
                                .is_none_or(Vec::is_empty),
                        "contract_automatic_serviceaccount",
                    )?;
                }
                require(
                    known.insert(key, item).is_none(),
                    "contract_duplicate_inventory",
                )?;
            }
            if known.len() == 2 {
                self.automatic = known;
                return self.persist();
            }
            require(
                std::time::Instant::now() < deadline,
                "contract_automatic_deadline",
            )?;
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    fn cleanup(&self) -> Result<(), String> {
        // Delete only exact UIDs returned by successful creates, with fresh RV preconditions.
        for (name, uids) in &self.created {
            let value = self.get(name)?;
            let (uid, rv) = identity(&value)?;
            require(uids.contains(&uid), "contract_cleanup_replacement")?;
            self.delete_exact(name, &uid, &rv)?;
        }
        let objects = self.inventory()?;
        require(
            self.automatic.len() == 2 && objects.len() == 2,
            "contract_cleanup_foreign_object",
        )?;
        for value in objects {
            let key = format!(
                "{}/{}",
                value["kind"].as_str().unwrap_or("?"),
                value["metadata"]["name"].as_str().unwrap_or("?")
            );
            let expected = self
                .automatic
                .get(&key)
                .ok_or("contract_cleanup_foreign_object")?;
            // UID and all public payload fields remain exact. Only bookkeeping may refresh.
            let mut now = value.clone();
            let mut before = expected.clone();
            for object in [&mut now, &mut before] {
                object["metadata"]
                    .as_object_mut()
                    .ok_or("contract_metadata")?
                    .remove("resourceVersion");
                object["metadata"]
                    .as_object_mut()
                    .ok_or("contract_metadata")?
                    .remove("managedFields");
            }
            require(now == before, "contract_automatic_object_changed")?;
        }
        let ns: Value = serde_json::from_str(
            &self.call(&["get", "namespace", &self.namespace, "-o", "json"], &[])?,
        )
        .map_err(|_| "contract_namespace_inventory")?;
        let (uid, rv) = identity(&ns)?;
        require(uid == self.namespace_uid, "contract_namespace_replaced")?;
        let path = format!("/api/v1/namespaces/{}", self.namespace);
        let options = json!({"apiVersion":"v1","kind":"DeleteOptions","preconditions":{"uid":uid,"resourceVersion":rv}});
        self.call(&["delete", "--raw", &path, "-f", "-"], &bytes(&options)?)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let raw = self.call(
                &[
                    "get",
                    "namespace",
                    &self.namespace,
                    "--ignore-not-found",
                    "-o",
                    "json",
                ],
                &[],
            )?;
            if raw.trim().is_empty() {
                return Ok(());
            }
            require(
                std::time::Instant::now() < deadline,
                "contract_namespace_cleanup_deadline",
            )?;
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
}

fn own_operation(value: &Value, operation: &str) -> bool {
    value["metadata"]["managedFields"]
        .as_array()
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|v| v["manager"] == "vzctl-kubernetes" && v["operation"] == operation)
        })
}
fn owns_spec_field(value: &Value, operation: &str, field: &str) -> bool {
    value["metadata"]["managedFields"]
        .as_array()
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["manager"] == "vzctl-kubernetes"
                    && entry["operation"] == operation
                    && entry["fieldsV1"]["f:spec"]
                        .get(format!("f:{field}"))
                        .is_some()
            })
        })
}

// Never persist guest diagnostics, even if an unexpected error escapes a future test helper.
fn failure_code(error: &str) -> &'static str {
    match error {
        "contract_api_inventory" => "contract_api_inventory",
        "contract_automatic_ca" => "contract_automatic_ca",
        "contract_automatic_deadline" => "contract_automatic_deadline",
        "contract_automatic_object_changed" => "contract_automatic_object_changed",
        "contract_automatic_serviceaccount" => "contract_automatic_serviceaccount",
        "contract_change_failed" => "contract_change_failed",
        "contract_change_outcome" => "contract_change_outcome",
        "contract_cleanup_foreign_object" => "contract_cleanup_foreign_object",
        "contract_cleanup_replacement" => "contract_cleanup_replacement",
        "contract_command_scope" => "contract_command_scope",
        "contract_create_inventory" => "contract_create_inventory",
        "contract_created_identity" => "contract_created_identity",
        "contract_duplicate_inventory" => "contract_duplicate_inventory",
        "contract_fixture_manager" => "contract_fixture_manager",
        "contract_forbidden_command" => "contract_forbidden_command",
        "contract_foreign_changed" => "contract_foreign_changed",
        "contract_foreign_conflict_missing" => "contract_foreign_conflict_missing",
        "contract_foreign_owner_changed" => "contract_foreign_owner_changed",
        "contract_foreign_target" => "contract_foreign_target",
        "contract_full_inventory" => "contract_full_inventory",
        "contract_hook" => "contract_hook",
        "contract_hook_missing" => "contract_hook_missing",
        "contract_idempotent_changed" => "contract_idempotent_changed",
        "contract_idempotent_failed" => "contract_idempotent_failed",
        "contract_inventory" => "contract_inventory",
        "contract_inventory_scope" => "contract_inventory_scope",
        "contract_json" => "contract_json",
        "contract_kubectl_failed" => "contract_kubectl_failed",
        "contract_ledger_commit" => "contract_ledger_commit",
        "contract_ledger_create" => "contract_ledger_create",
        "contract_ledger_sync" => "contract_ledger_sync",
        "contract_ledger_write" => "contract_ledger_write",
        "contract_legacy_apply_failed" => "contract_legacy_apply_failed",
        "contract_legacy_fixture" => "contract_legacy_fixture",
        "contract_legacy_outcome" => "contract_legacy_outcome",
        "contract_managed_fields" => "contract_managed_fields",
        "contract_metadata" => "contract_metadata",
        "contract_migration_manager_added" => "contract_migration_manager_added",
        "contract_namespace_cleanup_deadline" => "contract_namespace_cleanup_deadline",
        "contract_namespace_inventory" => "contract_namespace_inventory",
        "contract_namespace_not_exclusive" => "contract_namespace_not_exclusive",
        "contract_namespace_replaced" => "contract_namespace_replaced",
        "contract_new_failed" => "contract_new_failed",
        "contract_patch" => "contract_patch",
        "contract_patch_inventory_changed" => "contract_patch_inventory_changed",
        "contract_patch_manager" => "contract_patch_manager",
        "contract_payload" => "contract_payload",
        "contract_race_setup" => "contract_race_setup",
        "contract_rv_missing" => "contract_rv_missing",
        "contract_rv_retry_failed" => "contract_rv_retry_failed",
        "contract_service_apply" => "contract_service_apply",
        "contract_service_create" => "contract_service_create",
        "contract_service_defaults_changed" => "contract_service_defaults_changed",
        "contract_service_selector" => "contract_service_selector",
        "contract_uid_fixture" => "contract_uid_fixture",
        "contract_uid_guard_failed" => "contract_uid_guard_failed",
        "contract_uid_missing" => "contract_uid_missing",
        "contract_diagnostic_limit" => "contract_diagnostic_limit",
        "contract_service_defaults_missing" => "contract_service_defaults_missing",
        "contract_service_default_ownership" => "contract_service_default_ownership",
        "contract_service_protocol_default" => "contract_service_protocol_default",
        _ => "contract_unclassified_failure",
    }
}

fn uid_replacement_preserved(
    old_uid: &str,
    desired: &Value,
    replacement: &Value,
    after: &Value,
) -> bool {
    identity(replacement).is_ok_and(|(uid, _)| uid != old_uid)
        && manifests::matches_desired(desired, replacement)
        && replacement == after
}

fn run_cases(h: &mut Harness<'_>) -> Result<Vec<&'static str>, String> {
    let mut passed = Vec::new();
    h.phase = "automatic_inventory";
    h.capture_automatic()?;
    for (name, mixed) in [("legacy-update", false), ("mixed-update-apply", true)] {
        h.phase = if mixed {
            "mixed_update_apply"
        } else {
            "legacy_update"
        };
        let first = h.desired(name, "old")?;
        h.create(name, &first, "vzctl-kubernetes")?;
        if mixed {
            h.call(
                &[
                    "apply",
                    "--server-side",
                    "--field-manager=vzctl-kubernetes",
                    "-f",
                    "-",
                ],
                &bytes(&first)?,
            )?;
        }
        let old = h.get(name)?;
        require(
            own_operation(&old, "Update") && own_operation(&old, "Apply") == mixed,
            "contract_legacy_fixture",
        )?;
        let desired = h.desired(name, "new")?;
        let (result, _, _) = h.apply(name, &desired, "none")?;
        require(result.is_ok(), "contract_legacy_apply_failed")?;
        let after = h.get(name)?;
        require(
            identity(&after)?.0 == identity(&old)?.0
                && manifests::matches_desired(&desired, &after)
                && !own_operation(&after, "Update")
                && own_operation(&after, "Apply"),
            "contract_legacy_outcome",
        )?;
        passed.push(if mixed {
            "mixed_update_apply"
        } else {
            "legacy_update"
        });
    }
    h.phase = "create_and_change";
    let name = "new-and-change";
    let first = h.desired(name, "old")?;
    let (result, _, _) = h.apply(name, &first, "none")?;
    require(result.is_ok(), "contract_new_failed")?;
    let old = h.get(name)?;
    let desired = h.desired(name, "new")?;
    let (result, _, _) = h.apply(name, &desired, "none")?;
    require(result.is_ok(), "contract_change_failed")?;
    let after = h.get(name)?;
    require(
        identity(&old)?.0 == identity(&after)?.0 && manifests::matches_desired(&desired, &after),
        "contract_change_outcome",
    )?;
    passed.extend(["create", "changed_apply"]);
    let (result, _, _) = h.apply(name, &desired, "none")?;
    require(result.is_ok(), "contract_idempotent_failed")?;
    let repeated = h.get(name)?;
    require(
        super::managed_fields::same_values(&after, &repeated)
            && after["metadata"]["managedFields"] == repeated["metadata"]["managedFields"],
        "contract_idempotent_changed",
    )?;
    passed.push("idempotent");
    h.phase = "foreign_fields";
    let name = "foreign-conflict";
    let first = h.desired(name, "foreign")?;
    h.create(name, &first, "f47-foreign")?;
    let old = h.get(name)?;
    let desired = h.desired(name, "new")?;
    let (result, _, _) = h.apply(name, &desired, "none")?;
    require(
        expected_error(&result, "foreign_fields"),
        "contract_foreign_conflict_missing",
    )?;
    require(h.get(name)? == old, "contract_foreign_changed")?;
    passed.push("foreign_fields");
    h.phase = "foreign_owner";
    let name = "foreign-owner";
    let mut foreign = h.desired(name, "foreign")?;
    foreign["metadata"]["labels"][manifests::PROJECT] = json!("foreign-fixture");
    h.create(name, &foreign, "f47-foreign")?;
    let old = h.get(name)?;
    let desired = h.desired(name, "new")?;
    let (result, applies, _) = h.apply(name, &desired, "none")?;
    require(
        expected_error(&result, "foreign_owner") && applies == 0 && h.get(name)? == old,
        "contract_foreign_owner_changed",
    )?;
    passed.push("foreign_owner");
    h.phase = "service_default";
    let service = |port: u16| {
        manifests::owned(
            json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"service-default","namespace":h.namespace,"labels":{"vzctl.dev/f47-contract":h.namespace}},"spec":{"ports":[{"name":"http","port":port,"targetPort":8080}]}}),
            "controller-lab",
            "dev",
        )
    };
    let first = service(8080)?;
    let changed = service(8081)?;
    let (result, _, _) = h.apply("service-default", &first, "none")?;
    require(result.is_ok(), "contract_service_create")?;
    let before = h.get("service-default")?;
    let defaults = [
        "clusterIP",
        "clusterIPs",
        "ipFamilies",
        "ipFamilyPolicy",
        "sessionAffinity",
        "internalTrafficPolicy",
        "type",
    ];
    require(
        defaults
            .iter()
            .all(|field| !before["spec"][field].is_null())
            && before["spec"]["type"] == "ClusterIP"
            && before["spec"]["ports"][0]["protocol"] == "TCP",
        "contract_service_defaults_missing",
    )?;
    for desired in [&first, &changed] {
        let (result, _, _) = h.apply("service-default", desired, "none")?;
        require(result.is_ok(), "contract_service_apply")?;
        let after = h.get("service-default")?;
        require(
            identity(&after)?.0 == identity(&before)?.0
                && defaults
                    .iter()
                    .all(|key| after["spec"][key] == before["spec"][key])
                && manifests::matches_desired(desired, &after),
            "contract_service_defaults_changed",
        )?;
        require(
            defaults
                .iter()
                .filter(|field| owns_spec_field(&before, "Update", field))
                .all(|field| {
                    owns_spec_field(&after, "Update", field)
                        && !owns_spec_field(&after, "Apply", field)
                }),
            "contract_service_default_ownership",
        )?;
        require(
            after["spec"]["ports"][0]["protocol"] == "TCP",
            "contract_service_protocol_default",
        )?;
        require(
            after["spec"]["selector"].is_null(),
            "contract_service_selector",
        )?;
    }
    passed.push("service_defaults");
    for hook in ["uid", "rv"] {
        h.phase = if hook == "uid" {
            "uid_guard"
        } else {
            "rv_retry"
        };
        let name = if hook == "uid" { "uid-race" } else { "rv-race" };
        let first = h.desired(name, "old")?;
        let (result, _, _) = h.apply(name, &first, "none")?;
        require(result.is_ok(), "contract_race_setup")?;
        let original_uid = identity(&h.get(name)?)?.0;
        let desired = h.desired(name, "new")?;
        let (result, applies, fired) = h.apply(name, &desired, hook)?;
        require(fired, "contract_hook_missing")?;
        let after = h.get(name)?;
        if hook == "uid" {
            require(
                expected_error(&result, "uid")
                    && h.uid_replacement.as_ref().is_some_and(|replacement| {
                        uid_replacement_preserved(&original_uid, &desired, replacement, &after)
                    }),
                "contract_uid_guard_failed",
            )?;
        } else {
            require(
                result.is_ok()
                    && (2..=3).contains(&applies)
                    && manifests::matches_desired(&desired, &after)
                    && after["metadata"]["annotations"]["f47-race"] == "preserve",
                "contract_rv_retry_failed",
            )?;
        }
        passed.push(if hook == "uid" {
            "uid_guard"
        } else {
            "rv_retry"
        });
    }
    Ok(passed)
}

#[test]
#[ignore = "explicit private F47 plan and isolated live K3s namespace required"]
fn live_configmap_ssa_contract() {
    assert_eq!(
        std::env::var("VZCTL_F47_CONTRACT_EXECUTE").as_deref(),
        Ok("isolated-configmaps-only")
    );
    let path = std::env::var("VZCTL_F47_CONTRACT_PLAN").expect("private contract plan required");
    let expected = std::env::var("VZCTL_F47_CONTRACT_PLAN_SHA256").expect("plan SHA required");
    let path = Path::new(&path);
    let meta = fs::symlink_metadata(path).expect("plan metadata");
    assert!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && path.canonicalize().ok().as_deref() == Some(path)
            && meta.permissions().mode() & 0o077 == 0
            && meta.len() < 262144,
        "private bounded plan required"
    );
    let raw = fs::read(path).expect("plan read");
    assert_eq!(
        format!("{:x}", Sha256::digest(&raw)),
        expected,
        "plan changed"
    );
    let plan: Value = serde_json::from_slice(&raw).expect("plan JSON");
    assert_eq!(plan["schema"], "vzctl.f47-k3s-contract/v1");
    let namespace = plan["namespace"]
        .as_str()
        .expect("test namespace")
        .to_owned();
    assert!(
        namespace.starts_with("vzctl-f47-")
            && namespace.len() == 42
            && namespace[10..].bytes().all(|v| v.is_ascii_hexdigit()),
        "exclusive namespace required"
    );
    let config = Path::new(plan["config_path"].as_str().expect("config path"));
    assert_eq!(
        config,
        Path::new("/Users/frank/vzctl-labs/controller-lab/hypernetwork.config.yaml")
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(config).expect("config read"))
        ),
        plan["config_sha256"].as_str().expect("config SHA")
    );
    let environment = crate::config::validate_path(config)
        .unwrap_or_else(|_| panic!("contract configuration invalid"));
    assert_eq!(environment.spec.project, "controller-lab");
    let mut cluster = environment.spec.kubernetes.clusters["dev"].clone();
    assert_eq!(cluster.server, "server");
    assert_eq!(
        super::node_ip(&environment, &cluster, &cluster.server).expect("node IP"),
        "10.123.0.11"
    );
    cluster.default_namespace = namespace.clone();
    let state = crate::state_dir();
    assert_eq!(state.to_str(), plan["state_dir"].as_str());
    let socket = state.join("vz.sock");
    let guest = controller::Guest {
        environment: &environment,
        socket: &socket,
    };
    let namespace_object = json!({"apiVersion":"v1","kind":"Namespace","metadata":{"name":namespace,"labels":{"vzctl.dev/f47-contract":namespace}}});
    let ledger = path
        .parent()
        .expect("evidence directory")
        .join("owned-resources.json");
    assert!(!ledger.exists(), "prior contract resources require review");
    // Exclusive CREATE only; existing namespace is never adopted.
    let raw = guest
        .kubectl(
            &cluster,
            &["create", "-f", "-", "-o", "json"],
            &bytes(&namespace_object).expect("namespace encode"),
            20,
        )
        .unwrap_or_else(|_| panic!("exclusive namespace creation failed"));
    let created: Value = serde_json::from_str(&raw).expect("namespace result");
    let namespace_uid = identity(&created).expect("namespace UID").0;
    let mut h = Harness {
        guest,
        cluster,
        namespace,
        namespace_uid,
        created: BTreeMap::new(),
        automatic: BTreeMap::new(),
        phase: "starting",
        migrations: 0,
        ledger,
        diagnostics: Vec::new(),
        uid_replacement: None,
    };
    h.persist().expect("namespace UID ledger");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_cases(&mut h)));
    let cleanup = h.cleanup();
    let passed = match &outcome {
        Ok(Ok(cases)) => cases.clone(),
        _ => Vec::new(),
    };
    let code = match &outcome {
        Ok(Err(error)) => Some(failure_code(error)),
        Err(_) => Some("contract_panic"),
        Ok(Ok(_)) => cleanup.as_ref().err().map(|error| failure_code(error)),
    };
    let receipt = json!({"failure_code":code,"schema":"vzctl.f47-k3s-contract-result/v1","state":if outcome.as_ref().is_ok_and(|v|v.is_ok()) && cleanup.is_ok(){"passed"}else{"failed"},"cases":passed,"verified_migrations":h.migrations,"namespace":h.namespace,"namespace_uid":h.namespace_uid,"configmaps_cleaned":cleanup.is_ok(),"namespace_cleaned":cleanup.is_ok(),"failure_phase":h.phase});
    let report = path
        .parent()
        .expect("evidence directory")
        .join("contract-result.json");
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(report)
        .expect("private contract report");
    file.write_all(&bytes(&receipt).expect("report encode"))
        .expect("report write");
    file.sync_all().expect("report sync");
    assert!(
        cleanup.is_ok(),
        "exact UID cleanup failed; inspect private contract evidence"
    );
    assert!(
        matches!(outcome, Ok(Ok(_))),
        "F47 product contract failed; see fixed allowlist report"
    );
}

#[test]
fn contract_error_classifier_rejects_transport_and_harness_failures() {
    assert_eq!(
        failure_code("contract_service_default_ownership"),
        "contract_service_default_ownership"
    );
    assert_eq!(
        failure_code("raw diagnostic with private payload"),
        "contract_unclassified_failure"
    );
    for case in ["foreign_fields", "foreign_owner", "uid"] {
        for error in [
            "connection timed out",
            "contract_foreign_target",
            "contract_payload",
            "K3s node server: permission denied",
        ] {
            assert!(!expected_error(&Err(error.into()), case));
        }
        assert!(!expected_error(&Ok(()), case));
    }
    assert!(expected_error(
        &Err("Apply failed with conflict with \"f47-foreign\": .data.value".into()),
        "foreign_fields"
    ));
    assert!(expected_error(&Err("ownership conflict: ConfigMap foreign-owner already exists outside this cluster controller".into()),"foreign_owner"));
    assert!(expected_error(
        &Err("ownership conflict: Kubernetes resource was replaced during apply".into()),
        "uid"
    ));
}

#[test]
fn synthetic_diagnostics_are_bounded_and_refuse_unrelated_payloads() {
    let fixed = "field ownership bridge did not establish complete Apply ownership";
    assert_eq!(
        diagnostic_error(fixed, "test", "vzctl-f47-test")["message"],
        fixed
    );
    for error in [
        "token=private",
        "password hidden",
        "unrelated error",
        &"x".repeat(8193),
    ] {
        assert!(diagnostic_error(error, "test", "vzctl-f47-test")["message"].is_null());
    }
    let mut value = json!({"kind":"ConfigMap","metadata":{"name":"test","namespace":"vzctl-f47-test",
        "uid":"u","resourceVersion":"1","managedFields":[]},"data":{"value":"old"}});
    assert_eq!(
        diagnostic_object(&value, "test", "vzctl-f47-test")["uid"],
        "u"
    );
    value["data"]["value"] = json!("private");
    assert_eq!(
        diagnostic_object(&value, "test", "vzctl-f47-test")["class"],
        "unexpected_object_redacted"
    );
    value["data"]["value"] = json!("new");
    value["metadata"]["namespace"] = json!("app");
    assert_eq!(
        diagnostic_object(&value, "test", "vzctl-f47-test")["class"],
        "unexpected_object_redacted"
    );
}

#[test]
fn uid_fixture_requires_full_desired_intent_and_unchanged_replacement() {
    let desired = manifests::owned(
        json!({"apiVersion":"v1","kind":"ConfigMap",
        "metadata":{"name":"uid-race","namespace":"vzctl-f47-test"},"data":{"value":"new"}}),
        "controller-lab",
        "dev",
    )
    .unwrap();
    let mut replacement = desired.clone();
    replacement["metadata"]["uid"] = json!("new-uid");
    replacement["metadata"]["resourceVersion"] = json!("2");
    replacement["metadata"]["managedFields"] =
        json!([{"manager":"f47-foreign","operation":"Update"}]);
    assert!(uid_replacement_preserved(
        "old-uid",
        &desired,
        &replacement,
        &replacement
    ));
    assert!(!uid_replacement_preserved(
        "new-uid",
        &desired,
        &replacement,
        &replacement
    ));
    let mut wrong_value = replacement.clone();
    wrong_value["data"]["value"] = json!("replacement");
    assert!(!uid_replacement_preserved(
        "old-uid",
        &desired,
        &wrong_value,
        &wrong_value
    ));
    let mut wrong_hash = replacement.clone();
    wrong_hash["metadata"]["annotations"][manifests::DIGEST] = json!("old");
    assert!(!uid_replacement_preserved(
        "old-uid",
        &desired,
        &wrong_hash,
        &wrong_hash
    ));
    let mut changed = replacement.clone();
    changed["metadata"]["managedFields"] = json!([]);
    assert!(!uid_replacement_preserved(
        "old-uid",
        &desired,
        &replacement,
        &changed
    ));
    changed = replacement.clone();
    changed["metadata"]["uid"] = json!("third-uid");
    assert!(!uid_replacement_preserved(
        "old-uid",
        &desired,
        &replacement,
        &changed
    ));
    assert!(!expected_error(
        &Err("conflict with f47-foreign on .data.value".into()),
        "uid"
    ));
}
