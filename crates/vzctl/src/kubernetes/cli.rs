use super::{controller::Guest, manifests, node_ip, ClusterConfig};
use crate::config::{self, Environment};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) fn command(
    args: impl Iterator<Item = String>,
    _state: &Path,
    socket: &Path,
) -> ExitCode {
    let args: Vec<String> = args.collect();
    let json_output = args.windows(2).any(|pair| pair == ["--format", "json"]);
    let command = match args.first().map(String::as_str) {
        Some(action @ ("status" | "kubeconfig")) => format!("kubernetes.{action}"),
        _ => "kubernetes".into(),
    };
    match run(&args, socket) {
        Ok(data) => {
            let degraded = args[0] == "status"
                && data["clusters"].as_array().is_some_and(|clusters| {
                    clusters.iter().any(|cluster| cluster["state"] != "ready")
                });
            let code = if degraded { 24 } else { 0 };
            let summary = if args[0] == "status" {
                let count = data["clusters"].as_array().map_or(0, Vec::len);
                json!({"message":if degraded {"Kubernetes cluster is degraded"} else {"Kubernetes clusters are ready"},"clusters":count})
            } else {
                json!({"message":"kubeconfig exported","path":data["path"]})
            };
            if json_output {
                println!(
                    "{}",
                    json!({"apiVersion":"vzctl.dev/v1","command":command,"status":if degraded {"warn"} else {"ok"},"exit_code":code,"summary":summary,"data":data})
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&data).unwrap_or_default()
                );
            }
            ExitCode::from(code)
        }
        Err((code, error)) => {
            if json_output {
                println!(
                    "{}",
                    json!({"apiVersion":"vzctl.dev/v1","command":command,"status":"fail","exit_code":code,"summary":{"message":error},"error":{"code":code,"message":error},"message":error})
                );
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::from(code)
        }
    }
}

