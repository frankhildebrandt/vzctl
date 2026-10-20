//! A clean stop is neutral. Failed/unknown helpers remain diagnostic failures.
use super::*;

pub(super) struct Activity<'a> {
    environment: &'a Environment,
    runtime: &'a Value,
}

impl<'a> Activity<'a> {
    pub(super) fn new(environment: &'a Environment, runtime: &'a Value) -> Self {
        Self {
            environment,
            runtime,
        }
    }

    pub(super) fn state(&self, name: &str) -> &str {
        let id = format!("{}/{}", self.environment.spec.project, name);
        self.runtime
            .as_array()
            .into_iter()
            .flatten()
            .find(|record| record["vm_id"] == id)
            .map(|record| record["state"].as_str().unwrap_or("unknown"))
            // vm.list deliberately removes normally stopped helpers; unexpected
            // process exits retain a failed record in the Supervisor.
            .unwrap_or("stopped")
    }

    pub(super) fn stopped(&self, name: &str) -> bool {
        self.environment.spec.vms.contains_key(name) && self.state(name) == "stopped"
    }

    pub(super) fn all_stopped(&self) -> bool {
        !self.environment.spec.vms.is_empty()
            && self
                .environment
                .spec
                .vms
                .keys()
                .all(|name| self.stopped(name))
    }

    pub(super) fn network_needed(&self, network: &str) -> bool {
        self.environment.spec.vms.iter().any(|(name, vm)| {
            !self.stopped(name) && vm.networks.iter().any(|net| net.name == network)
        })
    }

    pub(super) fn ingress_dependency(&self, route: &crate::config::IngressRoute) -> Option<String> {
        match crate::config::IngressUpstream::parse(&route.to) {
            Ok(crate::config::IngressUpstream::Vm { name, .. }) if self.stopped(&name) => {
                Some(name)
            }
            _ => None,
        }
    }

    pub(super) fn host_ingress_needed(&self) -> bool {
        self.environment
            .spec
            .ingress
            .as_ref()
            .is_some_and(|ingress| {
                ingress.enabled
                    && (self
                        .environment
                        .spec
                        .oidc
                        .as_ref()
                        .is_some_and(|oidc| oidc.enabled)
                        || ingress
                            .routes
                            .iter()
                            .any(|route| self.ingress_dependency(route).is_none()))
            })
    }

    pub(super) fn checks_skipped(&self) -> bool {
        self.all_stopped()
            && !self.host_ingress_needed()
            && self
                .environment
                .spec
                .observability
                .probes
                .iter()
                .all(|probe| self.probe_dependency(probe).is_some())
    }

    pub(super) fn target_dependency(&self, target: &str) -> Option<String> {
        let authority = target
            .split_once("://")
            .map_or(target, |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("");
        let host = authority
            .split(':')
            .next()
            .unwrap_or("")
            .trim_end_matches('.');
        let project = &self.environment.spec.project;
        if let Some(ingress) = &self.environment.spec.ingress {
            if let Some(route) = ingress.routes.iter().find(|r| r.host == host) {
                return self.ingress_dependency(route);
            }
        }
        self.environment.spec.vms.iter().find_map(|(name, vm)| {
            if !self.stopped(name) {
                return None;
            }
            vm.networks
                .iter()
                .any(|net| {
                    host == net.ip || host == format!("{name}.{}.{}.vz.test", net.name, project)
                })
                .then(|| name.clone())
        })
    }

    pub(super) fn probe_dependency(&self, probe: &ObservabilityProbe) -> Option<String> {
        (probe.from != "host" && self.stopped(&probe.from))
            .then(|| probe.from.clone())
            .or_else(|| self.target_dependency(&probe.target))
    }
}

pub(super) fn skipped(vm: Option<&str>) -> Value {
    json!({"status":"skipped", "reason":"vm_stopped", "vm":vm})
}

/// Only a complete, unambiguous inventory can establish absence as a clean stop.
pub(super) fn validate_inventory(runtime: &Value) -> Result<(), (String, u8)> {
    let mut ids = std::collections::BTreeSet::new();
    if runtime.as_array().is_some_and(|records| {
        records.iter().all(|record| {
            record["vm_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty() && ids.insert(id))
                && record["state"]
                    .as_str()
                    .is_some_and(|state| !state.is_empty())
        })
    }) {
        return Ok(());
    }
    Err((
        "VM inventory is malformed; stopped state cannot be determined".into(),
        EXIT_INVALID,
    ))
}
