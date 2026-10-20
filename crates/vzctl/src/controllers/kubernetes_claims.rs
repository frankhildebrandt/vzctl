//! Namespace-local Kubernetes claims are translated into the same controller
//! inputs as YAML claims. Kubernetes input never supplies host paths or consumers.
use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const GROUP: &str = "services.vzctl.dev";
const VERSION: &str = "services.vzctl.dev/v1alpha1";
const MAX_CLAIMS: usize = 256;
const KINDS: [(&str, &str, ServiceKind); 4] = [
    ("IngressClaim", "ingressclaims", ServiceKind::Ingress),
    ("OIDCClaim", "oidcclaims", ServiceKind::Oidc),
    ("PostgresClaim", "postgresclaims", ServiceKind::Postgres),
    ("S3Claim", "s3claims", ServiceKind::S3),
];

#[derive(Clone)]
pub(crate) struct Source {
    pub claim_name: String,
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub generation: u64,
    pub binding_secret: String,
    /// Invalid API claims are reported separately and never reach a provider.
    pub validation_error: Option<(&'static str, String)>,
    plural: String,
    server_vm: String,
    server_ip: String,
}

impl Source {
    pub(super) fn affects_edge(&self) -> bool {
        self.plural == "ingressclaims" || self.plural == "oidcclaims"
    }
}

#[derive(serde::Serialize)]
pub(crate) struct DiscoveryWarning {
    pub cluster: String,
    pub code: &'static str,
    pub message: String,
}
pub(crate) struct DiscoverResult {
    pub environment: crate::config::Environment,
    pub sources: Vec<Source>,
    pub warnings: Vec<DiscoveryWarning>,
}

fn string(max: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":max})
}
fn dns_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
}
fn dns_schema() -> Value {
    json!({"type":"string","minLength":1,"maxLength":253,
        "pattern":"^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$"})
}

/// Structural schemas with a status subresource. Unknown spec fields are
/// retained so the strict importer can reject them rather than silently losing
/// forbidden paths/consumers during API-server pruning.
pub(crate) fn definitions() -> Vec<Value> {
    KINDS
        .iter()
        .map(|(kind, plural, service)| {
            let mut properties = json!({
                "provider":{"type":"string","minLength":1,"maxLength":63,"pattern":"^[A-Za-z0-9_-]+$"},
                "bindingSecret":dns_schema(),
            });
            let required = match service {
                ServiceKind::Postgres => {
                    properties["database"] = json!({"type":"string","minLength":1,"maxLength":63,"pattern":"^[A-Za-z][A-Za-z0-9_]*$"});
                    vec!["database"]
                }
                ServiceKind::S3 => {
                    properties["bucket"] = json!({"type":"string","minLength":3,"maxLength":63,"pattern":"^[a-z0-9][a-z0-9-]*[a-z0-9]$"});
                    vec!["bucket"]
                }
                ServiceKind::Oidc => {
                    properties["clientID"] = string(63);
                    properties["redirectURIs"] = json!({"type":"array","minItems":1,"maxItems":64,"items":string(2048)});
                    properties["scopes"] = json!({"type":"array","maxItems":16,"items":string(128)});
                    vec!["clientID", "redirectURIs"]
                }
                ServiceKind::Ingress => {
                    properties["host"] = dns_schema();
                    properties["backend"] = json!({"type":"object","x-kubernetes-preserve-unknown-fields":true,
                        "required":["kubernetes"],"properties":{"kubernetes":{"type":"object",
                            "x-kubernetes-preserve-unknown-fields":true,"required":["service","port"],
                            "properties":{"service":dns_schema(),"port":{"type":"integer","minimum":1,"maximum":65535}}}}});
                    vec!["host", "backend"]
                }
            };
            let status = json!({"type":"object","properties":{
                "observedGeneration":{"type":"integer","minimum":0},
                "state":{"type":"string","enum":["Pending","Ready","Degraded","Conflict","Retained"]},
                "claim":{"type":"string"},
                "bindingSecret":{"type":"object","properties":{"name":dns_schema(),"namespace":dns_schema()}},
                "conditions":{"type":"array","maxItems":8,"items":{"type":"object",
                    "required":["type","status","reason"],"properties":{
                        "type":{"type":"string"},"status":{"type":"string","enum":["True","False","Unknown"]},
                        "reason":{"type":"string"},"message":{"type":"string"},
                        "observedGeneration":{"type":"integer","minimum":0}}}}
            }});
            let schema=json!({"type":"object","required":["spec"],"properties":{
                "apiVersion":{"type":"string"},"kind":{"type":"string"},"metadata":{"type":"object"},
                "spec":{"type":"object","required":required,"properties":properties,
                    "x-kubernetes-preserve-unknown-fields":true},
                "status":status
            }});
            json!({
                "apiVersion":"apiextensions.k8s.io/v1","kind":"CustomResourceDefinition",
                "metadata":{"name":format!("{plural}.{GROUP}"),"labels":{"app.kubernetes.io/managed-by":"vzctl"}},
                "spec":{"group":GROUP,"scope":"Namespaced",
                    "names":{"plural":plural,"singular":plural.trim_end_matches('s'),"kind":kind},
                    "versions":[{"name":"v1alpha1","served":true,"storage":true,
                        "subresources":{"status":{}},
                        "additionalPrinterColumns":[
                            {"name":"State","type":"string","jsonPath":".status.state"},
                            {"name":"Binding","type":"string","jsonPath":".status.bindingSecret.name"}],
                        "schema":{"openAPIV3Schema":schema}}]
                }
            })
        })
        .collect()
}