fn run(args: &[String], socket: &Path) -> Result<Value, (u8, String)> {
    let usage = || {
        (2, "usage: vzctl kubernetes status [-C DIR] [--cluster NAME] [--format human|json] | kubeconfig -C DIR --cluster NAME --output PATH [--format human|json]".into())
    };
    let action = args.first().map(String::as_str).ok_or_else(usage)?;
    if !["status", "kubeconfig"].contains(&action) {
        return Err(usage());
    }
    let mut directory = PathBuf::from(".");
    let mut cluster_name = None;
    let mut output = None;
    let mut index = 1;
    while index < args.len() {
        let value = args.get(index + 1).ok_or_else(usage)?;
        match args[index].as_str() {
            "-C" | "--config" => directory = PathBuf::from(value),
            "--cluster" => cluster_name = Some(value.as_str()),
            "--output" if action == "kubeconfig" => output = Some(PathBuf::from(value)),
            "--format" if value == "human" || value == "json" => {}
            _ => return Err(usage()),
        }
        index += 2;
    }
    let config_file = config::config_path(&directory);
    let environment = config::validate_path(&config_file).map_err(|issues| {
        (
            3,
            issues
                .iter()
                .map(|i| format!("{}: {}", i.path, i.message))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    let clusters = &environment.spec.kubernetes.clusters;
    if let Some(name) = cluster_name {
        if !clusters.contains_key(name) {
            return Err((3, format!("unknown Kubernetes cluster {name:?}")));
        }
    }
    let guest = Guest {
        environment: &environment,
        socket,
    };
    if action == "kubeconfig" {
        let name = cluster_name.ok_or_else(usage)?;
        let output = output.ok_or_else(usage)?;
        let cluster = &clusters[name];
        let value = guest
            .exec(
                &cluster.server,
                &[
                    "sudo".into(),
                    "-n".into(),
                    "cat".into(),
                    "/etc/rancher/k3s/k3s.yaml".into(),
                ],
                &[],
                15,
            )
            .map_err(|e| (24, e))?;
        let mut kubeconfig: Value =
            serde_yaml::from_str(&value).map_err(|_| (24, "invalid guest kubeconfig".into()))?;
        rewrite_kubeconfig(
            &mut kubeconfig,
            node_ip(&environment, cluster, &cluster.server).map_err(|e| (3, e))?,
        )
        .map_err(|e| (24, e))?;
        let bytes = serde_yaml::to_string(&kubeconfig).map_err(|e| (24, e.to_string()))?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&output)
            .map_err(|e| {
                (
                    24,
                    format!(
                        "create kubeconfig {} (existing files are not overwritten): {e}",
                        output.display()
                    ),
                )
            })?;
        if let Err(error) = file
            .write_all(bytes.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = std::fs::remove_file(&output);
            return Err((24, format!("write kubeconfig: {error}")));
        }
        return Ok(json!({"cluster":name,"path":output,"credentials":"file-only","mode":"0600"}));
    }
    let base = config_file.parent().unwrap_or_else(|| Path::new("."));
    let statuses: Vec<Value> = clusters
        .iter()
        .filter(|(name, _)| cluster_name.is_none_or(|wanted| *name == wanted))
        .map(|(name, cluster)| cluster_status(&guest, &environment, base, name, cluster))
        .collect();
    Ok(json!({"project":environment.spec.project,"clusters":statuses}))
}

fn cluster_status(
    guest: &Guest<'_>,
    environment: &Environment,
    base: &Path,
    name: &str,
    cluster: &ClusterConfig,
) -> Value {
    let mut errors = Vec::new();
    let mut issues = Vec::new();
    let mut nodes = Vec::new();
    match guest.kubectl(cluster, &["get", "nodes", "-o", "json"], &[], 30) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => {
                if let Some(items) = value["items"].as_array() {
                    for node in items {
                        nodes.push(json!({"name":node["metadata"]["name"],"version":node["status"]["nodeInfo"]["kubeletVersion"],
                        "conditions":node["status"]["conditions"]}));
                    }
                } else {
                    record_issue(
                        &mut errors,
                        &mut issues,
                        "InvalidNodeInventory",
                        "nodes.get",
                        json!({}),
                    );
                }
            }
            Err(_) => record_issue(
                &mut errors,
                &mut issues,
                "InvalidNodeInventory",
                "nodes.get",
                json!({}),
            ),
        },
        Err(_) => record_issue(
            &mut errors,
            &mut issues,
            "NodeInventoryUnavailable",
            "nodes.get",
            json!({}),
        ),
    }
    let mut resources = Vec::new();
    for path in &cluster.manifests {
        match manifests::read(&base.join(path)) {
            Ok(objects) => {
                for value in objects {
                    let namespace = value["metadata"]["namespace"]
                        .as_str()
                        .unwrap_or(&cluster.default_namespace);
                    let identity = json!({"apiVersion":value["apiVersion"],"kind":value["kind"],
                    "metadata":{"name":value["metadata"]["name"],"namespace":value["metadata"]["namespace"]}});
                    let result = serde_json::to_vec(&identity)
                        .map_err(|e| e.to_string())
                        .and_then(|data| {
                            guest.kubectl(
                                cluster,
                                &[
                                    "get",
                                    "--ignore-not-found",
                                    "-f",
                                    "-",
                                    "-o",
                                    "json",
                                    "--namespace",
                                    namespace,
                                ],
                                &data,
                                30,
                            )
                        });
                    match result {
                        Ok(text) => match super::controller::read_inventory(&text, &value, namespace) {
                            Ok(None) => resources.push(json!({
                                "file":path,"name":value["metadata"]["name"],"namespace":namespace,"kind":value["kind"],"state":"missing"
                            })),
                            Ok(Some(current)) => {
                                let owned = manifests::check_existing(&current, &environment.spec.project, name).is_ok();
                                let matches = manifests::owned(value.clone(), &environment.spec.project, name)
                                    .is_ok_and(|wanted| manifests::matches_desired(&wanted, &current));
                                resources.push(json!({
                                    "file":path,"name":value["metadata"]["name"],"namespace":namespace,"kind":value["kind"],
                                    "state":if !owned {"ownership-conflict"} else if matches {"present"} else {"drift"},
                                    "conditions":current["status"]["conditions"]
                                }));
                            }
                            Err(_) => record_issue(&mut errors, &mut issues, "InvalidResourceInventory", "resource.get", json!({"file":path,"kind":value["kind"],"name":value["metadata"]["name"],"namespace":namespace})),
                        },
                        Err(_) => record_issue(&mut errors, &mut issues, "ResourceReadFailed", "resource.get", json!({"file":path,"kind":value["kind"],"name":value["metadata"]["name"],"namespace":namespace})),
                    }
                }
            }
            Err(_) => record_issue(
                &mut errors,
                &mut issues,
                "ManifestReadFailed",
                "manifest.read",
                json!({"file":path}),
            ),
        }
    }
    let expected: Vec<&str> = std::iter::once(cluster.server.as_str())
        .chain(cluster.agents.iter().map(String::as_str))
        .collect();
    let nodes_ready = expected.iter().all(|wanted| {
        nodes.iter().any(|node| {
            node["name"] == *wanted
                && node["version"] == cluster.version
                && node["conditions"].as_array().is_some_and(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c["type"] == "Ready" && c["status"] == "True")
                })
        })
    });
    if !nodes_ready && errors.is_empty() {
        record_issue(
            &mut errors,
            &mut issues,
            "NodesNotReadyAtPinnedVersion",
            "nodes.get",
            json!({}),
        );
    }
    for resource in &resources {
        let code = match resource["state"].as_str() {
            Some("missing") => "ResourceMissing",
            Some("ownership-conflict") => "OwnershipConflict",
            Some("drift") => "ResourceDrift",
            _ => continue,
        };
        record_issue(
            &mut errors,
            &mut issues,
            code,
            "resource.get",
            json!({"file":resource["file"],"kind":resource["kind"],"name":resource["name"],"namespace":resource["namespace"]}),
        );
    }
    let ready =
        errors.is_empty() && nodes_ready && resources.iter().all(|r| r["state"] == "present");
    json!({"cluster":name,"version":cluster.version,"state":if ready{"ready"}else{"degraded"},
        "nodes":nodes,"manifests":resources,"errors":errors,"issues":issues})
}

