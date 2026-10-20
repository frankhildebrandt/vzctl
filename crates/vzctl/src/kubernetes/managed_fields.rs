//! Retire only our imperative Create ownership before subsequent SSA changes.
//! Kubernetes keys a manager by operation too: Update != Apply, even with the
//! same name. Never force SSA or clear another manager's ownership.
use serde_json::{json, Map, Value};

const MANAGER: &str = "vzctl-kubernetes";

pub(super) struct Migration {
    pub bridge: Option<Value>,
    pub patch: Value,
    pub retained: Value,
}

fn ours(entry: &Value, operation: &str, version: &Value) -> bool {
    entry["manager"] == MANAGER
        && entry["operation"] == operation
        && entry["apiVersion"] == *version
        && entry.get("subresource").is_none_or(|v| v == "")
}

fn merge_fields(target: &mut Value, source: &Value) -> Result<(), String> {
    let source = source.as_object().ok_or("invalid managed field set")?;
    let target = target.as_object_mut().ok_or("invalid managed field set")?;
    for (key, value) in source {
        if let Some(current) = target.get_mut(key) {
            if current.as_object().map(Map::is_empty) != value.as_object().map(Map::is_empty) {
                return Err("incompatible atomic/structured managed field ownership".into());
            }
            merge_fields(current, value)?;
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn current_field<'a>(current: &'a Value, key: &str) -> Option<&'a Value> {
    if key == "." {
        return Some(current);
    }
    if let Some(name) = key.strip_prefix("f:") {
        return current.as_object()?.get(name);
    }
    let items = current.as_array()?;
    let mut matches = Vec::new();
    if let Some(raw) = key.strip_prefix("k:") {
        let selector: Value = serde_json::from_str(raw).ok()?;
        let keys = selector.as_object().filter(|keys| !keys.is_empty())?;
        matches.extend(
            items
                .iter()
                .filter(|item| keys.iter().all(|(k, v)| item.get(k) == Some(v))),
        );
    } else if let Some(raw) = key.strip_prefix("v:") {
        let value: Value = serde_json::from_str(raw).ok()?;
        matches.extend(items.iter().filter(|item| **item == value));
    }
    (matches.len() == 1).then(|| matches[0])
}

fn covers(cover: &Value, fields: &Value, current: &Value, list_element: bool) -> bool {
    let (Some(cover), Some(fields)) = (cover.as_object(), fields.as_object()) else {
        return false;
    };
    if fields.is_empty() {
        return cover.is_empty();
    }
    // SSA may omit a granular map/list CONTAINER marker while recording every
    // concrete child. A dot-only/atomic node or keyed/set ELEMENT membership
    // remains strict, even when that element happens to be an object.
    let granular_container = !list_element
        && ((current.is_object() && fields.keys().any(|key| key.starts_with("f:")))
            || (current.is_array()
                && fields
                    .keys()
                    .any(|key| key.starts_with("k:") || key.starts_with("v:"))));
    fields.iter().all(|(key, value)| {
        if key == "." && value == &json!({}) && granular_container {
            return true;
        }
        cover.get(key).is_some_and(|other| {
            current_field(current, key).is_some_and(|child| {
                covers(
                    other,
                    value,
                    child,
                    key.starts_with("k:") || key.starts_with("v:"),
                )
            })
        })
    })
}

/// Extract current values for precisely the recorded field set. Lists use
/// Kubernetes' keyed/set encodings. Unknown encodings fail before any mutation.
fn project(current: &Value, fields: &Value, depth: usize) -> Result<Value, String> {
    if depth > 64 {
        return Err("managed field projection exceeds depth limit".into());
    }
    let fields = fields.as_object().ok_or("invalid managed field set")?;
    if fields.is_empty() {
        return Ok(current.clone());
    }
    if let Some(object) = current.as_object() {
        let mut result = Map::new();
        for (key, child) in fields {
            if key == "." && child == &json!({}) {
                continue;
            }
            let key = key
                .strip_prefix("f:")
                .ok_or("unsupported managed map field")?;
            let value = object
                .get(key)
                .ok_or("managed field missing from current object")?;
            result.insert(key.into(), project(value, child, depth + 1)?);
        }
        return Ok(Value::Object(result));
    }
    if let Some(items) = current.as_array() {
        let mut result = Vec::new();
        let mut selected = std::collections::BTreeSet::new();
        // Preserve server order; do not sort associative/ordered list contents.
        for (index, item) in items.iter().enumerate() {
            for (key, child) in fields {
                if key == "." && child == &json!({}) {
                    continue;
                }
                let (matches, selector) = if let Some(raw) = key.strip_prefix("k:") {
                    let selector: Value =
                        serde_json::from_str(raw).map_err(|_| "invalid managed list key")?;
                    let values = selector
                        .as_object()
                        .filter(|v| !v.is_empty())
                        .ok_or("invalid managed list key")?;
                    if values
                        .values()
                        .any(|v| v.is_object() || v.is_array() || v.is_null())
                    {
                        return Err("unsupported managed list key value".into());
                    }
                    (
                        values.iter().all(|(k, v)| item.get(k) == Some(v)),
                        Some(selector),
                    )
                } else if let Some(raw) = key.strip_prefix("v:") {
                    let value: Value =
                        serde_json::from_str(raw).map_err(|_| "invalid managed set value")?;
                    (item == &value, None)
                } else {
                    return Err("unsupported managed list field encoding".into());
                };
                if matches {
                    if !selected.insert(key.clone()) {
                        return Err("ambiguous managed list element".into());
                    }
                    let mut value = project(item, child, depth + 1)?;
                    if let Some(selector) = selector {
                        let object = value
                            .as_object_mut()
                            .ok_or("managed keyed list is not a map")?;
                        for (key, value) in selector.as_object().unwrap() {
                            object.insert(key.clone(), value.clone());
                        }
                    }
                    result.push((index, value));
                }
            }
        }
        if selected.len() != fields.keys().filter(|key| *key != ".").count() {
            return Err("managed list element missing from current object".into());
        }
        if result.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("overlapping managed list element selectors".into());
        }
        return Ok(Value::Array(
            result.into_iter().map(|(_, value)| value).collect(),
        ));
    }
    Err("managed field structure does not match current object".into())
}

