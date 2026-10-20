use super::*;

pub(super) fn ensure_docker_context(environment: &Environment) -> Result<(), Failure> {
    let has_docker = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "docker"));
    if !has_docker {
        return Ok(());
    }
    crate::docker::ensure_context(&environment.spec.project, &crate::state_dir(), None)
        .map_err(|error| Failure::new(EXIT_STEP, format!("docker context: {error}")))?;
    Ok(())
}

pub(super) fn ensure_containers(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
) -> Result<(), Failure> {
    let config_dir = config::config_path(config_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let state_dir = crate::state_dir();
    let mut dns_records = Vec::new();
    for (vm_key, vm) in &environment.spec.vms {
        if !vm.roles.iter().any(|role| role == "docker") {
            continue;
        }
        if vm.compose_files.is_empty() && vm.containers.is_empty() {
            continue;
        }
        crate::docker::ensure_vm_containers(
            &environment.spec.project,
            &state_dir,
            &config_dir,
            vm_key,
            &vm.compose_files,
            &vm.containers,
            &environment.spec.service_claims,
        )
        .map_err(|error| {
            Failure::new(
                EXIT_STEP,
                format!("ensure containers on VM {vm_key}: {error}"),
            )
        })?;

        let docker_attachment = vm
            .networks
            .iter()
            .find(|attachment| {
                environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend == crate::config::NetworkBackend::Docker)
            })
            .ok_or_else(|| {
                Failure::new(
                    EXIT_STEP,
                    format!("docker VM {vm_key} has no backend: docker network"),
                )
            })?;
        let primary = vm
            .networks
            .iter()
            .find(|attachment| {
                environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend != crate::config::NetworkBackend::Docker)
            })
            .ok_or_else(|| {
                Failure::new(
                    EXIT_STEP,
                    format!("docker VM {vm_key} has no primary vmnet network"),
                )
            })?;
        let docker_cidr = environment.spec.networks[&docker_attachment.name]
            .cidr
            .parse::<ipnet::Ipv4Net>()
            .map_err(|error| Failure::new(EXIT_STEP, format!("docker DNS CIDR: {error}")))?;
        let networks = rpc(socket_path, "net.list", json!({}))?;
        let listener = networks["networks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|network| {
                network["name"] == primary.name && network["project"] == environment.spec.project
            })
            .ok_or_else(|| Failure::new(EXIT_STEP, "Docker DNS listener network missing"))?;
        let listener_id = listener.get("id").unwrap_or(&listener["name"]);
        let discovered = crate::docker::collect_vm_container_dns(
            &environment.spec.project,
            &state_dir,
            vm_key,
            &vm.compose_files,
            docker_cidr,
        )
        .map_err(|error| Failure::new(EXIT_STEP, format!("container DNS: {error}")))?;
        dns_records.extend(discovered.into_iter().map(|record| {
            json!({
                "name": record.name,
                "network": docker_attachment.name,
                "listener_network": listener_id,
                "stack": environment.spec.project,
                "project": environment.spec.project,
                "ip": record.ip.to_string(),
            })
        }));
    }
    rpc(
        socket_path,
        "dns.records.ensure",
        json!({
            "project": environment.spec.project,
            "records": dns_records,
        }),
    )?;
    Ok(())
}

pub(super) fn purge_docker_context(environment: &Environment) -> Result<(), Failure> {
    let has_docker = environment
        .spec
        .vms
        .values()
        .any(|vm| vm.roles.iter().any(|role| role == "docker"));
    if !has_docker {
        return Ok(());
    }
    let _ = crate::docker::remove_context(&environment.spec.project);
    Ok(())
}

pub(super) fn ensure_ca(environment: &Environment) -> Result<(), Failure> {
    let enabled = environment.spec.certs.as_ref().is_some_and(|c| c.enabled)
        || environment.spec.ingress.as_ref().is_some_and(|i| i.enabled)
        || environment.spec.oidc.as_ref().is_some_and(|o| o.enabled);
    if !enabled {
        return Ok(());
    }
    crate::certs::ensure_ca(&crate::state_dir(), false)
        .map_err(|e| Failure::new(EXIT_STEP, format!("ensure CA: {e}")))?;
    Ok(())
}

