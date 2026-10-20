//! Adapters to the existing stack edge. YAML remains the source of truth.
use super::storage::{digest, read_private};
use super::*;
use serde_json::{json, Value};

type EdgeApply<'a> = dyn Fn(
        &crate::config::Environment,
        ServiceKind,
        &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String>
    + 'a;
pub(super) fn reconcile(
    environment: &crate::config::Environment,
    config_file: &Path,
    socket: &Path,
    state: &Path,
    guest: &dyn GuestExec,
    complete_inventory: bool,
) -> (
    BTreeMap<String, ControllerResult<ReconcileResult>>,
    Vec<ControllerError>,
) {
    let apply =
        |projected: &crate::config::Environment, kind, guard: &dyn Fn() -> Result<(), String>| {
            crate::reconciler::reconcile_claim_edge(projected, config_file, socket, kind, guard)
        };
    reconcile_with_edge(environment, state, guest, complete_inventory, &apply)
}
fn reconcile_with_edge(
    environment: &crate::config::Environment,
    state: &Path,
    guest: &dyn GuestExec,
    complete_inventory: bool,
    apply: &EdgeApply<'_>,
) -> (
    BTreeMap<String, ControllerResult<ReconcileResult>>,
    Vec<ControllerError>,
) {
    let mut results = BTreeMap::new();
    let mut warnings = Vec::new();
    if !complete_inventory {
        for (name, claim) in &environment.spec.service_claims {
            if matches!(claim.kind, ServiceKind::Ingress | ServiceKind::Oidc) {
                results.insert(
                    name.clone(),
                    Err(ControllerError::new(
                        "DiscoveryIncomplete",
                        "edge configuration retained until complete claim inventory is available",
                    )),
                );
            }
        }
        warnings.push(ControllerError::new(
            "DiscoveryIncomplete",
            "edge configuration retained until complete claim inventory is available",
        ));
        return (results, warnings);
    }
    let permission = || guest.check_permission().map_err(|error| error.to_string());
    if let Err(error) = guest.check_permission() {
        warnings.push(error);
        return (results, warnings);
    }
    let mut projected = environment.clone();
    let mut routes = BTreeMap::new();
    for (name, claim) in &environment.spec.service_claims {
        if claim.kind != ServiceKind::Ingress {
            continue;
        }
        let result: ControllerResult<()> = (|| {
            let host = claim
                .host
                .as_ref()
                .ok_or_else(|| ControllerError::new("InvalidClaim", "ingress host missing"))?;
            let backend =
                match claim.backend.as_ref().ok_or_else(|| {
                    ControllerError::new("InvalidClaim", "ingress backend missing")
                })? {
                    Backend::Vm(backend) => format!("{}:{}", backend.vm, backend.port),
                    Backend::Kubernetes(backend) => {
                        node_port(environment, name, &backend.kubernetes, guest)?
                    }
                };
            let route = crate::config::IngressRoute {
                host: host.clone(),
                to: backend,
                requires: Vec::new(),
            };
            projected
                .spec
                .ingress
                .as_mut()
                .filter(|i| i.enabled)
                .ok_or_else(|| {
                    ControllerError::new("InvalidProvider", "stack ingress is disabled")
                })?
                .routes
                .push(route.clone());
            routes.insert(name.clone(), route);
            Ok(())
        })();
        if let Err(error) = result {
            // A temporary Kubernetes outage must not remove an already published route.
            // Reuse only this claim's last observed backend for its unchanged host.
            let path = state
                .join("controllers")
                .join(digest(&environment.spec.project))
                .join(format!("{}.json", digest(name)));
            if matches!(error.code, "KubernetesUnavailable" | "BackendPending") {
                if let Ok(bytes) = read_private(&path) {
                    if let Ok(previous) = serde_json::from_slice::<Value>(&bytes) {
                        let host = previous["resource"]["host"].as_str();
                        let backend = previous["resource"]["backend"].as_str();
                        if host == claim.host.as_deref() {
                            if let (Some(host), Some(backend)) = (host, backend) {
                                if backend.split_once(':').is_some_and(|(vm, port)| {
                                    environment.spec.vms.contains_key(vm)
                                        && port.parse::<u16>().is_ok_and(|port| port > 0)
                                }) {
                                    if let Some(ingress) = projected.spec.ingress.as_mut() {
                                        ingress.routes.push(crate::config::IngressRoute {
                                            host: host.into(),
                                            to: backend.into(),
                                            requires: Vec::new(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            results.insert(name.clone(), Err(error));
        }
    }
    if let Err(error) = guest.check_permission() {
        warnings.push(error);
        return (results, warnings);
    }
    if projected
        .spec
        .ingress
        .as_ref()
        .is_some_and(|ingress| ingress.enabled)
    {
        let applied = apply(&projected, ServiceKind::Ingress, &permission);
        if applied.is_err() {
            warnings.push(ControllerError::new(
                "IngressUnavailable",
                "stack ingress reconciliation failed; inspect edge health",
            ));
        }
        for (name, route) in routes {
            let result = match &applied {
                Ok(()) => Ok(ReconcileResult {
                    changed: false,
                    binding: Binding {
                        public: BTreeMap::from([
                            (
                                "url".into(),
                                format!(
                                    "https://{}{}",
                                    route.host,
                                    environment
                                        .spec
                                        .ingress
                                        .as_ref()
                                        .map(|ingress| ingress.https_port)
                                        .filter(|port| *port != 443)
                                        .map(|port| format!(":{port}"))
                                        .unwrap_or_default()
                                ),
                            ),
                            (
                                "caFile".into(),
                                crate::certs::ca_cert_path(state).display().to_string(),
                            ),
                        ]),
                        secrets: BTreeMap::new(),
                    },
                    resource: json!({"host":route.host,"backend":route.to}),
                }),
                Err(_) => Err(ControllerError::new(
                    "IngressUnavailable",
                    "stack ingress reconciliation failed; inspect edge health",
                )),
            };
            results.insert(name, result);
        }
    }
    if let Err(error) = guest.check_permission() {
        warnings.push(error);
        return (results, warnings);
    }
    if environment
        .spec
        .service_claims
        .values()
        .any(|claim| claim.kind == ServiceKind::Oidc)
    {
        let applied = apply(environment, ServiceKind::Oidc, &permission);
        let clients = if applied.is_ok() {
            read_private(&crate::oidc::clients_path(state, &environment.spec.project)).and_then(
                |bytes| {
                    serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                        ControllerError::new("InvalidState", "OIDC client state is invalid")
                    })
                },
            )
        } else {
            Err(ControllerError::new(
                "OidcUnavailable",
                "stack OIDC reconciliation failed; inspect edge health",
            ))
        };
        for (name, claim) in &environment.spec.service_claims {
            if claim.kind != ServiceKind::Oidc {
                continue;
            }
            let result = (|| {
                let clients = clients
                    .as_ref()
                    .map_err(|error| ControllerError::new(error.code, &error.message))?;
                let id = claim
                    .client_id
                    .as_deref()
                    .ok_or_else(|| ControllerError::new("InvalidClaim", "clientID missing"))?;
                let owner = format!("{}/{name}", environment.spec.project);
                let client = clients["clients"]
                    .as_array()
                    .and_then(|items| items.iter().find(|client| client["id"] == id))
                    .filter(|client| client["vzctlOwner"] == owner)
                    .ok_or_else(|| {
                        ControllerError::new(
                            "OwnershipConflict",
                            "OIDC client is not owned by this claim",
                        )
                    })?;
                let secret = client["secret"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ControllerError::new("InvalidState", "OIDC client secret is missing")
                    })?;
                let issuer = environment
                    .spec
                    .oidc
                    .as_ref()
                    .ok_or_else(|| ControllerError::new("InvalidProvider", "stack OIDC disabled"))?
                    .issuer
                    .clone();
                Ok(ReconcileResult {
                    changed: false,
                    binding: Binding {
                        public: BTreeMap::from([
                            ("issuer".into(), issuer),
                            ("clientID".into(), id.into()),
                            ("scopes".into(), "openid profile email".into()),
                        ]),
                        secrets: BTreeMap::from([("clientSecret".into(), secret.into())]),
                    },
                    resource: json!({"clientID":id,"redirectURIs":claim.redirect_uris}),
                })
            })();
            results.insert(name.clone(), result);
        }
    }
    (results, warnings)
}

fn node_port(
    environment: &crate::config::Environment,
    name: &str,
    backend: &KubernetesService,
    guest: &dyn GuestExec,
) -> ControllerResult<String> {
    let cluster = environment
        .spec
        .kubernetes
        .clusters
        .get(&backend.cluster)
        .ok_or_else(|| ControllerError::new("InvalidBackend", "Kubernetes cluster missing"))?;
    let server = environment
        .spec
        .vms
        .get(&cluster.server)
        .and_then(|vm| vm.networks.iter().find(|net| net.name == cluster.network))
        .ok_or_else(|| ControllerError::new("InvalidBackend", "Kubernetes node address missing"))?;
    let vm = format!("{}/{}", environment.spec.project, cluster.server);
    let prefix = vec![
        "sudo".into(),
        "-n".into(),
        "/usr/local/bin/k3s".into(),
        "kubectl".into(),
        "--kubeconfig=/etc/rancher/k3s/k3s.yaml".into(),
        format!("--server=https://{}:6443", server.ip),
        format!("--namespace={}", backend.namespace),
        "--request-timeout=20s".into(),
    ];
    let execute = |args: &[&str], input: &[u8]| -> ControllerResult<Value> {
        let mut command = prefix.clone();
        command.extend(args.iter().map(|v| v.to_string()));
        let output = guest.run(&vm, &command, &BTreeMap::new(), input, 30_000)?;
        if output.exit_code != 0 || output.truncated {
            return Err(ControllerError::new(
                "KubernetesUnavailable",
                "Kubernetes ingress service operation failed",
            ));
        }
        if output.stdout.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&output.stdout).map_err(|_| {
            ControllerError::new("InvalidResponse", "Kubernetes service response is invalid")
        })
    };
    let source = execute(&["get", "service", &backend.service, "-o", "json"], &[])?;
    let selector = source["spec"]["selector"]
        .as_object()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ControllerError::new(
                "UnsupportedBackend",
                "ingress requires a Service with a Pod selector",
            )
        })?;
    let source_port = source["spec"]["ports"]
        .as_array()
        .and_then(|ports| {
            ports.iter().find(|port| {
                port["port"] == backend.port && port["protocol"].as_str().unwrap_or("TCP") == "TCP"
            })
        })
        .ok_or_else(|| ControllerError::new("InvalidBackend", "Service TCP port missing"))?;
    let owner = format!("vzctl:{}/{name}", environment.spec.project);
    let resource_name = format!("vzctl-{}", &digest(&owner)[..20]);
    let old = execute(
        &[
            "get",
            "service",
            &resource_name,
            "--ignore-not-found",
            "-o",
            "json",
        ],
        &[],
    )?;
    if !old.is_null() && old["metadata"]["annotations"]["services.vzctl.dev/owner"] != owner {
        return Err(ControllerError::new(
            "OwnershipConflict",
            "ingress NodePort Service belongs to another owner",
        ));
    }
    let mut port = json!({"name":"http","protocol":"TCP","port":backend.port,"targetPort":source_port.get("targetPort").unwrap_or(&source_port["port"])});
    if let Some(number) = old["spec"]["ports"][0]["nodePort"].as_u64() {
        port["nodePort"] = json!(number);
    }
    let mut desired = json!({"apiVersion":"v1","kind":"Service","metadata":{"name":resource_name,"namespace":backend.namespace,
        "annotations":{"services.vzctl.dev/owner":owner},"labels":{"app.kubernetes.io/managed-by":"vzctl"}},
        "spec":{"type":"NodePort","selector":selector,"ports":[port],"externalTrafficPolicy":"Cluster"}});
    let current = if !old.is_null()
        && old["spec"]["selector"] == desired["spec"]["selector"]
        && old["spec"]["ports"] == desired["spec"]["ports"]
        && old["spec"]["type"] == "NodePort"
        && old["spec"]["externalTrafficPolicy"] == "Cluster"
    {
        old
    } else {
        if !old.is_null() {
            desired["metadata"]["resourceVersion"] = old["metadata"]["resourceVersion"].clone();
            desired["spec"]["clusterIP"] = old["spec"]["clusterIP"].clone();
            if let Some(ips) = old["spec"].get("clusterIPs") {
                desired["spec"]["clusterIPs"] = ips.clone();
            }
        }
        let bytes = serde_json::to_vec(&desired)
            .map_err(|_| ControllerError::new("InvalidBackend", "service cannot be encoded"))?;
        execute(
            &[
                if old.is_null() { "create" } else { "replace" },
                "-f",
                "-",
                "-o",
                "json",
                "--field-manager=vzctl-services",
            ],
            &bytes,
        )?
    };
    let port = current["spec"]["ports"][0]["nodePort"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= 65535)
        .ok_or_else(|| {
            ControllerError::new("BackendPending", "Kubernetes has not allocated a NodePort")
        })?;
    Ok(format!("{}:{port}", cluster.server))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Guest {
        responses: RefCell<VecDeque<Value>>,
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
            assert!(argv
                .iter()
                .any(|arg| arg == "--server=https://10.90.0.10:6443"));
            if !stdin.is_empty() {
                self.writes
                    .borrow_mut()
                    .push(serde_json::from_slice(stdin).unwrap());
            }
            let response = self
                .responses
                .borrow_mut()
                .pop_front()
                .expect("unexpected Kubernetes write");
            Ok(ExecOutput {
                exit_code: 0,
                stdout: response.to_string(),
                stderr: String::new(),
                truncated: false,
            })
        }
    }
    fn source() -> Value {
        json!({"spec":{"selector":{"app":"web"},"ports":[{"port":80,"targetPort":8080,"protocol":"TCP"}]}})
    }
    fn managed(policy: &str) -> Value {
        json!({
            "metadata":{"resourceVersion":"12","annotations":{"services.vzctl.dev/owner":"vzctl:kube-lab/web"}},
            "spec":{"type":"NodePort","clusterIP":"10.43.0.15","clusterIPs":["10.43.0.15"],
            "selector":{"app":"web"},"externalTrafficPolicy":policy,
            "ports":[{"name":"http","protocol":"TCP","port":80,"targetPort":8080,"nodePort":31080}]}
        })
    }
    fn fixture() -> (crate::config::Environment, KubernetesService) {
        (
            serde_yaml::from_str(include_str!(
                "../../tests/fixtures/validate/valid-kubernetes.yaml"
            ))
            .unwrap(),
            KubernetesService {
                cluster: "dev".into(),
                namespace: "app".into(),
                service: "web".into(),
                port: 80,
            },
        )
    }
    #[test]
    fn nodeport_repairs_local_traffic_policy_and_preserves_allocated_addresses() {
        let (env, backend) = fixture();
        let guest = Guest {
            responses: RefCell::new(VecDeque::from([
                source(),
                managed("Local"),
                managed("Cluster"),
            ])),
            writes: RefCell::new(Vec::new()),
        };
        assert_eq!(
            node_port(&env, "web", &backend, &guest).unwrap(),
            "kube-server:31080"
        );
        let writes = guest.writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0]["spec"]["externalTrafficPolicy"], "Cluster");
        assert_eq!(writes[0]["spec"]["clusterIP"], "10.43.0.15");
        assert_eq!(writes[0]["spec"]["ports"][0]["nodePort"], 31080);
        assert_eq!(writes[0]["metadata"]["resourceVersion"], "12");
    }
    #[test]
    fn unchanged_nodeport_does_not_write_and_foreign_service_is_not_adopted() {
        let (env, backend) = fixture();
        let guest = Guest {
            responses: RefCell::new(VecDeque::from([source(), managed("Cluster")])),
            writes: RefCell::new(Vec::new()),
        };
        assert_eq!(
            node_port(&env, "web", &backend, &guest).unwrap(),
            "kube-server:31080"
        );
        assert!(guest.writes.borrow().is_empty());
        let mut foreign = managed("Cluster");
        foreign["metadata"]["annotations"]["services.vzctl.dev/owner"] = json!("another-owner");
        let guest = Guest {
            responses: RefCell::new(VecDeque::from([source(), foreign])),
            writes: RefCell::new(Vec::new()),
        };
        assert_eq!(
            node_port(&env, "web", &backend, &guest).unwrap_err().code,
            "OwnershipConflict"
        );
        assert!(guest.writes.borrow().is_empty());
    }
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;
    use std::cell::Cell;
    struct RevokedGuest {
        reads: Cell<u32>,
    }
    impl GuestExec for RevokedGuest {
        fn check_permission(&self) -> ControllerResult<()> {
            if self.reads.get() > 0 {
                Err(ControllerError::new("RegistrationRevoked", "disabled"))
            } else {
                Ok(())
            }
        }
        fn run(
            &self,
            _: &str,
            _: &[String],
            _: &BTreeMap<String, String>,
            _: &[u8],
            _: u64,
        ) -> ControllerResult<ExecOutput> {
            self.reads.set(self.reads.get() + 1);
            Err(ControllerError::new(
                "RegistrationRevoked",
                "disabled during Kubernetes read",
            ))
        }
    }
    fn environment() -> crate::config::Environment {
        let mut environment: crate::config::Environment = serde_yaml::from_str(include_str!(
            "../../tests/fixtures/validate/valid-kubernetes.yaml"
        ))
        .unwrap();
        environment.spec.ingress =
            Some(serde_json::from_value(json!({"enabled":true,"routes":[]})).unwrap());
        environment.spec.service_claims.insert("web".into(),serde_json::from_value(json!({"kind":"ingress","host":"web.svc.kube-lab.vz.test","backend":{"kubernetes":{"cluster":"dev","namespace":"default","service":"web","port":80}}})).unwrap());
        environment
    }
    #[test]
    fn incomplete_discovery_preserves_existing_edge_without_any_guest_or_host_operation() {
        let guest = RevokedGuest {
            reads: Cell::new(0),
        };
        let (claims, warnings) = reconcile_with_edge(
            &environment(),
            Path::new("/unused/state"),
            &guest,
            false,
            &|_, _, _| panic!("host mutation after incomplete discovery"),
        );
        assert_eq!(guest.reads.get(), 0);
        assert_eq!(
            claims["web"].as_ref().err().unwrap().code,
            "DiscoveryIncomplete"
        );
        assert_eq!(warnings[0].code, "DiscoveryIncomplete");
    }
    #[test]
    fn revoked_kubernetes_read_cannot_fall_through_to_host_edge_mutation() {
        let guest = RevokedGuest {
            reads: Cell::new(0),
        };
        let (claims, warnings) = reconcile_with_edge(
            &environment(),
            Path::new("/unused/state"),
            &guest,
            true,
            &|_, _, _| panic!("host mutation after revocation"),
        );
        assert_eq!(guest.reads.get(), 1);
        assert_eq!(
            claims["web"].as_ref().err().unwrap().code,
            "RegistrationRevoked"
        );
        assert_eq!(warnings[0].code, "RegistrationRevoked");
    }
}