fn intended(fields: &Value, desired: &Value, depth: usize) -> Result<Value, String> {
    if depth > 64 {
        return Err("managed intent exceeds depth limit".into());
    }
    let fields = fields.as_object().ok_or("invalid managed intent fields")?;
    let mut result = Map::new();
    let mut selected_keys = std::collections::BTreeSet::new();
    for (key, child) in fields {
        let selected = if key == "." {
            Some(desired)
        } else if let Some(name) = key.strip_prefix("f:") {
            desired.as_object().and_then(|object| object.get(name))
        } else if let Some(raw) = key.strip_prefix("k:") {
            let selector: Value =
                serde_json::from_str(raw).map_err(|_| "invalid intent list key")?;
            let selector = selector
                .as_object()
                .filter(|s| !s.is_empty())
                .ok_or("invalid intent list key")?;
            let matches: Vec<_> = desired
                .as_array()
                .ok_or("invalid desired list")?
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    // Some associative keys are defaulted by the API (Service
                    // port protocol is one example). Match the explicit key
                    // components, requiring at least one and a unique result.
                    // The bridge still uses the complete CURRENT selector.
                    selector.keys().any(|name| item.get(name).is_some())
                        && selector.iter().all(|(name, value)| {
                            item.get(name).is_none_or(|actual| actual == value)
                        })
                })
                .collect();
            if matches.len() > 1 {
                return Err("ambiguous desired list key".into());
            }
            if let Some((index, item)) = matches.first() {
                if !selected_keys.insert(*index) {
                    return Err("ambiguous desired list key".into());
                }
                Some(*item)
            } else {
                None
            }
        } else if let Some(raw) = key.strip_prefix("v:") {
            let value: Value = serde_json::from_str(raw).map_err(|_| "invalid intent set value")?;
            desired
                .as_array()
                .ok_or("invalid desired set")?
                .iter()
                .find(|item| *item == &value)
        } else {
            return Err("unsupported managed intent encoding".into());
        };
        if let Some(selected) = selected {
            let child_object = child.as_object().ok_or("invalid managed intent child")?;
            let mut value = if child_object.is_empty() {
                json!({})
            } else {
                intended(child, selected, depth + 1)?
            };
            // Associative key fields are the identity of this same element,
            // including keys defaulted by the API. The bridge must carry them;
            // retire their matching Update ownership along with that identity.
            if let Some(raw) = key.strip_prefix("k:") {
                let selector: Value =
                    serde_json::from_str(raw).map_err(|_| "invalid intent list key")?;
                for name in selector
                    .as_object()
                    .ok_or("invalid intent list key")?
                    .keys()
                {
                    let field = format!("f:{name}");
                    if let Some(key_field) = child_object.get(&field) {
                        value
                            .as_object_mut()
                            .ok_or("invalid intent list fields")?
                            .insert(field, key_field.clone());
                    }
                }
            }
            if child_object.is_empty() || value.as_object().is_some_and(|v| !v.is_empty()) {
                result.insert(key.clone(), value);
            }
        }
    }
    Ok(Value::Object(result))
}

