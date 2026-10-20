//! Declarative, pinned K3s clusters on existing systemd ARM64 guests.
mod cli;
mod controller;
#[cfg(test)]
mod live_contract_tests;
mod managed_fields;
mod manifests;
mod resolver;
#[cfg(test)]
mod tests;

use crate::config::{Environment, ValidationIssue};
pub(crate) use cli::command;
pub(crate) use controller::ensure;
use ipnet::Ipv4Net;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KubernetesConfig {
    #[serde(default)]
    pub(crate) clusters: BTreeMap<String, ClusterConfig>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClusterConfig {
    #[schemars(regex(pattern = r"^v[0-9]+\.[0-9]+\.[0-9]+\+k3s[0-9]+$"))]
    pub(crate) version: String,
    #[schemars(regex(pattern = r"^[a-fA-F0-9]{64}$"))]
    pub(crate) sha256: String,
    pub(crate) server: String,
    #[serde(default)]
    pub(crate) agents: Vec<String>,
    pub(crate) network: String,
    #[serde(rename = "podCIDR")]
    pub(crate) pod_cidr: String,
    #[serde(rename = "serviceCIDR")]
    pub(crate) service_cidr: String,
    #[serde(default)]
    pub(crate) manifests: Vec<String>,
    #[serde(default = "default_namespace", rename = "defaultNamespace")]
    pub(crate) default_namespace: String,
}

fn default_namespace() -> String {
    "default".into()
}

fn dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value.as_bytes()[0] != b'-'
        && value.as_bytes()[value.len() - 1] != b'-'
}

pub(crate) fn manifest_paths(environment: &Environment, base: &Path) -> Vec<PathBuf> {
    environment
        .spec
        .kubernetes
        .clusters
        .values()
        .flat_map(|cluster| cluster.manifests.iter().map(|path| base.join(path)))
        .collect()
}

pub(crate) fn validate(
    environment: &Environment,
    base: Option<&Path>,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut used_vms = BTreeSet::new();
    let mut cluster_cidrs: Vec<(String, Ipv4Net)> = Vec::new();
    for (name, cluster) in &environment.spec.kubernetes.clusters {
        let at = format!("$.spec.kubernetes.clusters.{name}");
        let mut issue = |field: &str, message: String| {
            issues.push(ValidationIssue {
                path: format!("{at}{field}"),
                message,
                kind: "semantic",
            })
        };
        if !dns_label(name) {
            issue("", "cluster name must be a DNS label".into());
        }
        if !dns_label(&cluster.default_namespace) {
            issue(".defaultNamespace", "namespace must be a DNS label".into());
        }
        if !environment.spec.networks.contains_key(&cluster.network) {
            issue(".network", format!("unknown network {:?}", cluster.network));
        } else if environment.spec.networks[&cluster.network].backend
            == crate::config::NetworkBackend::Docker
        {
            issue(
                ".network",
                "K3s nodes require a vmnet network, not a Docker bridge".into(),
            );
        }
        for vm_name in std::iter::once(&cluster.server).chain(cluster.agents.iter()) {
            if !dns_label(vm_name) {
                issue(
                    "",
                    format!("Kubernetes node {vm_name:?} must be a DNS label"),
                );
            }
            if !used_vms.insert(vm_name) {
                issue(
                    "",
                    format!("VM {vm_name:?} occurs more than once or belongs to multiple clusters"),
                );
            }
            match environment.spec.vms.get(vm_name) {
                None => issue("", format!("unknown VM {vm_name:?}")),
                Some(vm) if !vm.networks.iter().any(|n| n.name == cluster.network) => {
                    issue(
                        ".network",
                        format!("VM {vm_name:?} is not attached to {:?}", cluster.network),
                    );
                }
                _ => {}
            }
        }
        for (field, value, max_prefix) in [
            ("podCIDR", &cluster.pod_cidr, 24),
            ("serviceCIDR", &cluster.service_cidr, 28),
        ] {
            match value.parse::<Ipv4Net>() {
                Ok(cidr)
                    if cidr.addr() == cidr.network()
                        && (8..=max_prefix).contains(&cidr.prefix_len()) =>
                {
                    if field == "podCIDR"
                        && cluster.agents.len().saturating_add(1)
                            > (1usize << (24 - cidr.prefix_len()))
                    {
                        issue(
                            ".podCIDR",
                            "pod CIDR must provide one /24 subnet per node".into(),
                        );
                    }
                    for (net_name, net) in &environment.spec.networks {
                        if let Ok(other) = net.cidr.parse::<Ipv4Net>() {
                            if cidr.contains(&other.network()) || other.contains(&cidr.network()) {
                                issue(
                                    &format!(".{field}"),
                                    format!("CIDR overlaps hypernetwork {net_name:?}"),
                                );
                            }
                        }
                    }
                    for (other_name, other) in &cluster_cidrs {
                        if cidr.contains(&other.network()) || other.contains(&cidr.network()) {
                            issue(&format!(".{field}"), format!("CIDR overlaps {other_name}"));
                        }
                    }
                    cluster_cidrs.push((format!("{name}.{field}"), cidr));
                }
                _ => issue(
                    &format!(".{field}"),
                    format!("expected canonical IPv4 CIDR /8 through /{max_prefix}"),
                ),
            }
        }
        if cluster.manifests.len() > 32 {
            issue(
                ".manifests",
                "at most 32 manifest files are supported".into(),
            );
        }
        for (index, path) in cluster.manifests.iter().enumerate() {
            if path.is_empty() || path.contains("://") {
                issue(
                    &format!(".manifests[{index}]"),
                    "expected a local YAML file path".into(),
                );
            } else if let Some(base) = base {
                match manifests::read(&base.join(path)) {
                    Err(error) => issue(&format!(".manifests[{index}]"), error),
                    Ok(objects) => {
                        for object in objects {
                            if let Err(error) =
                                manifests::owned(object, &environment.spec.project, name)
                            {
                                issue(&format!(".manifests[{index}]"), error);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn node_ip<'a>(
    environment: &'a Environment,
    cluster: &ClusterConfig,
    vm: &str,
) -> Result<&'a str, String> {
    let node = environment
        .spec
        .vms
        .get(vm)
        .ok_or_else(|| format!("K3s node {vm} does not exist"))?;
    let mut matches = node.networks.iter().filter(|n| n.name == cluster.network);
    let attachment = matches
        .next()
        .ok_or_else(|| format!("K3s node {vm} lacks network {}", cluster.network))?;
    if matches.next().is_some() {
        return Err(format!(
            "K3s node {vm} has ambiguous network {}",
            cluster.network
        ));
    }
    Ok(attachment.ip.as_str())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