pub(super) fn ensure_oidc(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
) -> Result<(), Failure> {
    ensure_oidc_guarded(environment, config_path, socket_path, &|| Ok(()))
}

pub(super) fn ensure_oidc_guarded(
    environment: &Environment,
    config_path: &Path,
    socket_path: &Path,
    guard: &dyn Fn() -> Result<(), Failure>,
) -> Result<(), Failure> {
    guard()?;
    let Some(oidc) = environment.spec.oidc.as_ref().filter(|o| o.enabled) else {
        return Ok(());
    };
    let state_dir = crate::state_dir();
    let mut vm_names = Vec::new();
    for (name, vm) in &environment.spec.vms {
        if vm.requires.iter().any(|r| r == "oidc") {
            vm_names.push(name.clone());
        }
    }
    let mut route_hosts = Vec::new();
    if let Some(ingress) = &environment.spec.ingress {
        for route in &ingress.routes {
            route_hosts.push((route.host.clone(), route.requires.clone()));
        }
    }
    guard()?;
    let clients = crate::oidc::auto_clients(
        &state_dir,
        &environment.spec.project,
        &environment.spec.domain,
        &vm_names,
        &route_hosts,
        &environment.spec.service_claims,
    )
    .map_err(|e| Failure::new(EXIT_STEP, format!("oidc clients: {e}")))?;

    let storage = state_dir
        .join("runtime")
        .join("oidc")
        .join(&environment.spec.project);
    guard()?;
    fs::create_dir_all(&storage).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;

    match oidc.mode {
        config::OidcMode::OidcSimple => {
            let config_json = crate::oidc::render_simple_config(
                &oidc.issuer,
                &oidc.listen,
                &clients,
                &oidc.users,
            )
            .map_err(|e| Failure::new(EXIT_STEP, format!("oidc-simple config: {e}")))?;
            let binary = state_dir.join("bin").join("vzctl-oidc-simple");
            let binary = if binary.exists() {
                binary
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/debug/vzctl-oidc-simple")
            };
            let binary = if binary.exists() {
                binary
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/release/vzctl-oidc-simple")
            };
            guard()?;
            rpc(
                socket_path,
                "oidc.ensure",
                json!({
                    "project": environment.spec.project,
                    "config": config_json,
                    "configName": "config.json",
                    "binary": binary.display().to_string(),
                    "arguments": ["--config", "{config}"],
                    "processName": format!("oidc-simple-{}", environment.spec.project),
                    "pidFile": "oidc.pid",
                    "listen": oidc.listen,
                }),
            )?;
        }
        config::OidcMode::Embedded => {
            let password_file = oidc.password_file.as_ref().map(|rel| {
                config::config_path(config_path)
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(rel)
            });
            let host_uplink = crate::oidc::load_host_uplink(&state_dir)
                .map_err(|e| Failure::new(EXIT_STEP, format!("oidc host uplink: {e}")))?;
            let resolved_uplink = crate::oidc::merge_uplink(
                &state_dir,
                &environment.spec.project,
                host_uplink.as_ref(),
                oidc.uplink.as_ref(),
            )
            .map_err(|e| Failure::new(EXIT_STEP, format!("oidc uplink: {e}")))?;
            let config_yaml = crate::oidc::render_dex_config(
                &oidc.issuer,
                &oidc.listen,
                &clients,
                password_file.as_deref(),
                &storage,
                resolved_uplink.as_ref(),
            )
            .map_err(|e| Failure::new(EXIT_STEP, format!("dex config: {e}")))?;

            let binary = state_dir.join("bin").join("dex");
            let binary = if binary.exists() {
                binary
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../daemon/Vendor/dex/dex")
            };
            guard()?;
            rpc(
                socket_path,
                "oidc.ensure",
                json!({
                    "project": environment.spec.project,
                    "config": config_yaml,
                    "configName": "config.yaml",
                    "binary": binary.display().to_string(),
                    "arguments": ["serve", "{config}"],
                    "processName": format!("dex-{}", environment.spec.project),
                    "pidFile": "oidc.pid",
                    "listen": oidc.listen,
                }),
            )?;
        }
    }
    Ok(())
}

