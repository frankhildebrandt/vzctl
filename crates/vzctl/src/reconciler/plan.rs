use super::*;

pub(super) fn load_config(path: &Path) -> Result<Environment, Failure> {
    let path = config::config_path(path);
    config::validate_path(&path).map_err(|issues| {
        Failure::new(
            EXIT_INVALID,
            issues
                .iter()
                .map(|issue| format!("{}: {}", issue.path, issue.message))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })
}

pub(super) fn desired_resources(environment: &Environment) -> Result<Vec<Resource>, Failure> {
    let mut resources = Vec::new();
    let project = environment.spec.project.clone();
    let stack = stack_id(environment);
    let mut add = |kind: &str, name: &str, spec: Value, state: &str| -> Result<(), Failure> {
        let spec = serde_json::to_string(&spec).map_err(|error| {
            Failure::new(EXIT_STEP, format!("serialize desired state: {error}"))
        })?;
        resources.push(Resource {
            kind: kind.to_string(),
            name: name.to_string(),
            labels: BTreeMap::from([
                ("managed-by".to_string(), "vzctl".to_string()),
                ("project".to_string(), project.clone()),
                ("stack_id".to_string(), stack.clone()),
                ("spec".to_string(), spec),
            ]),
            state: state.to_string(),
        });
        Ok(())
    };
    add(
        "dns",
        &environment.spec.domain,
        json!(environment.spec.dns),
        "active",
    )?;
    for (name, image) in &environment.spec.images {
        add("image", name, json!(image), "sealed")?;
    }
    for (name, network) in &environment.spec.networks {
        add("network", name, json!(network), "active")?;
    }
    for (name, vm) in &environment.spec.vms {
        add(
            "vm",
            &vm_runtime_id(environment, name),
            json!(vm),
            "running",
        )?;
    }
    for route in &environment.spec.routes {
        add("route", &route.name, json!(route), "active")?;
    }
    for policy in &environment.spec.policies {
        add("policy", &policy.name, json!(policy), "active")?;
    }
    add(
        "resilience",
        "default",
        json!(environment.spec.resilience),
        "active",
    )?;
    add(
        "observability",
        "default",
        json!(environment.spec.observability),
        "active",
    )?;
    for port in &environment.spec.ports {
        add("port", port, json!(port), "active")?;
    }
    for (name, path) in &environment.spec.volumes {
        add("volume", name, json!(path), "active")?;
    }
    if let Some(certs) = &environment.spec.certs {
        add("certs", "default", json!(certs), "active")?;
    }
    if let Some(ingress) = &environment.spec.ingress {
        add("ingress", "default", json!(ingress), "active")?;
    }
    if let Some(oidc) = &environment.spec.oidc {
        add("oidc", "default", json!(oidc), "active")?;
    }
    for (name, cluster) in &environment.spec.kubernetes.clusters {
        add("kubernetes", name, json!(cluster), "ready")?;
    }
    for (name, provider) in &environment.spec.service_providers {
        add("service_provider", name, json!(provider), "configured")?;
    }
    for (name, claim) in &environment.spec.service_claims {
        add("service_claim", name, json!(claim), "ready")?;
    }
    for vm in environment.spec.vms.keys() {
        for (container, claims) in
            crate::docker::binding_consumers(vm, &environment.spec.service_claims)
        {
            add(
                "container_binding",
                &format!("{vm}/{container}"),
                json!({"claims":claims}),
                "mounted",
            )?;
        }
    }
    resources.sort_by(|left, right| (&left.kind, &left.name).cmp(&(&right.kind, &right.name)));
    Ok(resources)
}

pub(super) fn actual_resources(state: &Value) -> Result<Vec<Resource>, Failure> {
    serde_json::from_value(state["resources"].clone())
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("invalid stack state: {error}")))
}