fn command(source: &Source) -> Vec<String> {
    vec![
        "sudo".into(),
        "-n".into(),
        "/usr/local/bin/k3s".into(),
        "kubectl".into(),
        "--kubeconfig=/etc/rancher/k3s/k3s.yaml".into(),
        format!("--server=https://{}:6443", source.server_ip),
        "--request-timeout=20s".into(),
    ]
}
fn execute(
    source: &Source,
    args: &[String],
    stdin: &[u8],
    guest: &dyn GuestExec,
) -> ControllerResult<Value> {
    let mut argv = command(source);
    argv.extend_from_slice(args);
    let output = guest
        .run(&source.server_vm, &argv, &BTreeMap::new(), stdin, 30_000)
        .map_err(|_| ControllerError::new("KubernetesUnavailable", "claim API request failed"))?;
    if output.exit_code != 0 || output.truncated || output.stdout.len() > 2 * 1024 * 1024 {
        return Err(ControllerError::new(
            "KubernetesUnavailable",
            "claim API request failed or returned excessive data; remote output redacted",
        ));
    }
    serde_json::from_str(&output.stdout).map_err(|_| {
        ControllerError::new("InvalidResponse", "invalid Kubernetes claim API response")
    })
}

fn invalid() -> ControllerError {
    ControllerError::new("InvalidKubernetesClaim", "claim spec contains unsupported fields or invalid resource metadata; host paths, remote consumers and cross-namespace backends are forbidden")
}
fn import(
    source: &mut Source,
    object: &Value,
    kind: &str,
    service: ServiceKind,
    environment: &crate::config::Environment,
) -> ControllerResult<ClaimConfig> {
    if object["apiVersion"] != VERSION || object["kind"] != kind {
        return Err(invalid());
    }
    let metadata = &object["metadata"];
    source.name = metadata["name"]
        .as_str()
        .filter(|v| dns_name(v))
        .ok_or_else(invalid)?
        .into();
    source.namespace = metadata["namespace"]
        .as_str()
        .filter(|v| v.len() <= 63 && !v.contains('.') && dns_name(v))
        .ok_or_else(invalid)?
        .into();
    source.uid = metadata["uid"]
        .as_str()
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .ok_or_else(invalid)?
        .into();
    source.generation = metadata["generation"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(invalid)?;
    source.claim_name = format!(
        "k8s-{}",
        &format!(
            "{:x}",
            Sha256::digest(format!("{}/{}", source.cluster, source.uid).as_bytes())
        )[..40]
    );
    let mut spec = object["spec"].as_object().ok_or_else(invalid)?.clone();
    let allowed: &[&str] = match service {
        ServiceKind::Postgres => &["provider", "bindingSecret", "database"],
        ServiceKind::S3 => &["provider", "bindingSecret", "bucket"],
        ServiceKind::Oidc => &[
            "provider",
            "bindingSecret",
            "clientID",
            "redirectURIs",
            "scopes",
        ],
        ServiceKind::Ingress => &["provider", "bindingSecret", "host", "backend"],
    };
    if spec.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid());
    }
    source.binding_secret = match spec.remove("bindingSecret") {
        Some(value) => value
            .as_str()
            .filter(|v| dns_name(v))
            .ok_or_else(invalid)?
            .to_string(),
        None => source.name.clone(),
    };
    if service == ServiceKind::Ingress {
        let backend = spec
            .get_mut("backend")
            .and_then(Value::as_object_mut)
            .ok_or_else(invalid)?;
        if backend.len() != 1 {
            return Err(invalid());
        }
        let target = backend
            .get_mut("kubernetes")
            .and_then(Value::as_object_mut)
            .ok_or_else(invalid)?;
        if target.len() != 2 || !target.contains_key("service") || !target.contains_key("port") {
            return Err(invalid());
        }
        target.insert("cluster".into(), json!(source.cluster));
        target.insert("namespace".into(), json!(source.namespace));
    }
    spec.insert(
        "kind".into(),
        serde_json::to_value(service).map_err(|_| invalid())?,
    );
    spec.insert(
        "consumers".into(),
        json!([{"kubernetes":{
            "cluster":source.cluster,"namespace":source.namespace,"secret":source.binding_secret
        }}]),
    );
    let claim: ClaimConfig = serde_json::from_value(Value::Object(spec)).map_err(|_| invalid())?;
    if let Some(provider) = &claim.provider {
        if !environment
            .spec
            .service_providers
            .get(provider)
            .is_some_and(|p| p.kind == service)
        {
            return Err(ControllerError::new(
                "InvalidProvider",
                "Kubernetes claim provider is not in the local stack allowlist",
            ));
        }
    }
    Ok(claim)
}

