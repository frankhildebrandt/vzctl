//! Explicit stack diagnostics: bounded read-only CLI calls and fixed projections.
use super::{selected, Value};
use serde_json::json;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn collect(
    executable: &Path,
    group: &str,
    stack: &Path,
    state: &Path,
    timeout: Duration,
) -> Result<Value, ()> {
    let mut child = Command::new(executable)
        .args([group, "status", "-C"])
        .arg(stack)
        .args(["--format", "json"])
        .env("VZCTL_STATE_DIR", state)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| ())?;
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or(())?;
        let fd = stdout.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(());
        }
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let mut eof = false;
        loop {
            let mut buffer = [0u8; 8192];
            for _ in 0..16 {
                if eof {
                    break;
                }
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(count) => {
                        bytes.extend_from_slice(&buffer[..count]);
                        if bytes.len() > 1_048_576 {
                            return Err(());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(()),
                }
            }
            if eof && child.try_wait().map_err(|_| ())?.is_some() {
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
                if value["command"] != format!("{group}.status")
                    || !matches!(value["status"].as_str(), Some("ok" | "warn"))
                {
                    return Err(());
                }
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

fn conditions(value: &Value) -> Value {
    json!(value
        .as_array()
        .into_iter()
        .flatten()
        .take(4)
        .map(|v| selected(v, &["type", "status", "reason"]))
        .collect::<Vec<_>>())
}

pub(super) fn project(envelope: &Value, group: &str) -> Value {
    let data = &envelope["data"];
    let mut result = selected(data, &["project", "source", "observedAt"]);
    result["status"] = selected(envelope, &["status"])["status"].clone();
    if group == "controller" {
        result["supervisor"] = selected(
            &data["supervisor"],
            &["enabled", "generation", "lastAttemptAt", "state"],
        );
        result["supervisor"]["lastStatus"] = selected(
            &data["supervisor"]["lastStatus"],
            &["state", "reason", "warnings"],
        );
        result["claims"] = json!(data["claims"]
            .as_array()
            .into_iter()
            .flatten()
            .take(16)
            .map(|claim| {
                let mut claim_status = selected(
                    claim,
                    &[
                        "name",
                        "kind",
                        "state",
                        "observedGeneration",
                        "observedAt",
                        "lastSuccess",
                    ],
                );
                claim_status["source"] = selected(
                    &claim["source"],
                    &["type", "cluster", "namespace", "name", "uid"],
                );
                claim_status["conditions"] = conditions(&claim["conditions"]);
                claim_status
            })
            .collect::<Vec<_>>());
        result["warnings"] = json!(data["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .take(16)
            .map(|v| selected(v, &["claim", "cluster", "namespace", "name", "code"]))
            .collect::<Vec<_>>());
        result["truncated"] = json!(
            data["claims"].as_array().is_some_and(|v| v.len() > 16)
                || data["warnings"].as_array().is_some_and(|v| v.len() > 16)
        );
    } else {
        result["clusters"] = json!(data["clusters"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .map(|cluster| {
                let mut status = selected(cluster, &["cluster", "version", "state"]);
                for (field, keys) in [
                    ("nodes", &["name", "version"][..]),
                    (
                        "manifests",
                        &["file", "name", "namespace", "kind", "state"][..],
                    ),
                ] {
                    status[field] = json!(cluster[field]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(8)
                        .map(|entry| {
                            let mut item = selected(entry, keys);
                            item["conditions"] = conditions(&entry["conditions"]);
                            item
                        })
                        .collect::<Vec<_>>());
                }
                status["issues"] = json!(cluster["issues"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(16)
                    .map(|issue| {
                        let mut item =
                            selected(issue, &["operation", "file", "kind", "name", "namespace"]);
                        item["code"] = json!(match issue["code"].as_str() {
                            Some(
                                code @ ("NodeInventoryUnavailable"
                                | "InvalidNodeInventory"
                                | "ResourceReadFailed"
                                | "InvalidResourceInventory"
                                | "ManifestReadFailed"
                                | "NodesNotReadyAtPinnedVersion"
                                | "ResourceMissing"
                                | "OwnershipConflict"
                                | "ResourceDrift"),
                            ) => code,
                            _ => "UnknownKubernetesIssue",
                        });
                        item
                    })
                    .collect::<Vec<_>>());
                status["truncated"] = json!(
                    ["nodes", "manifests"]
                        .iter()
                        .any(|field| cluster[*field].as_array().is_some_and(|v| v.len() > 8))
                        || cluster["issues"].as_array().is_some_and(|v| v.len() > 16)
                );
                status
            })
            .collect::<Vec<_>>());
        result["truncated"] = json!(data["clusters"].as_array().is_some_and(|v| v.len() > 8));
    }
    if serde_json::to_vec_pretty(&result).map_or(true, |bytes| bytes.len() > 16_384) {
        json!({"state":"unavailable","code":"DiagnosticSectionLimit","truncated":true})
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn stack_diagnostics_drop_nested_credentials_and_bound_inventory() {
        let claim = json!({"name":"db","state":"Degraded","kind":{"password":"PRIVATE"},
            "source":{"type":"kubernetes","cluster":"dev","namespace":"app","name":"db","uid":"claim-1","credentials":"PRIVATE"},
            "conditions":[{"type":"Ready","status":"False","reason":"GuestUnavailable","message":"PRIVATE"}],
            "binding":{"password":"PRIVATE"}});
        let value = project(
            &json!({"status":"warn","data":{"project":"demo",
            "supervisor":{"enabled":true,"generation":"g1","lastAttemptAt":42,"config_path":"PRIVATE"},
            "claims":vec![claim; 100],"warnings":[{"code":"DiscoveryUnavailable","message":"PRIVATE"}]}}),
            "controller",
        );
        assert_eq!(value["supervisor"]["generation"], "g1");
        assert_eq!(value["claims"].as_array().unwrap().len(), 16);
        assert_eq!(value["claims"][0]["source"]["uid"], "claim-1");
        assert_eq!(value["truncated"], true);
        assert!(!value.to_string().contains("PRIVATE"));
        let value = project(
            &json!({"status":"warn","data":{"project":"demo","clusters":[{
                "cluster":"dev","state":"degraded","kubeconfig":"PRIVATE","errors":["PRIVATE"],
                "issues":[{"code":"ResourceReadFailed","name":"db","namespace":"app","message":"PRIVATE"},
                    {"code":"PRIVATE","message":"PRIVATE"}],
                "manifests":[{"name":"db","kind":"Secret","data":{"password":"PRIVATE"}}]
            }]}}),
            "kubernetes",
        );
        assert_eq!(
            value["clusters"][0]["issues"][0]["code"],
            "ResourceReadFailed"
        );
        assert_eq!(
            value["clusters"][0]["issues"][1]["code"],
            "UnknownKubernetesIssue"
        );
        assert!(!value.to_string().contains("PRIVATE"));
        assert!(serde_json::to_vec(&value).unwrap().len() <= 16_384);
    }

    #[test]
    fn stack_collection_limits_output_time_and_ignores_raw_failures() {
        let root = std::env::temp_dir().join(format!(
            "vz-diag-proc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("cli");
        for body in [
            "exec /usr/bin/head -c 1100000 /dev/zero",
            "exec /bin/sleep 30",
            "echo PRIVATE >&2; echo PRIVATE; exit 24",
        ] {
            fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            let started = Instant::now();
            assert!(collect(
                &executable,
                "controller",
                &root,
                &root,
                Duration::from_millis(100)
            )
            .is_err());
            assert!(started.elapsed() < Duration::from_secs(3));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