pub(super) fn journal_context(state: &Value) -> Result<(Mode, bool), Failure> {
    let payload = state["journal"]["payload"]
        .as_str()
        .ok_or_else(|| Failure::new(EXIT_INCOMPLETE, "incomplete journal has no payload"))?;
    let payload: Value = serde_json::from_str(payload).map_err(|error| {
        Failure::new(EXIT_INCOMPLETE, format!("invalid journal payload: {error}"))
    })?;
    let mode = match payload["mode"].as_str() {
        Some("up") => Mode::Up,
        Some("apply") => Mode::Apply,
        Some("down") => Mode::Down,
        Some(other) => {
            return Err(Failure::new(
                EXIT_INCOMPLETE,
                format!("cannot resume journal mode {other}"),
            ))
        }
        None => {
            return Err(Failure::new(
                EXIT_INCOMPLETE,
                "incomplete journal has no mode",
            ))
        }
    };
    Ok((mode, payload["purge"].as_bool().unwrap_or(false)))
}

fn only_new_containers_added(current: &Resource, wanted: &Resource) -> bool {
    let read = |resource: &Resource| {
        resource
            .labels
            .get("spec")
            .and_then(|spec| serde_json::from_str::<VmConfig>(spec).ok())
    };
    let (Some(mut before), Some(mut after)) = (read(current), read(wanted)) else {
        // Incomplete or unknown historical state must not authorize a mutable update.
        return false;
    };
    if after.containers.len() <= before.containers.len()
        || !before
            .containers
            .iter()
            .all(|(name, config)| after.containers.get(name) == Some(config))
    {
        return false;
    }
    before.containers.clear();
    after.containers.clear();
    match (serde_json::to_value(before), serde_json::to_value(after)) {
        (Ok(before), Ok(after)) => before == after,
        _ => false,
    }
}

fn binding_consumer_may_exist(binding: &Resource, actual: &[Resource]) -> bool {
    let (Some((vm, container)), Some(project)) =
        (binding.name.split_once('/'), binding.labels.get("project"))
    else {
        return actual.iter().any(|resource| resource.kind == "vm");
    };
    let runtime_id = format!("{project}/{vm}");
    let previous_vm = actual.iter().find(|resource| {
        resource.kind == "vm"
            && (resource.name == runtime_id
                || (resource.name == vm
                    && resource
                        .labels
                        .get("project")
                        .is_none_or(|owner| owner == project)))
    });
    let Some(previous_vm) = previous_vm else {
        return false;
    };
    let Some(previous) = previous_vm
        .labels
        .get("spec")
        .and_then(|spec| serde_json::from_str::<VmConfig>(spec).ok())
    else {
        return true;
    };
    // Historical state stores Compose paths, not their service inventory. A service
    // may already exist there, so adding bindings still requires confirmation.
    previous.containers.contains_key(container) || !previous.compose_files.is_empty()
}