/// Keys match shared validation: provider identity plus resource name, and
/// namespace-local binding targets. Keep every valid import, including one
/// rejected by an earlier import, so a third duplicate cannot become a winner.
fn conflict_keys(environment: &crate::config::Environment, claim: &ClaimConfig) -> Vec<String> {
    let compatible: Vec<_> = environment
        .spec
        .service_providers
        .iter()
        .filter(|(_, provider)| provider.kind == claim.kind)
        .collect();
    let provider = match &claim.provider {
        Some(name) => environment.spec.service_providers.get(name),
        None if compatible.len() == 1 => Some(compatible[0].1),
        None => None,
    };
    let resource = match claim.kind {
        ServiceKind::Postgres => claim.database.as_deref(),
        ServiceKind::S3 => claim.bucket.as_deref(),
        ServiceKind::Oidc => claim.client_id.as_deref(),
        ServiceKind::Ingress => claim.host.as_deref(),
    };
    let mut keys = Vec::new();
    if let Some(resource) = resource {
        keys.push(
            json!([
                "resource",
                claim.kind,
                provider
                    .and_then(|value| value.via.as_deref())
                    .unwrap_or("stack-edge"),
                provider
                    .and_then(|value| value.endpoint.as_deref())
                    .unwrap_or("stack-edge"),
                resource
            ])
            .to_string(),
        );
    }
    for consumer in &claim.consumers {
        if let Consumer::Kubernetes(consumer) = consumer {
            keys.push(
                json!([
                    "binding",
                    consumer.kubernetes.cluster,
                    consumer.kubernetes.namespace,
                    consumer.kubernetes.secret
                ])
                .to_string(),
            );
        }
    }
    keys
}