pub(super) fn ensure_ingress(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    ensure_ingress_guarded(environment, socket_path, &|| Ok(()))
}

pub(super) fn ensure_ingress_guarded(
    environment: &Environment,
    socket_path: &Path,
    guard: &dyn Fn() -> Result<(), Failure>,
) -> Result<(), Failure> {
    guard()?;
    let Some(ingress) = environment.spec.ingress.as_ref().filter(|i| i.enabled) else {
        return Ok(());
    };
    let state_dir = crate::state_dir();
    // Mint leafs for each route (+ localhost aliases).
    for route in &ingress.routes {
        let mut extras = Vec::new();
        if ingress.host_aliases {
            if let Some(alias) = crate::ingress::short_localhost(&route.host) {
                extras.push(alias);
            }
        }
        guard()?;
        crate::certs::mint_leaf(&state_dir, &route.host, &extras)
            .map_err(|e| Failure::new(EXIT_STEP, format!("mint {}: {e}", route.host)))?;
    }

    let snapshot = rpc(socket_path, "net.list", json!({}))?;
    let networks = snapshot["networks"].as_array().cloned().unwrap_or_default();
    let attachments = snapshot["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rendered = crate::ingress::render(environment, &networks, &attachments, &state_dir)
        .map_err(|e| Failure::new(EXIT_STEP, format!("caddyfile: {e}")))?;

    let binary = state_dir.join("bin").join("caddy");
    let binary = if binary.exists() {
        binary
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../daemon/Vendor/caddy/caddy")
    };
    let host_entries: Vec<Value> = rendered
        .hosts
        .iter()
        .map(|host| {
            if let Some(networks) = rendered.guest_dns.get(host) {
                json!({ "host": host, "guest": networks })
            } else {
                json!({ "host": host })
            }
        })
        .collect();
    rpc_retrying(
        socket_path,
        "ingress.ensure",
        json!({
            "project": environment.spec.project,
            "caddyfile": rendered.caddyfile,
            "binary": binary.display().to_string(),
            "http_port": rendered.http_port,
            "https_port": rendered.https_port,
            "backend_http_port": rendered.caddy_http_port,
            "backend_https_port": rendered.caddy_https_port,
            "gateways": rendered.gateways,
            "gateway_bindings": rendered.gateway_bindings,
            "hosts": host_entries,
        }),
        guard,
    )?;
    Ok(())
}

pub(super) fn ensure_ca_rollout(
    environment: &Environment,
    socket_path: &Path,
) -> Result<(), Failure> {
    let enabled = environment.spec.certs.as_ref().is_some_and(|c| c.enabled)
        || environment.spec.ingress.as_ref().is_some_and(|i| i.enabled)
        || environment.spec.oidc.as_ref().is_some_and(|o| o.enabled);
    if !enabled {
        return Ok(());
    }
    let state_dir = crate::state_dir();
    let pem = crate::certs::read_ca_pem(&state_dir).map_err(|e| Failure::new(EXIT_STEP, e))?;
    let fingerprint =
        crate::certs::read_fingerprint(&state_dir).map_err(|e| Failure::new(EXIT_STEP, e))?;
    let targets = environment
        .spec
        .vms
        .keys()
        .map(|name| vm_runtime_id(environment, name))
        .collect::<Vec<_>>();
    rollout_ca_to_vms(&targets, &pem, &fingerprint, |method, params| {
        rpc(socket_path, method, params)
    })
}

const LEGACY_CA_INSTALL_SCRIPT: &str = r#"set -eu
fingerprint=$1
pem_b64=$2
ca_dir=/usr/local/share/ca-certificates
fingerprint_file=/var/lib/vzctl/ca.fingerprint
install -d -m 0755 "$ca_dir" /var/lib/vzctl
pem_tmp=$(mktemp "$ca_dir/.vzctl-local.crt.XXXXXX")
der_tmp=$(mktemp)
fingerprint_tmp=$(mktemp /var/lib/vzctl/.ca.fingerprint.XXXXXX)
trap 'rm -f "$pem_tmp" "$der_tmp" "$fingerprint_tmp"' EXIT
printf '%s' "$pem_b64" | base64 -d >"$pem_tmp"
openssl x509 -in "$pem_tmp" -outform DER >"$der_tmp"
actual=$(openssl dgst -sha256 -r "$der_tmp" | awk '{print $1}')
[ "$actual" = "$fingerprint" ] || {
  echo "CA fingerprint mismatch: expected $fingerprint, got $actual" >&2
  exit 1
}
chmod 0644 "$pem_tmp"
mv -f "$pem_tmp" "$ca_dir/vzctl-local.crt"
/usr/sbin/update-ca-certificates
openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt "$ca_dir/vzctl-local.crt" >/dev/null
printf '%s\n' "$fingerprint" >"$fingerprint_tmp"
chmod 0644 "$fingerprint_tmp"
mv -f "$fingerprint_tmp" "$fingerprint_file"
"#;

pub(super) fn rollout_ca_to_vms<F>(
    targets: &[String],
    pem: &str,
    fingerprint: &str,
    mut call: F,
) -> Result<(), Failure>
where
    F: FnMut(&str, Value) -> Result<Value, Failure>,
{
    let pem_b64 = base64::engine::general_purpose::STANDARD.encode(pem.as_bytes());
    let mut failures = Vec::new();
    for vm_id in targets {
        let primary = call(
            "vm.agent.ca_inject",
            json!({
                "vm_id": vm_id,
                "pem": pem,
                "fingerprint": fingerprint,
                "name": "vzctl-local",
            }),
        )
        .and_then(|result| validate_ca_inject_result(&result, fingerprint));
        if primary.is_ok() {
            continue;
        }
        let primary_message = primary
            .err()
            .map(|failure| failure.message)
            .unwrap_or_else(|| "unknown ca_inject failure".to_string());
        let fallback = call(
            "vm.exec",
            json!({
                "vm_id": vm_id,
                "cmd": [
                    "sudo",
                    "-n",
                    "sh",
                    "-c",
                    LEGACY_CA_INSTALL_SCRIPT,
                    "vzctl-ca-fallback",
                    fingerprint,
                    pem_b64,
                ],
                "timeout_ms": 60_000,
            }),
        )
        .and_then(validate_ca_exec_result);
        if let Err(failure) = fallback {
            failures.push(format!(
                "{vm_id}: ca_inject failed ({primary_message}); compatibility fallback failed ({})",
                failure.message
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_STEP,
            format!("CA rollout failed: {}", failures.join("; ")),
        ))
    }
}

pub(super) fn validate_ca_inject_result(
    result: &Value,
    fingerprint: &str,
) -> Result<Value, Failure> {
    if result["installed"] == true && result["fingerprint"].as_str() == Some(fingerprint) {
        Ok(result.clone())
    } else {
        Err(Failure::new(
            EXIT_STEP,
            "ca_inject returned an invalid result",
        ))
    }
}

pub(super) fn validate_ca_exec_result(result: Value) -> Result<Value, Failure> {
    if result["exit"].as_i64() == Some(0) {
        return Ok(result);
    }
    let detail = result["stderr"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| result["stdout"].as_str())
        .unwrap_or("missing vm.exec result");
    Err(Failure::new(
        EXIT_STEP,
        format!("guest command exited with {}: {detail}", result["exit"]),
    ))
}

pub(super) fn ensure_oidc_inject(
    environment: &Environment,
    socket_path: &Path,
) -> Result<(), Failure> {
    let Some(oidc) = environment.spec.oidc.as_ref().filter(|o| o.enabled) else {
        return Ok(());
    };
    let path = crate::oidc::clients_path(&crate::state_dir(), &environment.spec.project);
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;
    let clients: Value =
        serde_json::from_str(&raw).map_err(|e| Failure::new(EXIT_STEP, e.to_string()))?;
    let list = clients["clients"].as_array().cloned().unwrap_or_default();
    for client in list {
        let id = client["id"].as_str().unwrap_or("client");
        let secret = client["secret"].as_str().unwrap_or("");
        let redirect = client["redirectURIs"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !environment.spec.vms.contains_key(id) {
            continue;
        }
        let runtime_id = vm_runtime_id(environment, id);
        let env_block = format!(
            "OIDC_ISSUER={}\nOIDC_CLIENT_ID={id}\nOIDC_CLIENT_SECRET={secret}\n\
             OIDC_REDIRECT_URI={redirect}\nOIDC_CA_PATH=/etc/ssl/certs/ca-certificates.crt\n",
            oidc.issuer
        );
        let script = format!(
            "mkdir -p /etc/vzctl && cat > /etc/vzctl/oidc.env <<'EOF'\n{env_block}EOF\nchmod 600 /etc/vzctl/oidc.env"
        );
        let _ = rpc(
            socket_path,
            "vm.exec",
            json!({
                "vm_id": runtime_id,
                "cmd": ["bash", "-lc", script],
                "timeout_ms": 30_000,
            }),
        );
    }
    Ok(())
}

pub(super) fn purge_ingress(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let _ = rpc(
        socket_path,
        "ingress.purge",
        json!({ "project": environment.spec.project }),
    );
    Ok(())
}

pub(super) fn purge_oidc(environment: &Environment, socket_path: &Path) -> Result<(), Failure> {
    let _ = rpc(
        socket_path,
        "oidc.purge",
        json!({ "project": environment.spec.project }),
    );
    Ok(())
}

pub(super) fn ensure_service_claims(
    environment: &Environment,
    config_path: &Path,
    socket: &Path,
    progress: &mut ProgressReporter,
) -> Result<(), Failure> {
    if environment.spec.service_claims.is_empty() && environment.spec.kubernetes.clusters.is_empty()
    {
        return Ok(());
    }
    let file = fs::canonicalize(config::config_path(config_path)).map_err(|error| {
        Failure::new(EXIT_INVALID, format!("resolve controller config: {error}"))
    })?;
    rpc(
        socket,
        "controller.register",
        json!({"project":environment.spec.project,"config_path":file}),
    )?;
    let result = crate::controllers::reconcile(
        environment,
        &config::config_path(config_path),
        socket,
        &crate::state_dir(),
    )
    .map_err(|error| Failure::new(EXIT_STEP, format!("service claims: {error}")))?;
    check_claim_readiness(&result)?;
    let observed = result["claims"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|claim| claim["name"].as_str())
        .collect::<BTreeSet<_>>();
    if environment
        .spec
        .service_claims
        .keys()
        .any(|name| !observed.contains(name.as_str()))
    {
        return Err(Failure::new(
            EXIT_STEP,
            "service controller omitted a declared claim",
        ));
    }
    progress.log(json!({"event":"service_claims.ready","claims":result["claims"].as_array().map_or(0,Vec::len)}).to_string());
    Ok(())
}

pub(super) fn check_claim_readiness(result: &Value) -> Result<(), Failure> {
    if result["warnings"]
        .as_array()
        .is_some_and(|warnings| !warnings.is_empty())
    {
        return Err(Failure::new(
            EXIT_STEP,
            "service controller reported discovery or status warnings; inspect controller status",
        ));
    }
    let claims = result["claims"]
        .as_array()
        .ok_or_else(|| Failure::new(EXIT_STEP, "service controller omitted claim status"))?;
    let failed = claims
        .iter()
        .filter(|claim| claim["state"] != "Ready" && claim["state"] != "Retained")
        .map(|claim| {
            format!(
                "{}: {} ({})",
                claim["name"].as_str().unwrap_or("unknown"),
                claim["state"].as_str().unwrap_or("Unknown"),
                claim["conditions"]
                    .as_array()
                    .and_then(|conditions| conditions.iter().find(|c| c["type"] == "Ready"))
                    .and_then(|condition| condition["reason"].as_str())
                    .unwrap_or("NotReady")
            )
        })
        .collect::<Vec<_>>();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_STEP,
            format!("service claims not Ready: {}", failed.join("; ")),
        ))
    }
}