pub(super) fn build_plan(mode: Mode, desired: Vec<Resource>, actual: Vec<Resource>) -> Plan {
    let desired_map = desired
        .iter()
        .map(|resource| ((resource.kind.clone(), resource.name.clone()), resource))
        .collect::<BTreeMap<_, _>>();
    let actual_map = actual
        .iter()
        .map(|resource| ((resource.kind.clone(), resource.name.clone()), resource))
        .collect::<BTreeMap<_, _>>();
    let mut actions = Vec::new();
    for (key, wanted) in &desired_map {
        match actual_map.get(key) {
            None => {
                let recreate = wanted.kind == "container_binding"
                    && binding_consumer_may_exist(wanted, &actual);
                actions.push(PlanAction {
                    action: if recreate { "recreate" } else { "create" },
                    kind: wanted.kind.clone(),
                    name: wanted.name.clone(),
                    breaking: recreate,
                    reason: if recreate {
                        "managed binding mount required; existing consumer containers are recreated"
                    } else {
                        "missing from actual state"
                    }
                    .to_string(),
                });
            }
            Some(current) => {
                if current.labels.get("spec") != wanted.labels.get("spec") && mode != Mode::Up {
                    let added_containers =
                        wanted.kind == "vm" && only_new_containers_added(current, wanted);
                    actions.push(PlanAction {
                        action: if wanted.kind == "container_binding" {
                            "recreate"
                        } else {
                            "update"
                        },
                        kind: wanted.kind.clone(),
                        name: wanted.name.clone(),
                        breaking: matches!(wanted.kind.as_str(), "network" | "container_binding")
                            || (wanted.kind == "vm" && !added_containers),
                        reason: if wanted.kind == "container_binding" {
                            "binding mounts changed; consumer container must be recreated"
                        } else if added_containers {
                            "new containers added; existing containers and VM configuration are unchanged"
                        } else {
                            "desired spec differs from actual state"
                        }
                        .to_string(),
                    });
                } else if wanted.kind == "vm" && current.state == "stopped" && mode != Mode::Down {
                    actions.push(PlanAction {
                        action: "start",
                        kind: wanted.kind.clone(),
                        name: wanted.name.clone(),
                        breaking: false,
                        reason: "VM is stopped".to_string(),
                    });
                }
            }
        }
    }
    if !matches!(mode, Mode::Up | Mode::Down) {
        for (key, current) in &actual_map {
            if !desired_map.contains_key(key) {
                actions.push(PlanAction {
                    action: if matches!(current.kind.as_str(), "kubernetes" | "service_claim" | "service_provider") { "retain" } else if current.kind == "container_binding" { "recreate" } else { "delete" },
                    kind: current.kind.clone(),
                    name: current.name.clone(),
                    breaking: !matches!(current.kind.as_str(), "kubernetes" | "service_claim" | "service_provider"),
                    reason: if current.kind == "kubernetes" {
                        "Kubernetes declaration removed; existing cluster and objects are retained (no implicit prune)"
                    } else if current.kind == "container_binding" { "binding mounts removed; consumer container must be recreated" } else if matches!(current.kind.as_str(), "service_claim" | "service_provider") { "service declaration removed; external service resources are retained" } else { "managed resource absent from desired state" }.to_string(),
                });
            }
        }
    }
    if mode == Mode::Down {
        for vm in desired.iter().filter(|resource| resource.kind == "vm") {
            actions.push(PlanAction {
                action: "stop",
                kind: "vm".to_string(),
                name: vm.name.clone(),
                breaking: false,
                reason: "down stops VMs in reverse dependency order".to_string(),
            });
        }
    }
    actions.sort_by(|left, right| {
        (&left.kind, &left.name, left.action).cmp(&(&right.kind, &right.name, right.action))
    });
    Plan { actions, desired }
}

pub(super) fn dependency_order(vms: &BTreeMap<String, VmConfig>) -> Result<Vec<String>, Failure> {
    fn visit(
        name: &str,
        vms: &BTreeMap<String, VmConfig>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        result: &mut Vec<String>,
    ) -> Result<(), Failure> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            return Err(Failure::new(EXIT_INVALID, "VM dependency cycle"));
        }
        for dependency in &vms[name].depends_on {
            visit(dependency, vms, visiting, visited, result)?;
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        result.push(name.to_string());
        Ok(())
    }
    let mut result = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for name in vms.keys() {
        visit(name, vms, &mut visiting, &mut visited, &mut result)?;
    }
    Ok(result)
}

pub(super) fn stack_id(environment: &Environment) -> String {
    format!("{}:{}", environment.spec.project, environment.metadata.name)
}

pub(super) fn desired_hash(
    environment: &Environment,
    resources: &[Resource],
) -> Result<String, Failure> {
    // Hash the whole parsed document as well as file fingerprints. New config fields
    // are covered automatically, even before they gain a dedicated plan resource.
    let bytes = serde_json::to_vec(&(environment, resources))
        .map_err(|error| Failure::new(EXIT_STEP, format!("hash desired state: {error}")))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(super) fn plan_output(stack_id: &str, plan: &Plan) -> Value {
    json!({
        "message": if plan.actions.is_empty() { "no changes" } else { "changes planned" },
        "stack_id": stack_id,
        "actions": plan.actions,
        "changed": !plan.actions.is_empty(),
        "breaking": plan.actions.iter().filter(|action| action.breaking).count(),
    })
}
