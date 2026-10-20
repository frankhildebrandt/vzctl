use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;

pub(super) const MANAGED: &str = "vzctl.dev/managed-by";
pub(super) const PROJECT: &str = "vzctl.dev/project";
pub(super) const CLUSTER: &str = "vzctl.dev/cluster";
pub(super) const DIGEST: &str = "vzctl.dev/manifest-sha256";

pub(super) fn read(path: &Path) -> Result<Vec<Value>, String> {
    let file =
        fs::File::open(path).map_err(|e| format!("read manifest {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("manifest file exceeds 1 MiB".into());
    }
    let mut objects = Vec::new();
    for document in serde_yaml::Deserializer::from_slice(&bytes) {
        if objects.len() >= 128 {
            return Err("manifest exceeds 128 resources".into());
        }
        let value =
            Value::deserialize(document).map_err(|e| format!("invalid Kubernetes YAML: {e}"))?;
        if value.is_null() {
            continue;
        }
        let values = if value["kind"] == "List" {
            value["items"]
                .as_array()
                .ok_or("List requires items array")?
                .clone()
        } else {
            vec![value]
        };
        for value in values {
            if objects.len() >= 128 {
                return Err("manifest exceeds 128 resources".into());
            }
            for pointer in ["/apiVersion", "/kind", "/metadata/name"] {
                if value
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(format!("resource requires nonempty {pointer}"));
                }
            }
            if value["metadata"]["generateName"].is_string() {
                return Err(
                    "generateName is unsupported for declarative resources; use metadata.name"
                        .into(),
                );
            }
            if !value["metadata"]["labels"].is_null() && !value["metadata"]["labels"].is_object() {
                return Err("metadata.labels must be an object".into());
            }
            if value["metadata"]["labels"]
                .as_object()
                .is_some_and(|labels| labels.values().any(|v| !v.is_string()))
            {
                return Err("metadata.labels values must be strings".into());
            }
            if !value["metadata"]["annotations"].is_null()
                && !value["metadata"]["annotations"].is_object()
            {
                return Err("metadata.annotations must be an object".into());
            }
            if value["metadata"]["annotations"]
                .as_object()
                .is_some_and(|annotations| annotations.values().any(|value| !value.is_string()))
            {
                return Err("metadata.annotations values must be strings".into());
            }
            if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 240 * 1024 {
                return Err("individual resource exceeds 240 KiB guest-transfer limit".into());
            }
            objects.push(value);
        }
    }
    if objects.is_empty() {
        return Err("manifest contains no resources".into());
    }
    Ok(objects)
}

pub(super) fn owned(mut value: Value, project: &str, cluster: &str) -> Result<Value, String> {
    if value["metadata"]["labels"].is_null() {
        value["metadata"]["labels"] = json!({});
    }
    for (key, wanted) in [(MANAGED, "vzctl"), (PROJECT, project), (CLUSTER, cluster)] {
        if let Some(existing) = value["metadata"]["labels"][key].as_str() {
            if existing != wanted {
                return Err(format!("manifest has conflicting ownership label {key}"));
            }
        }
        value["metadata"]["labels"][key] = json!(wanted);
    }
    if value["metadata"]["annotations"].is_null() {
        value["metadata"]["annotations"] = json!({});
    }
    value["metadata"]["annotations"]
        .as_object_mut()
        .ok_or("metadata.annotations must be an object")?
        .remove(DIGEST);
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).map_err(|e| e.to_string())?)
    );
    value["metadata"]["annotations"][DIGEST] = json!(digest);
    Ok(value)
}

/// API defaults may add object fields, including inside arrays. Array membership
/// and order stay exact; every declared field must still match.
pub(super) fn matches_desired(desired: &Value, actual: &Value) -> bool {
    match desired {
        Value::Object(values) => values.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|observed| matches_desired(value, observed))
        }),
        Value::Array(values) => actual.as_array().is_some_and(|observed| {
            values.len() == observed.len()
                && values
                    .iter()
                    .zip(observed)
                    .all(|(value, actual)| matches_desired(value, actual))
        }),
        _ => desired == actual,
    }
}

pub(super) fn check_existing(value: &Value, project: &str, cluster: &str) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }
    if value["kind"] == "List" {
        for item in value["items"]
            .as_array()
            .ok_or("invalid Kubernetes inventory")?
        {
            check_existing(item, project, cluster)?;
        }
        return Ok(());
    }
    let labels = &value["metadata"]["labels"];
    if labels[MANAGED] != "vzctl" || labels[PROJECT] != project || labels[CLUSTER] != cluster {
        return Err(format!(
            "ownership conflict: {} {} already exists outside this cluster controller",
            value["kind"].as_str().unwrap_or("?"),
            value["metadata"]["name"].as_str().unwrap_or("?")
        ));
    }
    Ok(())
}