pub(crate) fn discover(
    environment: &crate::config::Environment,
    guest: &dyn GuestExec,
) -> ControllerResult<DiscoverResult> {
    if !super::validate_config(environment).is_empty() {
        return Err(ControllerError::new(
            "InvalidConfig",
            "YAML service declarations must be valid before importing Kubernetes claims",
        ));
    }
    let mut projected = environment.clone();
    let mut sources: Vec<Source> = Vec::new();
    let mut imported_owners = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    let mut warnings = Vec::new();
    for (cluster_name, cluster) in &environment.spec.kubernetes.clusters {
        let Some(server) = environment.spec.vms.get(&cluster.server).and_then(|vm| {
            vm.networks
                .iter()
                .find(|network| network.name == cluster.network)
        }) else {
            warnings.push(DiscoveryWarning {
                cluster: cluster_name.clone(),
                code: "InvalidCluster",
                message: "claim discovery requires a Kubernetes server network".into(),
            });
            continue;
        };
        for (kind, plural, service) in KINDS {
            let template = Source {
                claim_name: String::new(),
                cluster: cluster_name.clone(),
                namespace: String::new(),
                name: String::new(),
                uid: String::new(),
                generation: 0,
                binding_secret: String::new(),
                validation_error: None,
                plural: plural.into(),
                server_vm: format!("{}/{}", environment.spec.project, cluster.server),
                server_ip: server.ip.clone(),
            };
            let result = match execute(
                &template,
                &[
                    "get".into(),
                    format!("{plural}.{GROUP}"),
                    "--all-namespaces".into(),
                    "--chunk-size=100".into(),
                    "-o".into(),
                    "json".into(),
                ],
                &[],
                guest,
            ) {
                Ok(result) => result,
                Err(error) => {
                    warnings.push(DiscoveryWarning {
                        cluster: cluster_name.clone(),
                        code: error.code,
                        message: error.message,
                    });
                    break;
                }
            };
            let Some(items) = result["items"].as_array() else {
                warnings.push(DiscoveryWarning {
                    cluster: cluster_name.clone(),
                    code: "InvalidResponse",
                    message: "claim list has no items array".into(),
                });
                break;
            };
            if sources.len() + items.len() > MAX_CLAIMS {
                warnings.push(DiscoveryWarning {
                    cluster: cluster_name.clone(),
                    code: "TooManyClaims",
                    message: "Kubernetes claim discovery exceeds 256 resources".into(),
                });
                break;
            }
            for object in items {
                // Removing a claim retains provisioned data. Never provision new
                // resources for an object which is already terminating.
                if object["metadata"]
                    .get("deletionTimestamp")
                    .is_some_and(|v| !v.is_null())
                {
                    continue;
                }
                let mut source = template.clone();
                let imported = import(&mut source, object, kind, service, environment);
                if source.claim_name.is_empty() {
                    warnings.push(DiscoveryWarning {
                        cluster: cluster_name.clone(),
                        code: "InvalidResponse",
                        message: "claim metadata is invalid; status cannot be safely addressed"
                            .into(),
                    });
                    continue;
                }
                if projected
                    .spec
                    .service_claims
                    .contains_key(&source.claim_name)
                    || sources
                        .iter()
                        .any(|old| old.claim_name == source.claim_name)
                {
                    // A duplicate UID must not let list order select an owner.
                    if let Some(old) = sources
                        .iter_mut()
                        .find(|old| old.claim_name == source.claim_name)
                    {
                        old.validation_error = Some((
                            "DuplicateClaim",
                            "Kubernetes claim identity is duplicated".into(),
                        ));
                        projected.spec.service_claims.remove(&source.claim_name);
                    }
                    source.validation_error = Some((
                        "DuplicateClaim",
                        "Kubernetes claim identity collides with another declaration".into(),
                    ));
                } else {
                    match imported {
                        Ok(claim) => {
                            // YAML declarations have explicit precedence. Only imports
                            // valid against YAML alone participate in peer conflicts.
                            let mut standalone = environment.clone();
                            standalone
                                .spec
                                .service_claims
                                .insert(source.claim_name.clone(), claim.clone());
                            if super::validate_config(&standalone).is_empty() {
                                for key in conflict_keys(environment, &claim) {
                                    imported_owners
                                        .entry(key)
                                        .or_default()
                                        .insert(source.claim_name.clone());
                                }
                            }
                            let mut candidate = projected.clone();
                            candidate
                                .spec
                                .service_claims
                                .insert(source.claim_name.clone(), claim);
                            if super::validate_config(&candidate).is_empty() {
                                projected = candidate;
                            } else {
                                source.validation_error=Some(("InvalidKubernetesClaim","claim conflicts with the stack or references an invalid provider/backend".into()));
                            }
                        }
                        Err(error) => source.validation_error = Some((error.code, error.message)),
                    }
                }
                sources.push(source);
            }
        }
    }
    let duplicates = imported_owners
        .values()
        .filter(|owners| owners.len() > 1)
        .flat_map(|owners| owners.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    for source in &mut sources {
        if duplicates.contains(&source.claim_name) {
            source.validation_error = Some((
                "DuplicateClaim",
                "Kubernetes claims declare the same provider resource or binding target".into(),
            ));
            projected.spec.service_claims.remove(&source.claim_name);
        }
    }
    Ok(DiscoverResult {
        environment: projected,
        sources,
        warnings,
    })
}

/// A fixed reason allowlist prevents provider text, URLs or credentials supplied
/// in a record from ever becoming Kubernetes status. Details stay in redacted CLI
/// diagnostics. The status contains a Secret reference, never its values.
fn status(source: &Source, record: &Value) -> Value {
    let state = match record["state"].as_str() {
        Some("Ready") => "Ready",
        Some("Conflict") => "Conflict",
        Some("Pending") => "Pending",
        Some("Retained") => "Retained",
        _ => "Degraded",
    };
    let reason = match record["conditions"][0]["reason"].as_str().unwrap_or("") {
        value @ ("Reconciled"
        | "OwnershipConflict"
        | "OwnershipUncertain"
        | "ProviderUnavailable"
        | "ProviderTimeout"
        | "CredentialCheckFailed"
        | "PrivilegeDrift"
        | "InvalidProvider"
        | "UnsupportedProvider"
        | "BindingDeliveryFailed"
        | "KubernetesUnavailable"
        | "BackendPending"
        | "IngressUnavailable"
        | "OidcUnavailable"
        | "OutOfDate"
        | "InvalidKubernetesClaim"
        | "InvalidClaim"
        | "DuplicateClaim") => value,
        _ if state == "Ready" => "Reconciled",
        _ => "ReconcileFailed",
    };
    let mut result = json!({
        "observedGeneration":source.generation,"state":state,"claim":source.claim_name,
        "conditions":[{"type":"Ready","status":if state=="Ready"{"True"}else{"False"},
            "reason":reason,"observedGeneration":source.generation,
            "message":if state=="Ready"{"Service provisioned and binding delivered"}else{"Service not ready; inspect redacted controller diagnostics"}}]
    });
    if state == "Ready" {
        result["bindingSecret"] =
            json!({"name":source.binding_secret,"namespace":source.namespace});
    }
    result
}

pub(crate) fn update_status(
    source: &Source,
    record: &Value,
    guest: &dyn GuestExec,
) -> ControllerResult<()> {
    let desired = status(source, record);
    let args = [
        "get".into(),
        format!("{}.{}", source.plural, GROUP),
        source.name.clone(),
        format!("--namespace={}", source.namespace),
        "-o".into(),
        "json".into(),
    ];
    let current = execute(source, &args, &[], guest)?;
    if current["metadata"]["uid"] != source.uid
        || current["metadata"]["generation"] != source.generation
    {
        return Err(ControllerError::new(
            "ClaimChanged",
            "claim identity or generation changed during reconciliation; retry discovery",
        ));
    }
    if current["status"] == desired {
        return Ok(());
    }
    let patch = json!([
        {"op":"test","path":"/metadata/uid","value":source.uid},
        {"op":"test","path":"/metadata/generation","value":source.generation},
        {"op":"add","path":"/status","value":desired}
    ]);
    execute(
        source,
        &[
            "patch".into(),
            format!("{}.{}", source.plural, GROUP),
            source.name.clone(),
            format!("--namespace={}", source.namespace),
            "--subresource=status".into(),
            "--type=json".into(),
            "--patch-file=/dev/stdin".into(),
            "-o".into(),
            "json".into(),
        ],
        &serde_json::to_vec(&patch).map_err(|_| invalid())?,
        guest,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Guest {
        replies: RefCell<VecDeque<Value>>,
        writes: RefCell<Vec<Value>>,
    }
    impl GuestExec for Guest {
        fn run(
            &self,
            vm: &str,
            argv: &[String],
            _: &BTreeMap<String, String>,
            stdin: &[u8],
            _: u64,
        ) -> ControllerResult<ExecOutput> {
            assert_eq!(vm, "kube-lab/kube-server");
            assert!(argv.contains(&"--server=https://10.90.0.10:6443".into()));
            if !stdin.is_empty() {
                assert!(argv.contains(&"--subresource=status".into()));
                self.writes
                    .borrow_mut()
                    .push(serde_json::from_slice(stdin).unwrap());
            }
            let value = self
                .replies
                .borrow_mut()
                .pop_front()
                .expect("unexpected API call");
            Ok(ExecOutput {
                exit_code: 0,
                stdout: value.to_string(),
                stderr: "do-not-leak-secret".into(),
                truncated: false,
            })
        }
    }
    fn guest(replies: Vec<Value>) -> Guest {
        Guest {
            replies: RefCell::new(replies.into()),
            writes: RefCell::new(Vec::new()),
        }
    }
    fn environment() -> crate::config::Environment {
        let mut env: crate::config::Environment = serde_yaml::from_str(include_str!(
            "../../tests/fixtures/validate/valid-kubernetes.yaml"
        ))
        .unwrap();
        env.spec.service_providers=serde_json::from_value(json!({
            "database":{"kind":"postgres","via":"kube-server","endpoint":"127.0.0.1:5432",
                "advertiseEndpoint":"db.lan.example.test:5432","adminUser":"postgres","passwordFile":"admin"},
            "objects":{"kind":"s3","via":"kube-server","endpoint":"http://127.0.0.1:9000",
                "advertiseEndpoint":"http://objects.lan.example.test:9000","implementation":"minio",
                "accessKeyFile":"access","secretKeyFile":"secret"},
        })).unwrap();
        env.spec.ingress = Some(serde_json::from_value(json!({"enabled":true})).unwrap());
        env.spec.oidc = Some(
            serde_json::from_value(
                json!({"enabled":true,"issuer":"https://auth.svc.kube-lab.vz.test"}),
            )
            .unwrap(),
        );
        env
    }
    fn object(kind: &str, name: &str, spec: Value) -> Value {
        json!({"apiVersion":VERSION,"kind":kind,"metadata":{
            "name":name,"namespace":"app","uid":format!("uid-{name}"),"generation":1},"spec":spec})
    }
    fn source() -> Source {
        Source {
            claim_name: String::new(),
            cluster: "dev".into(),
            namespace: String::new(),
            name: String::new(),
            uid: String::new(),
            generation: 0,
            binding_secret: String::new(),
            plural: "postgresclaims".into(),
            validation_error: None,
            server_vm: "kube-lab/kube-server".into(),
            server_ip: "10.90.0.10".into(),
        }
    }
    #[test]
    fn definitions_have_structural_spec_and_preserve_forbidden_fields_for_rejection() {
        let definitions = definitions();
        assert_eq!(definitions.len(), 4);
        for crd in definitions {
            assert_eq!(crd["spec"]["scope"], "Namespaced");
            let version = &crd["spec"]["versions"][0];
            assert!(version["subresources"]["status"].is_object());
            assert_eq!(
                version["schema"]["openAPIV3Schema"]["properties"]["spec"]
                    ["x-kubernetes-preserve-unknown-fields"],
                true
            );
            assert!(
                version["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"]
                    .get("consumers")
                    .is_none()
            );
            assert!(
                version["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"]
                    .get("passwordFile")
                    .is_none()
            );
        }
    }
    #[test]
    fn discovery_projects_all_four_claims_and_keeps_existing_yaml_claims() {
        let mut env = environment();
        env.spec.service_claims.insert(
            "yaml-db".into(),
            serde_json::from_value(
                json!({"kind":"postgres","database":"yaml_app","provider":"database"}),
            )
            .unwrap(),
        );
        let guest = guest(vec![
            json!({"items":[object("IngressClaim","web",json!({"host":"web.svc.kube-lab.vz.test","backend":{"kubernetes":{"service":"web","port":80}}}))]}),
            json!({"items":[object("OIDCClaim","login",json!({"clientID":"app","redirectURIs":["https://web.svc.kube-lab.vz.test/callback"]}))]}),
            json!({"items":[object("PostgresClaim","database",json!({"database":"app","bindingSecret":"app-db"}))]}),
            json!({"items":[object("S3Claim","files",json!({"bucket":"app-files"}))]}),
        ]);
        let discovered = discover(&env, &guest).unwrap();
        assert!(discovered.warnings.is_empty());
        let (projected, sources) = (discovered.environment, discovered.sources);
        assert_eq!(projected.spec.service_claims.len(), 5);
        assert!(projected.spec.service_claims.contains_key("yaml-db"));
        assert_eq!(
            env.spec.service_claims.len(),
            1,
            "input environment remains unchanged"
        );
        assert_eq!(sources.len(), 4);
        for source in sources {
            let claim = &projected.spec.service_claims[&source.claim_name];
            assert!(source.claim_name.starts_with("k8s-"));
            assert_eq!(source.claim_name.len(), 44);
            let Consumer::Kubernetes(consumer) = &claim.consumers[0] else {
                panic!("consumer must be namespace-local")
            };
            assert_eq!(consumer.kubernetes.namespace, "app");
            assert_eq!(consumer.kubernetes.cluster, "dev");
            assert_eq!(consumer.kubernetes.secret, source.binding_secret);
            if let Some(Backend::Kubernetes(backend)) = &claim.backend {
                assert_eq!(backend.kubernetes.namespace, "app");
                assert_eq!(backend.kubernetes.cluster, "dev");
            }
        }
        assert!(guest.writes.borrow().is_empty(), "discovery is read-only");
    }
    #[test]
    fn conflicting_kubernetes_resources_have_no_api_order_winner() {
        for reverse in [false, true] {
            let mut env = environment();
            env.spec.service_providers.insert(
                "alias".into(),
                env.spec.service_providers["database"].clone(),
            );
            let mut objects = vec![
                object(
                    "PostgresClaim",
                    "first",
                    json!({"database":"same_app","provider":"database"}),
                ),
                object(
                    "PostgresClaim",
                    "second",
                    json!({"database":"same_app","provider":"alias"}),
                ),
                object(
                    "PostgresClaim",
                    "third",
                    json!({"database":"same_app","provider":"database"}),
                ),
                object(
                    "PostgresClaim",
                    "unrelated",
                    json!({"database":"other_app","provider":"database"}),
                ),
            ];
            if reverse {
                objects.reverse();
            }
            let api = guest(vec![
                json!({"items":[]}),
                json!({"items":[]}),
                json!({"items":objects}),
                json!({"items":[]}),
            ]);
            let discovered = discover(&env, &api).unwrap();
            assert_eq!(discovered.environment.spec.service_claims.len(), 1);
            assert_eq!(
                discovered
                    .sources
                    .iter()
                    .filter(|source| source
                        .validation_error
                        .as_ref()
                        .is_some_and(|(code, _)| *code == "DuplicateClaim"))
                    .count(),
                3
            );
            let unrelated = discovered
                .sources
                .iter()
                .find(|source| source.name == "unrelated")
                .unwrap();
            assert!(unrelated.validation_error.is_none());
            assert!(discovered
                .environment
                .spec
                .service_claims
                .contains_key(&unrelated.claim_name));
            assert!(api.writes.borrow().is_empty());
        }
    }

    #[test]
    fn yaml_claim_keeps_precedence_over_conflicting_kubernetes_claims() {
        let mut env = environment();
        env.spec.service_claims.insert(
            "yaml-db".into(),
            serde_json::from_value(
                json!({"kind":"postgres","database":"app","provider":"database"}),
            )
            .unwrap(),
        );
        let api = guest(vec![
            json!({"items":[]}),
            json!({"items":[]}),
            json!({"items":[
                object("PostgresClaim","first",json!({"database":"app"})),
                object("PostgresClaim","second",json!({"database":"app"})),
            ]}),
            json!({"items":[]}),
        ]);
        let discovered = discover(&env, &api).unwrap();
        assert_eq!(discovered.environment.spec.service_claims.len(), 1);
        assert!(discovered
            .environment
            .spec
            .service_claims
            .contains_key("yaml-db"));
        assert!(discovered
            .sources
            .iter()
            .all(|source| source.validation_error.is_some()));
    }

    #[test]
    fn cross_kind_secret_target_collision_invalidates_both_imports() {
        let env = environment();
        let api = guest(vec![
            json!({"items":[]}),
            json!({"items":[]}),
            json!({"items":[
                object("PostgresClaim","database",json!({"database":"app","bindingSecret":"shared"})),
            ]}),
            json!({"items":[
                object("S3Claim","files",json!({"bucket":"app-files","bindingSecret":"shared"})),
            ]}),
        ]);
        let discovered = discover(&env, &api).unwrap();
        assert!(discovered.environment.spec.service_claims.is_empty());
        assert!(discovered.sources.iter().all(|source| source
            .validation_error
            .as_ref()
            .is_some_and(|(code, _)| *code == "DuplicateClaim")));
    }

    #[test]
    fn import_rejects_host_paths_consumers_and_cross_namespace_backends() {
        let env = environment();
        for spec in [
            json!({"database":"app","passwordFile":"/host/secret"}),
            json!({"database":"app","kind":"postgres"}),
            json!({"database":"app","consumers":[{"vm":"kube-server"}]}),
            json!({"database":"app","provider":"another-stack"}),
        ] {
            assert!(import(
                &mut source(),
                &object("PostgresClaim", "db", spec),
                "PostgresClaim",
                ServiceKind::Postgres,
                &env
            )
            .is_err());
        }
        for backend in [
            json!({"vm":"kube-server","port":80}),
            json!({"kubernetes":{"service":"web","port":80,"namespace":"other"}}),
            json!({"kubernetes":{"service":"web","port":80,"cluster":"other"}}),
        ] {
            assert!(import(
                &mut source(),
                &object(
                    "IngressClaim",
                    "web",
                    json!({"host":"app.test","backend":backend})
                ),
                "IngressClaim",
                ServiceKind::Ingress,
                &env
            )
            .is_err());
        }
    }
    #[test]
    fn recreated_resource_gets_a_new_claim_owner() {
        let env = environment();
        let mut first = source();
        let mut second = source();
        let mut resource = object("PostgresClaim", "db", json!({"database":"app"}));
        import(
            &mut first,
            &resource,
            "PostgresClaim",
            ServiceKind::Postgres,
            &env,
        )
        .unwrap();
        resource["metadata"]["uid"] = json!("replacement-uid");
        import(
            &mut second,
            &resource,
            "PostgresClaim",
            ServiceKind::Postgres,
            &env,
        )
        .unwrap();
        assert_ne!(
            first.claim_name, second.claim_name,
            "same name must not adopt the old object's database"
        );
    }
    #[test]
    fn status_is_secret_free_unchanged_is_noop_and_replacement_is_rejected() {
        let mut source = source();
        let resource = object("PostgresClaim", "db", json!({"database":"app"}));
        import(
            &mut source,
            &resource,
            "PostgresClaim",
            ServiceKind::Postgres,
            &environment(),
        )
        .unwrap();
        let record = json!({"state":"Ready","binding":{"password":"do-not-leak-secret"},
            "conditions":[{"reason":"do-not-leak-secret","message":"do-not-leak-secret"}]});
        let api = guest(vec![resource.clone(), json!({})]);
        update_status(&source, &record, &api).unwrap();
        let writes = api.writes.borrow();
        assert_eq!(writes.len(), 1);
        assert!(!writes[0].to_string().contains("do-not-leak-secret"));
        assert_eq!(
            writes[0][0],
            json!({"op":"test","path":"/metadata/uid","value":"uid-db"})
        );
        assert_eq!(
            writes[0][1],
            json!({"op":"test","path":"/metadata/generation","value":1})
        );
        assert_eq!(
            writes[0][2]["value"]["bindingSecret"],
            json!({"name":"db","namespace":"app"})
        );
        let mut unchanged = resource.clone();
        unchanged["status"] = status(&source, &record);
        let api = guest(vec![unchanged]);
        update_status(&source, &record, &api).unwrap();
        assert!(api.writes.borrow().is_empty());
        for (key, value) in [("uid", json!("replacement")), ("generation", json!(2))] {
            let mut changed = resource.clone();
            changed["metadata"][key] = value;
            let api = guest(vec![changed]);
            assert_eq!(
                update_status(&source, &record, &api).unwrap_err().code,
                "ClaimChanged"
            );
            assert!(api.writes.borrow().is_empty());
        }
    }
    #[test]
    fn discovery_marks_duplicate_identity_and_bounds_excessive_results() {
        let env = environment();
        let object = object(
            "IngressClaim",
            "web",
            json!({"host":"web.test","backend":{"kubernetes":{"service":"web","port":80}}}),
        );
        let api = guest(vec![
            json!({"items":[object.clone(),object.clone()]}),
            json!({"items":[]}),
            json!({"items":[]}),
            json!({"items":[]}),
        ]);
        let discovered = discover(&env, &api).unwrap();
        assert!(discovered.environment.spec.service_claims.is_empty());
        assert_eq!(discovered.sources.len(), 2);
        assert!(discovered
            .sources
            .iter()
            .all(|s| s.validation_error.as_ref().unwrap().0 == "DuplicateClaim"));
        let api = guest(vec![json!({"items":vec![object;257]})]);
        let discovered = discover(&env, &api).unwrap();
        assert_eq!(discovered.warnings[0].code, "TooManyClaims");
        assert!(discovered.sources.is_empty());
    }
    #[test]
    fn invalid_claim_and_cluster_discovery_failure_do_not_block_other_services() {
        let mut env = environment();
        env.spec.service_claims.insert(
            "yaml-db".into(),
            serde_json::from_value(
                json!({"kind":"postgres","database":"yaml_app","provider":"database"}),
            )
            .unwrap(),
        );
        let api = guest(vec![
            json!({"items":[
                object("IngressClaim","bad",json!({"host":"web.test","backend":{"vm":"kube-server","port":80}})),
                object("IngressClaim","good",json!({"host":"good.test","backend":{"kubernetes":{"service":"web","port":80}}}))
            ]}),
            json!({"not":"an API list"}),
        ]);
        let result = discover(&env, &api).unwrap();
        assert_eq!(
            result.environment.spec.service_claims.len(),
            2,
            "YAML plus valid API claim remain runnable"
        );
        assert_eq!(result.sources.len(), 2);
        assert!(result.sources[0].validation_error.is_some());
        assert!(result.sources[1].validation_error.is_none());
        assert_eq!(result.warnings.len(), 1);
        assert!(!serde_json::to_string(&result.warnings)
            .unwrap()
            .contains("do-not-leak-secret"));
    }
}
