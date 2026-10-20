use super::*;
use crate::controllers::{ClaimConfig, Consumer};

pub(crate) type ContainerBindings = BTreeMap<String, Vec<String>>;

pub(crate) fn for_vm(vm: &str, claims: &BTreeMap<String, ClaimConfig>) -> ContainerBindings {
    let mut result = BTreeMap::<String, Vec<String>>::new();
    for (name, claim) in claims {
        for consumer in &claim.consumers {
            if let Consumer::Container(target) = consumer {
                if let Some((node, container)) = target.container.split_once('/') {
                    if node == vm {
                        let values = result.entry(container.into()).or_default();
                        if !values.contains(name) {
                            values.push(name.clone());
                        }
                    }
                }
            }
        }
    }
    result
}

pub(super) fn mount(claim: &str) -> String {
    format!("/var/lib/vzctl/bindings/{claim}:/run/vzctl/bindings/{claim}:ro")
}

pub(super) fn with_mounts(
    mut volumes: Vec<String>,
    claims: &[String],
) -> Result<Vec<String>, String> {
    for claim in claims {
        let target = format!("/run/vzctl/bindings/{claim}");
        if volumes
            .iter()
            .any(|volume| volume.split(':').nth(1) == Some(&target))
        {
            return Err(format!(
                "container mount conflicts with managed binding {claim}"
            ));
        }
        volumes.push(mount(claim));
    }
    Ok(volumes)
}

pub(super) fn compose_services(path: &Path) -> Result<BTreeSet<String>, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("read Compose {}: {error}", path.display()))?;
    let document: JsonValue = serde_yaml::from_str(&source)
        .map_err(|error| format!("parse Compose {}: {error}", path.display()))?;
    document["services"]
        .as_object()
        .map(|services| services.keys().cloned().collect())
        .ok_or_else(|| {
            format!(
                "Compose {} requires services for binding resolution",
                path.display()
            )
        })
}

pub(super) fn compose_override(
    services: &BTreeSet<String>,
    bindings: &ContainerBindings,
) -> JsonValue {
    let services = bindings
        .iter()
        .filter(|(name, _)| services.contains(*name))
        .map(|(name, claims)| {
            let volumes = claims
                .iter()
                .map(|claim| {
                    json!({
                        "type":"bind","source":format!("/var/lib/vzctl/bindings/{claim}"),
                        "target":format!("/run/vzctl/bindings/{claim}"),"read_only":true,
                        "bind":{"create_host_path":false}
                    })
                })
                .collect::<Vec<_>>();
            (name.clone(), json!({"volumes":volumes}))
        })
        .collect::<serde_json::Map<_, _>>();
    json!({"services":services})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bindings_are_read_only_and_change_the_container_hash() {
        let container: ContainerConfig = serde_yaml::from_str("image: app:1\n").unwrap();
        let bare = container_config_hash(&container, &[]);
        let volumes = with_mounts(vec![], &["db".into()]).unwrap();
        assert_eq!(
            volumes,
            vec!["/var/lib/vzctl/bindings/db:/run/vzctl/bindings/db:ro"]
        );
        assert_ne!(bare, container_config_hash(&container, &volumes));
        assert!(with_mounts(volumes, &["db".into()]).is_err());
    }
    #[test]
    fn compose_override_only_targets_matching_services_and_never_creates_missing_sources() {
        let bindings = BTreeMap::from([
            ("api".into(), vec!["db".into()]),
            ("worker".into(), vec!["bucket".into()]),
        ]);
        let value = compose_override(&BTreeSet::from(["api".into()]), &bindings);
        assert!(value["services"].get("worker").is_none());
        let mount = &value["services"]["api"]["volumes"][0];
        assert_eq!(mount["read_only"], true);
        assert_eq!(mount["bind"]["create_host_path"], false);
        assert!(value.to_string().find("password").is_none());
    }
}