// Stable, attributable diagnostics; raw guest/provider stderr never enters status.
fn record_issue(
    errors: &mut Vec<String>,
    issues: &mut Vec<Value>,
    code: &str,
    operation: &str,
    mut identity: Value,
) {
    errors.push(code.to_owned());
    identity["code"] = json!(code);
    identity["operation"] = json!(operation);
    issues.push(identity);
}

pub(super) fn rewrite_kubeconfig(value: &mut Value, address: &str) -> Result<(), String> {
    let clusters = value["clusters"]
        .as_array_mut()
        .ok_or("kubeconfig has no clusters")?;
    if clusters.len() != 1 {
        return Err("expected one cluster in K3s kubeconfig".into());
    }
    let cluster = clusters[0]["cluster"]
        .as_object_mut()
        .ok_or("invalid kubeconfig cluster")?;
    if cluster.get("insecure-skip-tls-verify") == Some(&json!(true)) {
        return Err("refusing kubeconfig with disabled TLS verification".into());
    }
    let server = cluster
        .get("server")
        .and_then(Value::as_str)
        .ok_or("missing kubeconfig server")?;
    if ![
        "https://127.0.0.1:6443",
        "https://localhost:6443",
        &format!("https://{address}:6443"),
    ]
    .contains(&server)
    {
        return Err("unexpected guest kubeconfig endpoint".into());
    }
    cluster.insert("server".into(), json!(format!("https://{address}:6443")));
    Ok(())
}
