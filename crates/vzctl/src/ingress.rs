//! Caddyfile generation for v0.2 ingress (#44).

use crate::config::{Environment, IngressUpstream};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::Path;

pub(crate) struct RenderedIngress {
    pub(crate) caddyfile: String,
    pub(crate) hosts: Vec<String>,
    /// vmnet host-service `.1` addresses (+ loopback bind) for HostGatewayIngressProxy.
    /// Guest TCP to bridge `.0` is blackholed on macOS vmnet; `.1` is reachable.
    pub(crate) gateways: Vec<String>,
    pub(crate) gateway_bindings: Vec<IngressGatewayBinding>,
    /// Public proxy listen ports (typically 80/443).
    pub(crate) http_port: u16,
    pub(crate) https_port: u16,
    /// Unprivileged ports where Caddy actually listens on `ingress.bind`.
    pub(crate) caddy_http_port: u16,
    pub(crate) caddy_https_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct IngressGatewayBinding {
    pub(crate) cidr: String,
    pub(crate) allowed_sources: Vec<String>,
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
    networks: &[Value],
    attachments: &[Value],
    state_dir: &Path,
) -> Result<RenderedIngress, String> {
    let Some(ingress) = environment.spec.ingress.as_ref().filter(|i| i.enabled) else {
        return Ok(RenderedIngress {
            caddyfile: String::new(),
            hosts: Vec::new(),
            gateways: Vec::new(),
            gateway_bindings: Vec::new(),
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
        }
    }

    let mut binding_sources: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for item in networks {
        if item["project"] != environment.spec.project
            || item["runtime_state"].as_str().unwrap_or("active") != "active"
            || item["backend"].as_str().unwrap_or("vmnet") == "docker"
        {
            continue;
        }
        let Some(cidr) = item["cidr"].as_str() else {
            continue;
        };
        let parsed = cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|error| format!("invalid live ingress CIDR {cidr:?}: {error}"))?;
        let canonical = parsed.trunc().to_string();
        let host_service = Ipv4Addr::from(u32::from(parsed.network()) + 1);
        gateways.insert(host_service.to_string(), ());
        binding_sources
            .entry(canonical.clone())
            .or_default()
            .insert(canonical);
    }

    // A logical Docker subnet has no host bridge. Its containers leave through
    // the owning Docker VM's primary (first non-Docker) vmnet NIC.
    for (docker_name, docker_network) in &environment.spec.networks {
        if docker_network.backend != crate::config::NetworkBackend::Docker {
            continue;
        }
        let owner = environment
            .spec
            .vms
            .values()
            .find(|vm| {
                vm.networks
                    .iter()
                    .any(|attachment| attachment.name == *docker_name)
            })
            .ok_or_else(|| format!("docker network {docker_name:?} has no owning VM"))?;
        let primary = owner
            .networks
            .iter()
            .find(|attachment| {
                environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend != crate::config::NetworkBackend::Docker)
            })
            .ok_or_else(|| format!("docker network {docker_name:?} owner has no vmnet NIC"))?;
        let primary_cidr = environment.spec.networks[&primary.name]
            .cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|error| format!("invalid primary vmnet CIDR: {error}"))?
            .trunc()
            .to_string();
        let docker_cidr = docker_network
            .cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|error| format!("invalid Docker CIDR: {error}"))?
            .trunc()
            .to_string();
        if let Some(sources) = binding_sources.get_mut(&primary_cidr) {
            sources.insert(docker_cidr);
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
        gateway_bindings: binding_sources
            .into_iter()
            .map(|(cidr, allowed_sources)| IngressGatewayBinding {
                cidr,
                allowed_sources: allowed_sources.into_iter().collect(),
            })
            .collect(),
        http_port,
        https_port,
        caddy_http_port,
        caddy_https_port,
    })
}

pub(crate) fn short_localhost(host: &str) -> Option<String> {
    host.split('.').next().map(|s| format!("{s}.localhost"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn active_vmnet_gateways_and_docker_primary_sources_are_rendered() {
        let environment = crate::config::validate_source(
            r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: demo }
spec:
  project: demo
  domain: demo.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
    lan: { cidr: 10.90.0.0/24, mode: shared }
    containers: { cidr: 10.95.0.0/24, mode: shared, backend: docker, dhcp: false, natEgress: false }
  routes: []
  policies: []
  certs: { enabled: true, onRotate: reinject }
  ingress:
    enabled: true
    routes:
      - { host: app.svc.demo.vz.test, to: "docker:8080" }
  vms:
    docker:
      from: ubuntu-base
      disk: 40G
      networks:
        - { name: lan, ip: 10.90.0.10 }
        - { name: containers, ip: 10.95.0.2 }
      roles: [docker, router]
"#,
        )
        .unwrap();
        let networks = vec![
            json!({"name":"dmz","cidr":"10.80.0.0/24","project":"demo","backend":"vmnet","runtime_state":"active"}),
            json!({"name":"lan","cidr":"10.90.0.0/24","project":"demo","backend":"vmnet","runtime_state":"active"}),
            json!({"name":"containers","cidr":"10.95.0.0/24","project":"demo","backend":"docker","runtime_state":"active"}),
            json!({"name":"stale","cidr":"10.70.0.0/24","project":"demo","backend":"vmnet","runtime_state":"orphaned"}),
            json!({"name":"foreign","cidr":"10.60.0.0/24","project":"other","backend":"vmnet","runtime_state":"active"}),
        ];
        let attachments = vec![json!({
            "vm_id":"demo/docker","network":"lan","ip":"10.90.0.10","project":"demo"
        })];

        let rendered =
            render(&environment, &networks, &attachments, &std::env::temp_dir()).unwrap();

        assert_eq!(rendered.gateways, ["10.80.0.1", "10.90.0.1", "127.0.0.1"]);
        assert_eq!(
            rendered.gateway_bindings,
            [
                IngressGatewayBinding {
                    cidr: "10.80.0.0/24".into(),
                    allowed_sources: vec!["10.80.0.0/24".into()],
                },
                IngressGatewayBinding {
                    cidr: "10.90.0.0/24".into(),
                    allowed_sources: vec!["10.90.0.0/24".into(), "10.95.0.0/24".into()],
                },
            ]
        );
    }
}