fn field_intersection(left: &Value, right: &Value) -> Result<Value, String> {
    let left = left.as_object().ok_or("invalid field intersection")?;
    let right = right.as_object().ok_or("invalid field intersection")?;
    let mut result = Map::new();
    for (key, child) in left {
        if let Some(other) = right.get(key) {
            let empty = child
                .as_object()
                .ok_or("invalid field intersection child")?
                .is_empty();
            if empty
                != other
                    .as_object()
                    .ok_or("invalid field intersection child")?
                    .is_empty()
            {
                return Err("incompatible atomic/structured field intersection".into());
            }
            let value = field_intersection(child, other)?;
            if empty || value.as_object().is_some_and(|v| !v.is_empty()) {
                result.insert(key.clone(), value);
            }
        }
    }
    Ok(Value::Object(result))
}

fn field_difference(fields: &Value, removed: &Value) -> Result<Value, String> {
    let mut result = fields
        .as_object()
        .ok_or("invalid field difference")?
        .clone();
    for (key, child) in removed.as_object().ok_or("invalid field difference")? {
        if let Some(current) = result.get(key) {
            let empty = child
                .as_object()
                .ok_or("invalid field difference child")?
                .is_empty();
            if empty
                != current
                    .as_object()
                    .ok_or("invalid field difference child")?
                    .is_empty()
            {
                return Err("incompatible atomic/structured field difference".into());
            }
            let value = field_difference(current, child)?;
            if value.as_object().is_some_and(Map::is_empty) {
                result.remove(key);
            } else {
                result.insert(key.clone(), value);
            }
        }
    }
    Ok(Value::Object(result))
}

