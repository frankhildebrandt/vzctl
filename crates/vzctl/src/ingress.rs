//! Caddyfile generation for v0.2 ingress (#44).

use crate::config::{Environment, IngressUpstream};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) struct RenderedIngress {
    pub(crate) caddyfile: String,
    pub(crate) hosts: Vec<String>,
    pub(crate) gateways: Vec<String>,
}

/// Build a Caddyfile and host list from the environment + live attachments.
pub(crate) fn render(
    environment: &Environment,
    attachments: &[Value],
    state_dir: &Path,
) -> Result<RenderedIngress, String> {
    let Some(ingress) = environment.spec.ingress.as_ref().filter(|i| i.enabled) else {
        return Ok(RenderedIngress {
            caddyfile: String::new(),
            hosts: Vec::new(),
            gateways: Vec::new(),
        });
    };

    let mut vm_ips: BTreeMap<String, String> = BTreeMap::new();
    let mut gateways = BTreeMap::new();
    for item in attachments {
        let Some(vm) = item["vm_id"].as_str() else {
            continue;
        };
        if item["project"] != environment.spec.project {
            continue;
        }
        if let Some(ip) = item["ip"].as_str() {
            vm_ips.entry(vm.to_string()).or_insert_with(|| ip.to_string());
            if let Some(network) = item["network"].as_str() {
                if let Some(net) = environment.spec.networks.get(network) {
                    if let Ok(cidr) = net.cidr.parse::<ipnet::Ipv4Net>() {
                        let gw = cidr.network();
                        gateways.insert(gw.to_string(), ());
                    }
                }
            }
        }
    }

    let mut blocks = String::from("{\n\tauto_https off\n}\n\n");
    let mut hosts = Vec::new();

    for route in &ingress.routes {
        hosts.push(route.host.clone());
        let upstream = IngressUpstream::parse(&route.to)?;
        let backend = match upstream {
            IngressUpstream::Vm { name, port } => {
                let runtime = crate::runtime_vm_id(&environment.spec.project, &name);
                let ip = vm_ips
                    .get(&runtime)
                    .or_else(|| vm_ips.get(&name))
                    .ok_or_else(|| {
                        format!(
                            "ingress route {} needs attachment IP for VM {runtime}",
                            route.host
                        )
                    })?;
                format!("{ip}:{port}")
            }
            IngressUpstream::Oidc { port } => format!("127.0.0.1:{port}"),
        };

        let (cert, key, _) = crate::certs::leaf_paths(state_dir, &route.host);
        let mut site_hosts = vec![route.host.clone()];
        if ingress.host_aliases {
            if let Some(short) = route.host.split('.').next() {
                let alias = format!("{short}.localhost");
                site_hosts.push(alias.clone());
                // Also mint alias as SAN via ensure path; list for DNS host aliases only on host.
            }
        }

        if ingress.redirect_http {
            for host in &site_hosts {
                blocks.push_str(&format!(
                    "http://{host} {{\n\tredir https://{host}{{uri}} permanent\n}}\n\n"
                ));
            }
        }

        blocks.push_str(&format!(
            "https://{} {{\n\ttls \"{}\" \"{}\"\n\treverse_proxy {}\n}}\n\n",
            site_hosts.join(", "),
            cert.display(),
            key.display(),
            backend
        ));
    }

    Ok(RenderedIngress {
        caddyfile: blocks,
        hosts,
        gateways: gateways.into_keys().collect(),
    })
}

pub(crate) fn short_localhost(host: &str) -> Option<String> {
    host.split('.').next().map(|s| format!("{s}.localhost"))
}
