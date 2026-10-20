use super::*;
use crate::config::{Environment, ValidationIssue};
use std::collections::BTreeSet;
fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
fn dns_label(value: &str) -> bool {
    key(value)
        && !value.contains('_')
        && value == value.to_ascii_lowercase()
        && !value.starts_with('-')
        && !value.ends_with('-')
}
fn dns_name(value: &str) -> bool {
    value.len() <= 253 && value.split('.').all(dns_label)
}
fn loopback(value: &str) -> bool {
    let endpoint = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .unwrap_or(value)
        .trim_end_matches('/');
    let host = if let Some(bracketed) = endpoint.strip_prefix('[') {
        bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .unwrap_or(bracketed)
    } else {
        endpoint
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(endpoint)
    };
    let hostname = host.to_ascii_lowercase();
    hostname == "localhost"
        || hostname.ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
            std::net::IpAddr::V4(ip) => ip.is_loopback(),
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback())
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_origins_require_advertised_address_even_without_port() {
        for endpoint in [
            "http://localhost/",
            "https://LOCALHOST",
            "http://127.0.0.1/",
            "http://[::1]",
            "https://[::1]/",
            "[::1]:5432",
            "http://[::ffff:127.0.0.1]/",
            "http://app.localhost:9000",
        ] {
            assert!(loopback(endpoint), "{endpoint}");
        }
        for endpoint in [
            "http://objects:9000",
            "http://[2001:db8::1]/",
            "db.lan.demo.vz.test:5432",
        ] {
            assert!(!loopback(endpoint), "{endpoint}");
        }
        for endpoint in ["http://127.1/", "http://2130706433/"] {
            assert!(
                !super::super::s3::valid_endpoint(endpoint),
                "ambiguous numeric hosts must be rejected"
            );
        }
    }

    #[test]
    fn remote_claim_cannot_advertise_loopback_or_omit_address() {
        let mut environment: Environment = serde_yaml::from_str(include_str!(
            "../../tests/fixtures/validate/valid-kubernetes.yaml"
        ))
        .unwrap();
        environment.spec.service_claims.insert(
            "files".into(),
            serde_json::from_value(serde_json::json!({
                "kind":"s3","bucket":"app-files","consumers":[{"vm":"kube-worker"}]
            }))
            .unwrap(),
        );
        for endpoint in [
            "http://localhost/",
            "http://[::1]",
            "http://127.0.0.1:9000/",
        ] {
            environment.spec.service_providers.insert(
                "objects".into(),
                serde_json::from_value(serde_json::json!({
                    "kind":"s3","via":"kube-server","endpoint":endpoint,"implementation":"minio",
                    "accessKeyFile":"access","secretKeyFile":"secret"
                }))
                .unwrap(),
            );
            assert!(
                validate_config(&environment)
                    .iter()
                    .any(|i| i.message.contains("advertiseEndpoint")),
                "{endpoint}"
            );
            let provider = environment
                .spec
                .service_providers
                .get_mut("objects")
                .unwrap();
            provider.advertise_endpoint = Some("http://[::1]/".into());
            assert!(validate_config(&environment)
                .iter()
                .any(|i| i.message.contains("advertiseEndpoint")));
            environment
                .spec
                .service_providers
                .get_mut("objects")
                .unwrap()
                .advertise_endpoint = Some("http://objects.lan.example.test:9000".into());
            assert!(validate_config(&environment).is_empty());
        }
    }
    #[test]
    fn distinct_provider_vms_may_use_the_same_loopback_address_and_database_name() {
        let mut environment: Environment = serde_yaml::from_str(include_str!(
            "../../tests/fixtures/validate/valid-kubernetes.yaml"
        ))
        .unwrap();
        for (name, vm) in [("first", "kube-server"), ("second", "kube-worker")] {
            environment.spec.service_providers.insert(name.into(),serde_json::from_value(serde_json::json!({
                "kind":"postgres","via":vm,"endpoint":"127.0.0.1:5432","adminUser":"postgres","passwordFile":"admin"
            })).unwrap());
            environment.spec.service_claims.insert(
                name.into(),
                serde_json::from_value(serde_json::json!({
                    "kind":"postgres","database":"app","provider":name
                }))
                .unwrap(),
            );
        }
        assert!(validate_config(&environment).is_empty());
        environment
            .spec
            .service_providers
            .get_mut("second")
            .unwrap()
            .via = Some("kube-server".into());
        assert!(validate_config(&environment)
            .iter()
            .any(|issue| issue.message.contains("already declares")));
    }
}
pub(crate) fn validate_config(environment: &Environment) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: &str| {
        issues.push(ValidationIssue {
            path,
            message: message.into(),
            kind: "semantic",
        })
    };
    for (name, provider) in &environment.spec.service_providers {
        let path = format!("spec.serviceProviders.{name}");
        if !key(name) {
            issue(
                path.clone(),
                "provider name must contain 1..63 letters, digits, underscores or hyphens",
            );
        }
        if let Some(vm) = &provider.via {
            if !environment.spec.vms.contains_key(vm) {
                issue(format!("{path}.via"), "provider references unknown VM");
            }
        }
        match provider.kind {
            ServiceKind::Postgres | ServiceKind::S3 => {
                if provider.via.is_none() || provider.endpoint.is_none() {
                    issue(path.clone(), "provider requires via and endpoint");
                }
                for endpoint in provider
                    .endpoint
                    .iter()
                    .chain(provider.advertise_endpoint.iter())
                {
                    let valid = if provider.kind == ServiceKind::Postgres {
                        super::postgres::endpoint(endpoint).is_ok()
                    } else {
                        super::s3::valid_endpoint(endpoint)
                    };
                    if !valid {
                        issue(path.clone(),"provider endpoint is invalid; credentials in endpoint URLs are forbidden");
                    }
                }
                if provider.kind == ServiceKind::Postgres
                    && (provider.admin_user.as_ref().is_none_or(|s| s.is_empty())
                        || provider.password_file.as_ref().is_none_or(|s| s.is_empty()))
                {
                    issue(
                        path.clone(),
                        "PostgreSQL requires adminUser and passwordFile",
                    );
                }
                if provider.kind == ServiceKind::S3
                    && (provider.implementation.as_deref() != Some("minio")
                        || provider
                            .access_key_file
                            .as_ref()
                            .is_none_or(|s| s.is_empty())
                        || provider
                            .secret_key_file
                            .as_ref()
                            .is_none_or(|s| s.is_empty()))
                {
                    issue(
                        path.clone(),
                        "S3 requires implementation: minio, accessKeyFile and secretKeyFile",
                    );
                }
            }
            ServiceKind::Ingress | ServiceKind::Oidc => {
                if provider.via.is_some() || provider.endpoint.is_some() {
                    issue(path,"Ingress/OIDC use the configured stack edge provider, not an external endpoint");
                }
            }
        }
    }
    let mut resources = BTreeSet::new();
    let mut secret_targets = BTreeSet::new();
    for (name, claim) in &environment.spec.service_claims {
        let path = format!("spec.serviceClaims.{name}");
        if !key(name) {
            issue(
                path.clone(),
                "claim name must contain 1..63 letters, digits, underscores or hyphens",
            );
        }
        let compatible = environment
            .spec
            .service_providers
            .iter()
            .filter(|(_, p)| p.kind == claim.kind)
            .collect::<Vec<_>>();
        let selected = if let Some(provider) = &claim.provider {
            let selected = compatible
                .iter()
                .find(|(n, _)| *n == provider)
                .map(|(_, p)| *p);
            if selected.is_none() {
                issue(
                    format!("{path}.provider"),
                    "unknown or incompatible provider",
                );
            }
            selected
        } else if compatible.len() == 1 {
            Some(compatible[0].1)
        } else {
            if compatible.len() > 1
                || (compatible.is_empty()
                    && matches!(claim.kind, ServiceKind::Postgres | ServiceKind::S3))
            {
                issue(
                    format!("{path}.provider"),
                    "select exactly one compatible provider",
                );
            }
            None
        };
        let required = match claim.kind {
            ServiceKind::Postgres => claim.database.as_deref(),
            ServiceKind::S3 => claim.bucket.as_deref(),
            ServiceKind::Oidc => claim.client_id.as_deref(),
            ServiceKind::Ingress => claim.host.as_deref(),
        };
        if required.is_none_or(str::is_empty) {
            issue(path.clone(), "claim is missing its resource name");
        }
        if let Some(resource) = required {
            let identity = format!(
                "{:?}/{}/{}/{resource}",
                claim.kind,
                selected
                    .and_then(|p| p.via.as_deref())
                    .unwrap_or("stack-edge"),
                selected
                    .and_then(|p| p.endpoint.as_deref())
                    .unwrap_or("stack-edge")
            );
            if !resources.insert(identity) {
                issue(
                    path.clone(),
                    "another claim already declares this provider resource",
                );
            }
        }
        match claim.kind {
            ServiceKind::Postgres => {
                if !claim
                    .database
                    .as_deref()
                    .is_some_and(super::postgres::database_name)
                {
                    issue(
                        format!("{path}.database"),
                        "invalid or reserved PostgreSQL database name",
                    );
                }
            }
            ServiceKind::S3 => {
                if !claim.bucket.as_deref().is_some_and(super::s3::valid_bucket) {
                    issue(
                        format!("{path}.bucket"),
                        "invalid or reserved S3 bucket name",
                    );
                }
            }
            ServiceKind::Ingress => {
                if !claim.host.as_deref().is_some_and(dns_name) {
                    issue(format!("{path}.host"), "ingress host must be a DNS name");
                }
                if !environment.spec.ingress.as_ref().is_some_and(|i| i.enabled) {
                    issue(path.clone(), "ingress claim requires enabled spec.ingress");
                }
                if environment.spec.ingress.as_ref().is_some_and(|i| {
                    i.routes
                        .iter()
                        .any(|r| Some(&r.host) == claim.host.as_ref())
                }) {
                    issue(
                        path.clone(),
                        "ingress host is already declared in spec.ingress.routes",
                    );
                }
                match &claim.backend {
                    Some(Backend::Vm(backend)) => {
                        if !environment.spec.vms.contains_key(&backend.vm) || backend.port == 0 {
                            issue(format!("{path}.backend"), "unknown VM or invalid port");
                        }
                    }
                    Some(Backend::Kubernetes(backend)) => {
                        let target = &backend.kubernetes;
                        if !environment
                            .spec
                            .kubernetes
                            .clusters
                            .contains_key(&target.cluster)
                            || !dns_label(&target.namespace)
                            || !dns_name(&target.service)
                            || target.port == 0
                        {
                            issue(
                                format!("{path}.backend"),
                                "invalid Kubernetes service backend",
                            );
                        }
                    }
                    None => issue(format!("{path}.backend"), "ingress requires backend"),
                }
            }
            ServiceKind::Oidc => {
                if !environment.spec.oidc.as_ref().is_some_and(|o| o.enabled) {
                    issue(path.clone(), "OIDC claim requires enabled spec.oidc");
                }
                if !claim.client_id.as_deref().is_some_and(key) {
                    issue(
                        path.clone(),
                        "OIDC clientID must contain 1..63 letters, digits, underscores or hyphens",
                    );
                }
                let scopes: BTreeSet<_> = claim.scopes.iter().map(String::as_str).collect();
                if !scopes.is_empty() && scopes != BTreeSet::from(["openid", "profile", "email"]) {
                    issue(format!("{path}.scopes"),"UnsupportedOidcScopes: built-in providers support standard openid/profile/email; per-client restrictions are unavailable");
                }
                if claim.redirect_uris.is_empty()
                    || claim.redirect_uris.iter().any(|uri| {
                        !uri.starts_with("https://")
                            || uri.chars().any(char::is_control)
                            || uri.contains('*')
                            || uri.contains('#')
                            || uri
                                .trim_start_matches("https://")
                                .split('/')
                                .next()
                                .is_some_and(|s| s.is_empty() || s.contains('@'))
                    })
                {
                    issue(format!("{path}.redirectURIs"),"OIDC requires explicit HTTPS redirect URIs without credentials, fragments or wildcards");
                }
            }
        }
        for consumer in &claim.consumers {
            let remote = match consumer {
                Consumer::Vm(target) => {
                    if !environment.spec.vms.contains_key(&target.vm) {
                        issue(format!("{path}.consumers"), "unknown VM consumer");
                    }
                    selected.and_then(|p| p.via.as_ref()) != Some(&target.vm)
                }
                Consumer::Container(target) => {
                    match target.container.split_once('/') {
                        Some((vm, container)) => {
                            if !key(container)
                                || !environment.spec.vms.get(vm).is_some_and(|v| {
                                    v.roles.contains(&"docker".into())
                                        && (v.containers.contains_key(container)
                                            || !v.compose_files.is_empty())
                                })
                            {
                                issue(format!("{path}.consumers"),"container consumer needs a declared container or Compose service on a Docker VM");
                            }
                        }
                        None => issue(
                            format!("{path}.consumers"),
                            "container consumer must be vm/container",
                        ),
                    }
                    true
                }
                Consumer::Kubernetes(target) => {
                    let target = &target.kubernetes;
                    if !environment
                        .spec
                        .kubernetes
                        .clusters
                        .contains_key(&target.cluster)
                        || !dns_label(&target.namespace)
                        || !dns_name(&target.secret)
                    {
                        issue(format!("{path}.consumers"), "invalid Kubernetes consumer");
                    }
                    if !secret_targets.insert(format!(
                        "{}/{}/{}",
                        target.cluster, target.namespace, target.secret
                    )) {
                        issue(
                            format!("{path}.consumers"),
                            "Kubernetes binding secret is assigned more than once",
                        );
                    }
                    true
                }
            };
            if remote
                && selected.is_some_and(|p| {
                    p.endpoint.as_deref().is_some_and(loopback)
                        && p.advertise_endpoint.as_deref().is_none_or(loopback)
                })
            {
                issue(path.clone(),"loopback provider requires a non-loopback advertiseEndpoint for remote consumers");
            }
        }
    }
    issues
}