pub(super) fn migration(current: &Value, desired: &Value) -> Result<Option<Migration>, String> {
    let Some(raw) = current["metadata"].get("managedFields") else {
        return Ok(None);
    };
    let entries = raw.as_array().ok_or("invalid managedFields inventory")?;
    if entries.len() > 128
        || serde_json::to_vec(raw)
            .map_err(|_| "managedFields serialization")?
            .len()
            > 128 * 1024
    {
        return Err("managedFields inventory exceeds migration limit".into());
    }
    let version = &current["apiVersion"];
    let updates: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| ours(entry, "Update", version))
        .collect();
    if updates.is_empty() {
        return Ok(None);
    }
    let mut update_fields = json!({});
    let mut apply_fields = json!({});
    for entry in entries {
        if ours(entry, "Update", version) || ours(entry, "Apply", version) {
            if entry["fieldsType"] != "FieldsV1" || !entry["fieldsV1"].is_object() {
                return Err("unsupported own managed field format".into());
            }
            merge_fields(
                if entry["operation"] == "Update" {
                    &mut update_fields
                } else {
                    &mut apply_fields
                },
                &entry["fieldsV1"],
            )?;
        }
    }
    // Transfer only declared intent and previously applied fields. Server-owned
    // defaults from Create (e.g. immutable Service allocation fields) stay in
    // Update; claiming then omitting them in SSA could otherwise prune them.
    let mut transferred = intended(&update_fields, desired, 0)?;
    merge_fields(
        &mut transferred,
        &field_intersection(&update_fields, &apply_fields)?,
    )?;
    if transferred.as_object().is_some_and(Map::is_empty) {
        return Ok(None);
    }
    let uid = current["metadata"]["uid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("field migration requires UID")?;
    let rv = current["metadata"]["resourceVersion"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("field migration requires resourceVersion")?;
    let bridge = if covers(&apply_fields, &transferred, current, false) {
        None
    } else {
        // Establish Apply with unchanged current values, including existing Apply
        // fields so this bridge cannot prune previous declarative ownership.
        merge_fields(&mut apply_fields, &transferred)?;
        let mut value = project(current, &apply_fields, 0)?;
        value["apiVersion"] = current["apiVersion"].clone();
        value["kind"] = current["kind"].clone();
        value["metadata"]["name"] = current["metadata"]["name"].clone();
        if let Some(namespace) = current["metadata"].get("namespace") {
            value["metadata"]["namespace"] = namespace.clone();
        }
        value["metadata"]["uid"] = json!(uid);
        value["metadata"]["resourceVersion"] = json!(rv);
        Some(value)
    };
    let mut patch = vec![
        json!({"op":"test","path":"/metadata/uid","value":uid}),
        json!({"op":"test","path":"/metadata/resourceVersion","value":rv}),
        json!({"op":"test","path":"/metadata/managedFields","value":raw}),
    ];
    let mut retained = entries.clone();
    for (index, entry) in updates.iter().rev() {
        let remaining = field_difference(
            &entry["fieldsV1"],
            &field_intersection(&entry["fieldsV1"], &transferred)?,
        )?;
        if remaining.as_object().is_some_and(Map::is_empty) {
            patch.push(json!({"op":"remove","path":format!("/metadata/managedFields/{index}")}));
            retained.remove(*index);
        } else if remaining != entry["fieldsV1"] {
            patch.push(json!({"op":"replace","path":format!("/metadata/managedFields/{index}/fieldsV1"),"value":remaining}));
            retained[*index]["fieldsV1"] = remaining;
        }
    }
    // Never issue []/[{}] or a blanket reset. The bridge must first establish
    // ownership if the only entry is the imperative Create manager.
    Ok(Some(Migration {
        bridge,
        patch: Value::Array(patch),
        retained: json!(retained),
    }))
}

pub(super) fn same_values(before: &Value, after: &Value) -> bool {
    fn values(value: &Value) -> Value {
        let mut result = value.clone();
        if let Some(meta) = result["metadata"].as_object_mut() {
            meta.remove("resourceVersion");
            meta.remove("managedFields");
        }
        result
    }
    values(before) == values(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_granular_map_markers_do_not_repeat_an_established_bridge() {
        // Actual K3s v1.33.5+k3s1 FieldsV1 from the isolated F47 contract:
        // identical-value SSA keeps all concrete fields but omits map markers.
        let update_fields = json!({"f:data":{".":{},"f:value":{}},"f:metadata":{
            "f:annotations":{".":{},"f:vzctl.dev/manifest-sha256":{}},
            "f:labels":{".":{},"f:vzctl.dev/cluster":{},"f:vzctl.dev/f47-contract":{},
                "f:vzctl.dev/managed-by":{},"f:vzctl.dev/project":{}}}});
        let apply_fields = json!({"f:data":{"f:value":{}},"f:metadata":{
            "f:annotations":{"f:vzctl.dev/manifest-sha256":{}},
            "f:labels":{"f:vzctl.dev/cluster":{},"f:vzctl.dev/f47-contract":{},
                "f:vzctl.dev/managed-by":{},"f:vzctl.dev/project":{}}}});
        let update = json!({"manager":MANAGER,"operation":"Update","apiVersion":"v1","fieldsType":"FieldsV1","fieldsV1":update_fields});
        let apply = json!({"manager":MANAGER,"operation":"Apply","apiVersion":"v1","fieldsType":"FieldsV1","fieldsV1":apply_fields});
        let current = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"legacy-only","namespace":"f47-test","uid":"u","resourceVersion":"2",
            "annotations":{"vzctl.dev/manifest-sha256":"old"},"labels":{"vzctl.dev/cluster":"dev","vzctl.dev/f47-contract":"f47-test","vzctl.dev/managed-by":"vzctl","vzctl.dev/project":"controller-lab"},
            "managedFields":[apply.clone(),update]},"data":{"value":"old","foreign":"keep"}});
        let mut desired = current.clone();
        desired["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("managedFields");
        desired["data"] = json!({"value":"new"});
        desired["metadata"]["annotations"]["vzctl.dev/manifest-sha256"] = json!("new");
        let result = migration(&current, &desired).unwrap().unwrap();
        assert!(
            result.bridge.is_none(),
            "real SSA field coverage must finish the bridge"
        );
        assert_eq!(result.retained, json!([apply]));
        assert_eq!(
            result.patch[3],
            json!({"op":"remove","path":"/metadata/managedFields/1"})
        );
        // Missing concrete ownership must still need a bridge; foreign values
        // cannot be claimed just because their enclosing map has a marker.
        let mut missing = current.clone();
        missing["metadata"]["managedFields"][0]["fieldsV1"]
            .as_object_mut()
            .unwrap()
            .remove("f:data");
        let bridge = migration(&missing, &desired)
            .unwrap()
            .unwrap()
            .bridge
            .unwrap();
        assert_eq!(bridge["data"], json!({"value":"old"}));
        assert!(bridge["data"].get("foreign").is_none());
    }

    #[test]
    fn map_marker_normalization_never_relaxes_missing_leaves_or_list_membership() {
        assert!(covers(
            &json!({"f:x":{}}),
            &json!({".":{},"f:x":{}}),
            &json!({"x":"v"}),
            false
        ));
        assert!(!covers(
            &json!({}),
            &json!({".":{},"f:x":{}}),
            &json!({"x":"v"}),
            false
        ));
        assert!(!covers(&json!({}), &json!({".":{}}), &json!({}), false));
        assert!(!covers(
            &json!({"f:x":{}}),
            &json!({}),
            &json!({"x":"v"}),
            false
        ));
        assert!(!covers(
            &json!({"f:x":{}}),
            &json!({".":{},"f:x":{}}),
            &json!(["v"]),
            false
        ));
        let current = json!({"items":[{"name":"app","image":"old"}]});
        let fields = json!({"f:items":{"k:{\"name\":\"app\"}":{".":{},"f:name":{},"f:image":{}}}});
        let cover = json!({"f:items":{"k:{\"name\":\"app\"}":{"f:name":{},"f:image":{}}}});
        assert!(!covers(&cover, &fields, &current, false));
        assert!(covers(&fields, &fields, &current, false));
        // Set element identity is strict even when the element itself is a map.
        let current = json!([{"name":"app"}]);
        let fields = json!({"v:{\"name\":\"app\"}":{".":{},"f:name":{}}});
        let cover = json!({"v:{\"name\":\"app\"}":{"f:name":{}}});
        assert!(!covers(&cover, &fields, &current, false));
        assert!(covers(&fields, &fields, &current, false));
    }

    #[test]
    fn api_service_list_container_marker_preserves_element_identity_and_defaults() {
        // Measured Service fields after identical-value SSA on real K3s: only
        // the list CONTAINER dot is omitted, not the keyed element membership.
        let element = json!({".":{},"f:name":{},"f:port":{},"f:protocol":{},"f:targetPort":{}});
        let key = "k:{\"port\":8080,\"protocol\":\"TCP\"}";
        let update_fields = json!({"f:spec":{"f:internalTrafficPolicy":{},"f:sessionAffinity":{},"f:type":{},
            "f:ports":{".":{},key:element}}});
        let apply_fields = json!({"f:spec":{"f:ports":{key:element}}});
        let entry = |operation, fields| json!({"manager":MANAGER,"operation":operation,"apiVersion":"v1","fieldsType":"FieldsV1","fieldsV1":fields});
        let current = json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"service-default","namespace":"f47-test","uid":"u","resourceVersion":"2",
            "managedFields":[entry("Apply",apply_fields.clone()),entry("Update",update_fields.clone())]},
            "spec":{"internalTrafficPolicy":"Cluster","sessionAffinity":"None","type":"ClusterIP","clusterIP":"10.43.0.12",
                "ports":[{"name":"http","port":8080,"protocol":"TCP","targetPort":8080}]}});
        let desired = json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"service-default","namespace":"f47-test"},
            "spec":{"ports":[{"name":"http","port":8080,"targetPort":9090}]}});
        let result = migration(&current, &desired).unwrap().unwrap();
        assert!(
            result.bridge.is_none(),
            "list container normalization must complete the bridge"
        );
        assert_eq!(result.retained[0], current["metadata"]["managedFields"][0]);
        assert_eq!(
            result.retained[1]["fieldsV1"],
            json!({"f:spec":{"f:internalTrafficPolicy":{},"f:sessionAffinity":{},"f:type":{}}})
        );
        let mut missing = apply_fields.clone();
        missing["f:spec"]["f:ports"][key]
            .as_object_mut()
            .unwrap()
            .remove(".");
        assert!(!covers(
            &missing["f:spec"]["f:ports"],
            &update_fields["f:spec"]["f:ports"],
            &current["spec"]["ports"],
            false
        ));
        assert!(!covers(&json!({}), &json!({".":{}}), &json!([]), false));
        assert!(!covers(
            &json!({}),
            &update_fields["f:spec"]["f:ports"],
            &current["spec"]["ports"],
            false
        ));
        // Set containers follow the same rule while each v: identity stays strict.
        assert!(covers(
            &json!({"v:\"x\"":{}}),
            &json!({".":{},"v:\"x\"":{}}),
            &json!(["x"]),
            false
        ));
        assert!(!covers(
            &json!({}),
            &json!({".":{},"v:\"x\"":{}}),
            &json!(["x"]),
            false
        ));
    }

    #[test]
    fn service_defaults_stay_in_update_while_declared_fields_move_to_apply() {
        let fields = json!({"f:spec":{"f:clusterIP":{},"f:clusterIPs":{},"f:ipFamilies":{},
            "f:sessionAffinity":{},"f:type":{},"f:ports":{"k:{\"port\":80,\"protocol\":\"TCP\"}":{
                ".":{},"f:port":{},"f:protocol":{},"f:targetPort":{}}}}});
        let own = json!({"manager":MANAGER,"operation":"Update","apiVersion":"v1",
            "fieldsType":"FieldsV1","fieldsV1":fields});
        let current = json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"proof",
            "namespace":"app","uid":"service-uid","resourceVersion":"1","managedFields":[own]},
            "spec":{"clusterIP":"10.43.0.10","clusterIPs":["10.43.0.10"],"ipFamilies":["IPv4"],
                "sessionAffinity":"None","type":"ClusterIP","ports":[{"port":80,"protocol":"TCP","targetPort":8080}]}});
        let desired = json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"proof","namespace":"app"},
            "spec":{"ports":[{"port":80,"targetPort":8081}]}});
        let result = migration(&current, &desired).unwrap().unwrap();
        let bridge = result.bridge.unwrap();
        assert_eq!(
            bridge["spec"],
            json!({"ports":[{"port":80,"protocol":"TCP","targetPort":8080}]})
        );
        let expected = json!({"f:spec":{"f:clusterIP":{},"f:clusterIPs":{},"f:ipFamilies":{},
            "f:sessionAffinity":{},"f:type":{}}});
        assert_eq!(result.retained[0]["fieldsV1"], expected);
        assert_eq!(
            result.patch[3],
            json!({"op":"replace","path":"/metadata/managedFields/0/fieldsV1","value":expected})
        );
        let mut migrated = current.clone();
        migrated["metadata"]["managedFields"] = result.retained;
        assert!(migration(&migrated, &desired).unwrap().is_none());
        // A future explicit declaration can transfer a former default safely.
        let mut next = desired;
        next["spec"]["type"] = json!("NodePort");
        let future = migration(&migrated, &next).unwrap().unwrap();
        assert_eq!(future.bridge.unwrap()["spec"]["type"], "ClusterIP");
        assert!(future.retained[0]["fieldsV1"]["f:spec"]
            .get("f:type")
            .is_none());
        assert!(future.retained[0]["fieldsV1"]["f:spec"]
            .get("f:clusterIP")
            .is_some());
    }

    #[test]
    fn defaulted_list_keys_must_identify_one_current_and_desired_element() {
        let fields = json!({"k:{\"port\":80,\"protocol\":\"TCP\"}":{"f:port":{},"f:protocol":{}},
            "k:{\"port\":80,\"protocol\":\"UDP\"}":{"f:port":{},"f:protocol":{}}});
        assert!(intended(&fields, &json!([{"port":80}]), 0).is_err());
        assert!(intended(
            &fields,
            &json!([{"port":80,"protocol":"TCP"},{"port":80,"protocol":"TCP"}]),
            0
        )
        .is_err());
        assert_eq!(
            intended(
                &fields,
                &json!([{"port":80,"protocol":"TCP"},{"port":80,"protocol":"UDP"}]),
                0
            )
            .unwrap(),
            fields
        );
    }

    #[test]
    fn formerly_applied_fields_are_released_from_update_for_normal_ssa_removal() {
        let update = json!({"manager":MANAGER,"operation":"Update","apiVersion":"v1",
            "fieldsType":"FieldsV1","fieldsV1":{"f:data":{"f:removed":{},"f:serverDefault":{}}}});
        let apply = json!({"manager":MANAGER,"operation":"Apply","apiVersion":"v1",
            "fieldsType":"FieldsV1","fieldsV1":{"f:data":{"f:removed":{}}}});
        let current = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"x","uid":"u",
            "resourceVersion":"1","managedFields":[update,apply]},"data":{"removed":"old","serverDefault":"keep"}});
        let desired =
            json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"x"},"data":{}});
        let result = migration(&current, &desired).unwrap().unwrap();
        assert!(result.bridge.is_none());
        assert_eq!(
            result.retained[0]["fieldsV1"],
            json!({"f:data":{"f:serverDefault":{}}})
        );
        assert_eq!(result.retained[1], current["metadata"]["managedFields"][1]);
    }

    #[test]
    fn bridge_projects_only_own_fields_and_exact_list_entries() {
        let actual = json!({
            "spec":{"containers":[
                {"name":"sidecar","image":"foreign","env":[{"name":"FOREIGN","value":"private"}]},
                {"name":"app","image":"our-image","env":[{"name":"OURS","value":"value"},{"name":"INJECTED","value":"foreign"}]}
            ],"set":["outside","ours"]},
            "data":{"ours":"public","foreign":"private"}
        });
        let fields = json!({
            "f:spec":{
                "f:containers":{"k:{\"name\":\"app\"}":{
                    ".":{}, "f:name":{}, "f:image":{},
                    "f:env":{"k:{\"name\":\"OURS\"}":{".":{},"f:name":{},"f:value":{}}}
                }},
                "f:set":{"v:\"ours\"":{}}
            },
            "f:data":{"f:ours":{}}
        });
        assert_eq!(
            project(&actual, &fields, 0).unwrap(),
            json!({
                "spec":{"containers":[{"name":"app","image":"our-image","env":[{"name":"OURS","value":"value"}]}],"set":["ours"]},
                "data":{"ours":"public"}
            })
        );
        for fields in [
            json!({"f:spec":{"f:containers":{"i:0":{}}}}),
            json!({"f:spec":{"f:containers":{"k:{\"name\":\"absent\"}":{}}}}),
            json!({"f:absent":{}}),
            json!({"unknown":{}}),
        ] {
            assert!(project(&actual, &fields, 0).is_err());
        }
        assert!(project(
            &json!([{"name":"same"},{"name":"same"}]),
            &json!({"k:{\"name\":\"same\"}":{}}),
            0
        )
        .is_err());
    }

    #[test]
    fn migration_keeps_other_versions_subresources_and_managers() {
        let own = json!({"manager":MANAGER,"operation":"Update","apiVersion":"v1","fieldsType":"FieldsV1","fieldsV1":{"f:data":{"f:x":{}}}});
        let mut entries = vec![own.clone()];
        for (key, value) in [
            ("manager", "another"),
            ("apiVersion", "v2"),
            ("subresource", "status"),
            ("operation", "Apply"),
        ] {
            let mut entry = own.clone();
            entry[key] = json!(value);
            entries.push(entry);
        }
        let actual = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"x","uid":"uid","resourceVersion":"1","managedFields":entries},"data":{"x":"current"}});
        let migration = migration(&actual, &actual).unwrap().unwrap();
        assert!(migration.bridge.is_none());
        assert_eq!(migration.retained, json!(entries[1..]));
        assert_eq!(migration.patch.as_array().unwrap().len(), 4);
        assert_eq!(
            migration.patch[3],
            json!({"op":"remove","path":"/metadata/managedFields/0"})
        );
    }
}
