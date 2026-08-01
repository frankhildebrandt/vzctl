//! Caddyfile generation for v0.2 ingress (#44).

use crate::config::{Environment, IngressUpstream};
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::Path;

pub(crate) struct RenderedIngress {
    pub(crate) caddyfile: String,
    pub(crate) hosts: Vec<String>,
    /// vmnet host-service `.1` addresses (+ loopback bind) for HostGatewayIngressProxy.
    /// Guest TCP to bridge `.0` is blackholed on macOS vmnet; `.1` is reachable.
    pub(crate) gateways: Vec<String>,
    /// Public proxy listen ports (typically 80/443).
    pub(crate) http_port: u16,
    pub(crate) https_port: u16,
    /// Unprivileged ports where Caddy actually listens on `ingress.bind`.
    pub(crate) caddy_http_port: u16,
    pub(crate) caddy_https_port: u16,
}

/// Map privileged public ports to unprivileged loopback ports for the Caddy child.
pub(crate) fn caddy_listen_port(public: u16) -> u16 {
    if public >= 1024 {
        public
    } else {
        public.saturating_add(18_000)
    }
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
            http_port: 80,
            https_port: 443,
            caddy_http_port: caddy_listen_port(80),
            caddy_https_port: caddy_listen_port(443),
        });
    };

    let mut vm_ips: BTreeMap<String, String> = BTreeMap::new();
    let mut gateways = BTreeMap::new();
    // Host clients resolve *.svc → 127.0.0.1; proxy that loopback too.
    gateways.insert(ingress.bind.clone(), ());
    for item in attachments {
        let Some(vm) = item["vm_id"].as_str() else {
            continue;
        };
        if item["project"] != environment.spec.project {
            continue;
        }
        if let Some(ip) = item["ip"].as_str() {
            vm_ips
                .entry(vm.to_string())
                .or_insert_with(|| ip.to_string());
            if let Some(network) = item["network"].as_str() {
                if let Some(net) = environment.spec.networks.get(network) {
                    if matches!(net.backend, crate::config::NetworkBackend::Docker) {
                        continue;
                    }
                    if let Ok(cidr) = net.cidr.parse::<ipnet::Ipv4Net>() {
                        // Host-service address = network + 1 (not gateway .0).
                        let host_service = Ipv4Addr::from(u32::from(cidr.network()) + 1);
                        gateways.insert(host_service.to_string(), ());
                    }
                }
            }
        }
    }

    let bind = &ingress.bind;
    let http_port = ingress.http_port;
    let https_port = ingress.https_port;
    let caddy_http_port = caddy_listen_port(http_port);
    let caddy_https_port = caddy_listen_port(https_port);

    let mut blocks = format!(
        "{{\n\tauto_https off\n\thttp_port {caddy_http_port}\n\thttps_port {caddy_https_port}\n\tdefault_bind {bind}\n}}\n\n"
    );
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
                    "http://{host} {{\n\tbind {bind}\n\tredir https://{host}{{uri}} permanent\n}}\n\n"
                ));
            }
        }

        blocks.push_str(&format!(
            "https://{} {{\n\tbind {bind}\n\ttls \"{}\" \"{}\"\n\treverse_proxy {}\n}}\n\n",
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
        http_port,
        https_port,
        caddy_http_port,
        caddy_https_port,
    })
}

pub(crate) fn short_localhost(host: &str) -> Option<String> {
    host.split('.').next().map(|s| format!("{s}.localhost"))
}
