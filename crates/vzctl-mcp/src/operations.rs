//! Read-only operational tools: fixed argv, bounded collection and status projection.
use crate::VzctlCli;
use serde_json::{json, Value};
use std::fs::{self, DirBuilder, File};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

impl VzctlCli {
    pub(crate) fn controller_status(&self, path: &str) -> Result<Value, String> {
        let value = collect(
            self,
            &["controller", "status", "-C", path, "--format", "json"],
        )?;
        if value["command"] != "controller.status" {
            return Err("unexpected controller status response".into());
        }
        let data = &value["data"];
        let claims = data["claims"]
            .as_array()
            .ok_or("controller status unavailable")?;
        Ok(
            json!({"apiVersion":"vzctl.dev/v1","command":"controller.status",
            "status":scalar(&value["status"]),"exit_code":scalar(&value["exit_code"]),
            "data":{"project":scalar(&data["project"]),"source":scalar(&data["source"]),
                "observedAt":scalar(&data["observedAt"]),
                "supervisor":supervisor_status(&data["supervisor"]),
                "warnings":data["warnings"].as_array().into_iter().flatten().take(256)
                    .map(|warning|select(warning,&["claim","cluster","namespace","name","code"])).collect::<Vec<_>>(),
                "claims":claims.iter().take(256).map(|claim| {
                    let mut result = select(claim, &["name","kind","state","observedGeneration","observedAt","lastSuccess","changed"]);
                    result["conditions"] = conditions(&claim["conditions"]);
                    result["source"] = select(&claim["source"],&["type","cluster","namespace","name","uid"]);
                    result["binding"] = public_binding(&claim["binding"]);
                    result
                }).collect::<Vec<_>>()}}),
        )
    }

    pub(crate) fn kubernetes_status(
        &self,
        path: &str,
        cluster: Option<&str>,
    ) -> Result<Value, String> {
        let mut args = vec!["kubernetes", "status", "-C", path, "--format", "json"];
        if let Some(cluster) = cluster {
            args.extend(["--cluster", cluster]);
        }
        let value = collect(self, &args)?;
        if value["command"] != "kubernetes.status" {
            return Err("unexpected Kubernetes status response".into());
        }
        let data = &value["data"];
        let clusters = data["clusters"]
            .as_array()
            .ok_or("Kubernetes status unavailable")?;
        Ok(
            json!({"apiVersion":"vzctl.dev/v1","command":"kubernetes.status",
            "status":scalar(&value["status"]),"data":{"project":scalar(&data["project"]),
            "clusters":clusters.iter().take(64).map(|cluster| {
                let mut result = select(cluster,&["cluster","version","state"]);
                for (field, keys) in [("nodes",vec!["name","version"]),("manifests",vec!["file","name","namespace","kind","state"])] {
                    result[field] = json!(cluster[field].as_array().into_iter().flatten().take(256).map(|item| {
                        let mut result=select(item,&keys);
                        result["conditions"]=conditions(&item["conditions"]);
                        result
                    }).collect::<Vec<_>>());
                }
                // Raw guest errors and condition messages can contain application content.
                result["errorCount"] = json!(cluster["errors"].as_array().map_or(0,Vec::len));
                result["issues"] = kubernetes_issues(&cluster["issues"]);
                result
            }).collect::<Vec<_>>()}}),
        )
    }

    pub(crate) fn diagnose(&self, stack: Option<&str>) -> Result<Value, String> {
        let temporary = TemporaryDirectory::new()?;
        let output = temporary.0.join("report.json");
        let path = output.to_str().ok_or("unsupported temporary path")?;
        let mut args = vec!["diagnose", "--output", path, "--format", "json"];
        if let Some(stack) = stack {
            args.extend(["-C", stack]);
        }
        let result = collect(self, &args)?;
        if result["command"] != "diagnose"
            || !matches!(result["status"].as_str(), Some("ok" | "warn"))
        {
            return Err("diagnose collection failed".into());
        }
        if !fs::symlink_metadata(&output).is_ok_and(|meta| meta.is_file()) {
            return Err("diagnose did not create a regular report".into());
        }
        let mut bytes = Vec::new();
        File::open(&output)
            .map_err(|_| "diagnose report unavailable")?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "diagnose report unreadable")?;
        if bytes.len() > 65536 {
            return Err("diagnose report exceeds 64 KiB".into());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid diagnose report")?;
        if value["apiVersion"] != "vzctl.dev/diagnose/v1" {
            return Err("unknown diagnose report version".into());
        }
        Ok(diagnose_projection(&value))
    }
}

fn scalar(value: &Value) -> Value {
    match value {
        Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            if [
                "password",
                "secret",
                "token",
                "authorization",
                "private key",
                "://",
                "@",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                json!("[redacted]")
            } else {
                json!(text
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(160)
                    .collect::<String>())
            }
        }
        _ => Value::Null,
    }
}
fn select(value: &Value, keys: &[&str]) -> Value {
    let mut selected = serde_json::Map::new();
    for key in keys {
        selected.insert((*key).into(), scalar(&value[*key]));
    }
    Value::Object(selected)
}
fn conditions(value: &Value) -> Value {
    json!(value
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
        .map(|condition| select(condition, &["type", "status", "reason"]))
        .collect::<Vec<_>>())
}

fn supervisor_status(value: &Value) -> Value {
    let mut result = select(value, &["enabled", "generation", "lastAttemptAt", "state"]);
    result["lastStatus"] = select(&value["lastStatus"], &["state", "reason", "warnings"]);
    result
}

fn public_binding(value: &Value) -> Value {
    let mut result = select(
        value,
        &[
            "host", "port", "database", "user", "bucket", "region", "clientID", "scopes", "caFile",
        ],
    );
    for key in ["url", "issuer", "endpoint"] {
        result[key] = value[key].as_str().map(public_url).unwrap_or(Value::Null);
    }
    result
}

// Published endpoints are useful references, but userinfo, query strings and
// fragments can carry credentials. Reject those entirely, including encodings.
fn public_url(value: &str) -> Value {
    let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return json!("[redacted]");
    };
    let authority = rest.split('/').next().unwrap_or("");
    if value.len() > 512
        || authority.is_empty()
        || !authority
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-:[]".contains(&c))
        || !rest
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-:[]/_~".contains(&c))
    {
        return json!("[redacted]");
    }
    json!(value)
}

fn kubernetes_issues(value: &Value) -> Value {
    json!(value
        .as_array()
        .into_iter()
        .flatten()
        .take(256)
        .map(|issue| {
            let mut result = select(issue, &["operation", "file", "kind", "name", "namespace"]);
            result["code"] = json!(match issue["code"].as_str() {
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
            result
        })
        .collect::<Vec<_>>())
}
fn diagnose_projection(value: &Value) -> Value {
    let mut result = select(value, &["apiVersion", "created_at_unix_ms", "cli_version"]);
    result["host"] = select(
        &value["host"],
        &["os", "arch", "logical_cpus", "memory_mib"],
    );
    result["supervisor"] = select(
        &value["supervisor"],
        &[
            "ok",
            "version",
            "pid",
            "uptime_ms",
            "db_ok",
            "networks",
            "network_orphans",
            "dns_ok",
            "vz_edge_ok",
            "vz_net_ok",
        ],
    );
    result["supervisor"]["network_resilience"] =
        select(&value["supervisor"]["network_resilience"], &["state"]);
    for (field, keys, limit) in [
        ("services", vec!["service", "socket_present"], 3),
        ("vms", vec!["vm_id", "state", "pid"], 64),
        (
            "journals",
            vec!["stack_id", "generation", "step", "status", "updated_at"],
            32,
        ),
    ] {
        result[field] = json!(value[field]
            .as_array()
            .into_iter()
            .flatten()
            .take(limit)
            .map(|item| select(item, &keys))
            .collect::<Vec<_>>());
    }
    result["warnings"] = json!(value["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
        .map(scalar)
        .collect::<Vec<_>>());
    result["controllers"] = select(
        &value["controllers"],
        &[
            "project",
            "status",
            "state",
            "source",
            "observedAt",
            "code",
            "truncated",
        ],
    );
    result["controllers"]["supervisor"] = supervisor_status(&value["controllers"]["supervisor"]);
    result["controllers"]["claims"] = json!(value["controllers"]["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .map(|claim| {
            let mut entry = select(
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
            entry["conditions"] = conditions(&claim["conditions"]);
            entry["source"] = select(
                &claim["source"],
                &["type", "cluster", "namespace", "name", "uid"],
            );
            entry
        })
        .collect::<Vec<_>>());
    result["controllers"]["warnings"] = json!(value["controllers"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .map(|warning| select(warning, &["claim", "cluster", "namespace", "name", "code"]))
        .collect::<Vec<_>>());
    result["kubernetes"] = select(
        &value["kubernetes"],
        &["project", "status", "state", "code", "truncated"],
    );
    result["kubernetes"]["clusters"] = json!(value["kubernetes"]["clusters"]
        .as_array()
        .into_iter()
        .flatten()
        .take(8)
        .map(|cluster| {
            let mut entry = select(cluster, &["cluster", "version", "state", "truncated"]);
            entry["issues"] = kubernetes_issues(&cluster["issues"]);
            for (field, keys) in [
                ("nodes", &["name", "version"][..]),
                (
                    "manifests",
                    &["file", "kind", "name", "namespace", "state"][..],
                ),
            ] {
                entry[field] = json!(cluster[field]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(8)
                    .map(|item| {
                        let mut item_status = select(item, keys);
                        item_status["conditions"] = conditions(&item["conditions"]);
                        item_status
                    })
                    .collect::<Vec<_>>());
            }
            entry
        })
        .collect::<Vec<_>>());
    result["collection"] =
        json!({"raw_logs":false,"config_contents":false,"environment":false,"credentials":false});
    result
}

fn collect(cli: &VzctlCli, args: &[&str]) -> Result<Value, String> {
    collect_with_timeout(cli, args, TIMEOUT)
}

fn collect_with_timeout(cli: &VzctlCli, args: &[&str], timeout: Duration) -> Result<Value, String> {
    let mut child = Command::new(cli.binary())
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "vzctl could not be started")?;
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or("missing vzctl output")?;
        let fd = stdout.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err("cannot read bounded vzctl output".into());
        }
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut eof = false;
        loop {
            for _ in 0..16 {
                if eof {
                    break;
                }
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(size) => {
                        bytes.extend_from_slice(&buffer[..size]);
                        if bytes.len() > OUTPUT_LIMIT {
                            return Err("vzctl status exceeds output limit".into());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err("cannot read vzctl status".into()),
                }
            }
            if eof
                && child
                    .try_wait()
                    .map_err(|_| "vzctl process failed")?
                    .is_some()
            {
                // Warn/exit24 is useful status data. Projection accepts only the
                // expected status envelope; raw stderr/error responses stay private.
                return serde_json::from_slice(&bytes)
                    .map_err(|_| "invalid vzctl status JSON".into());
            }
            if Instant::now() >= deadline {
                return Err("vzctl status timed out".into());
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

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Result<Self, String> {
        let mut bytes = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|_| "cannot create private report directory")?;
        let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("vzctl-mcp-diagnose-{token}"));
        DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "cannot create private report directory")?;
        Ok(Self(path))
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn fake_cli(directory: &TemporaryDirectory, body: &str) -> VzctlCli {
        let script = directory.0.join("vzctl");
        fs::write(&script, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        VzctlCli::for_test(script)
    }

    #[test]
    fn controller_warn_is_data_and_argv_and_projection_exclude_credentials() {
        let directory = TemporaryDirectory::new().unwrap();
        let arguments = directory.0.join("args");
        let cli = fake_cli(
            &directory,
            &format!(
                r#"
printf '%s\n' "$@" > '{}'
echo 'SECRET-STDERR' >&2
cat <<'JSON'
{{"command":"controller.status","status":"warn","exit_code":24,"data":{{"project":"demo","supervisor":{{"enabled":true,"generation":"registration-1","lastAttemptAt":42,"config_path":"PRIVATE-PATH","lastStatus":{{"state":"Degraded","reason":"ReconcileDeadlineExceeded","private":"SECRET"}}}},"warnings":[{{"cluster":"dev","code":"DiscoveryUnavailable","message":"SECRET-WARNING"}}],"claims":[{{"name":"db","kind":"postgres","state":"Degraded","source":{{"type":"kubernetes","cluster":"dev","uid":"object-1","secret":"SECRET"}},"binding":{{"host":"db.lan.demo.vz.test","port":"5432","database":"app","user":"app_user","url":"https://app.svc.demo.vz.test/","issuer":"https://user:SECRET@example.test/","endpoint":"https://example.test/?password=SECRET","caFile":"/tmp/public-ca.crt","password":"SECRET-BINDING","unknown":"SECRET-UNKNOWN"}},"secrets":{{"x":"SECRET"}},"conditions":[{{"type":"Ready","status":"False","reason":"GuestUnavailable","message":"SECRET-MESSAGE"}}]}}]}}}}
JSON
exit 24
"#,
                arguments.display()
            ),
        );
        let value = cli
            .controller_status("/tmp/stack name; $(touch danger)")
            .unwrap();
        assert_eq!(value["exit_code"], 24);
        assert_eq!(value["data"]["supervisor"]["enabled"], true);
        assert_eq!(value["data"]["supervisor"]["generation"], "registration-1");
        assert_eq!(value["data"]["supervisor"]["lastAttemptAt"], 42);
        assert_eq!(
            value["data"]["supervisor"]["lastStatus"]["reason"],
            "ReconcileDeadlineExceeded"
        );
        assert_eq!(
            value["data"]["claims"][0]["binding"]["host"],
            "db.lan.demo.vz.test"
        );
        assert_eq!(
            value["data"]["claims"][0]["binding"]["url"],
            "https://app.svc.demo.vz.test/"
        );
        assert_eq!(
            value["data"]["claims"][0]["binding"]["caFile"],
            "/tmp/public-ca.crt"
        );
        assert!(!value.to_string().contains("PRIVATE-PATH"));
        assert!(value["data"]["claims"][0]["binding"]
            .get("unknown")
            .is_none());
        assert_eq!(value["data"]["warnings"][0]["code"], "DiscoveryUnavailable");
        assert_eq!(value["data"]["claims"][0]["source"]["uid"], "object-1");
        assert_eq!(
            value["data"]["claims"][0]["conditions"][0]["reason"],
            "GuestUnavailable"
        );
        assert!(!value.to_string().contains("SECRET"));
        let argv = fs::read_to_string(arguments).unwrap();
        assert_eq!(
            argv,
            "controller\nstatus\n-C\n/tmp/stack name; $(touch danger)\n--format\njson\n"
        );
    }

    #[test]
    fn kubernetes_status_excludes_secret_payloads_and_guest_errors() {
        let directory = TemporaryDirectory::new().unwrap();
        let cli = fake_cli(
            &directory,
            r#"
cat <<'JSON'
{"command":"kubernetes.status","status":"ok","data":{"project":"demo","clusters":[{"cluster":"dev","state":"degraded","kubeconfig":"SECRET-KUBE","nodes":[{"name":"node","version":"v1","conditions":[{"type":"Ready","status":"False","message":"SECRET-NODE"}]}],"manifests":[{"kind":"Secret","name":"db","state":"present","data":{"password":"SECRET-DATA"}}],"errors":["SECRET-ERROR"],"issues":[{"code":"ResourceReadFailed","operation":"resource.get","kind":"ConfigMap","name":"app","message":"SECRET-ERROR"},{"code":"SECRET-UNTRUSTED","message":"SECRET"}]}]}}
JSON
"#,
        );
        let value = cli.kubernetes_status("/tmp/example", Some("dev")).unwrap();
        assert_eq!(value["data"]["clusters"][0]["errorCount"], 1);
        assert_eq!(
            value["data"]["clusters"][0]["issues"][0]["code"],
            "ResourceReadFailed"
        );
        assert_eq!(value["data"]["clusters"][0]["issues"][0]["name"], "app");
        assert_eq!(
            value["data"]["clusters"][0]["issues"][1]["code"],
            "UnknownKubernetesIssue"
        );
        assert_eq!(value["data"]["clusters"][0]["nodes"][0]["name"], "node");
        assert!(!value.to_string().contains("SECRET-"));
    }

    #[test]
    fn diagnose_returns_projection_and_removes_private_temporary_report() {
        let directory = TemporaryDirectory::new().unwrap();
        let recorded = directory.0.join("report-path");
        let cli = fake_cli(
            &directory,
            &format!(
                r#"
printf '%s' "$3" > '{}'
printf '%s\n' "$@" > '{}'
cat > "$3" <<'JSON'
{{"apiVersion":"vzctl.dev/diagnose/v1","host":{{"os":"macos","credentials":"SECRET"}},"vms":[{{"vm_id":"demo/web","state":"running","environment":"SECRET"}}],"journals":[],"controllers":{{"project":"demo","status":"warn","supervisor":{{"enabled":true,"generation":"g1","config_path":"SECRET"}},"claims":[{{"name":"db","state":"Degraded","binding":{{"password":"SECRET"}},"conditions":[{{"type":"Ready","status":"False","reason":"GuestUnavailable"}}]}}]}},"kubernetes":{{"project":"demo","clusters":[{{"cluster":"dev","state":"degraded","issues":[{{"code":"NodeInventoryUnavailable","operation":"nodes.get","message":"SECRET"}}],"kubeconfig":"SECRET"}}]}},"warnings":["vm_inventory_unavailable"],"raw_logs":"SECRET"}}
JSON
echo '{{"command":"diagnose","status":"warn","exit_code":0}}'
"#,
                recorded.display(),
                recorded.with_extension("args").display()
            ),
        );
        let value = cli.diagnose(None).unwrap();
        assert_eq!(value["host"]["os"], "macos");
        assert_eq!(value["controllers"]["supervisor"]["generation"], "g1");
        assert_eq!(
            value["controllers"]["claims"][0]["conditions"][0]["reason"],
            "GuestUnavailable"
        );
        assert_eq!(
            value["kubernetes"]["clusters"][0]["issues"][0]["code"],
            "NodeInventoryUnavailable"
        );
        assert_eq!(value["collection"]["raw_logs"], false);
        assert!(!value.to_string().contains("SECRET"));
        let report = PathBuf::from(fs::read_to_string(&recorded).unwrap());
        assert!(!report.exists());
        assert!(!report.parent().unwrap().exists());
        cli.diagnose(Some("/tmp/stack path; $(false)")).unwrap();
        let argv = fs::read_to_string(recorded.with_extension("args")).unwrap();
        assert!(argv.ends_with("\n-C\n/tmp/stack path; $(false)\n"));
    }

    #[test]
    fn raw_failure_output_is_never_returned_and_tools_are_read_only() {
        let directory = TemporaryDirectory::new().unwrap();
        let cli = fake_cli(
            &directory,
            "echo SECRET-STDOUT; echo SECRET-STDERR >&2; exit 24",
        );
        assert!(!cli.controller_status(".").unwrap_err().contains("SECRET"));
        for tool in [
            crate::VzctlMcp::controller_status_tool_attr(),
            crate::VzctlMcp::kubernetes_status_tool_attr(),
            crate::VzctlMcp::diagnose_tool_attr(),
            crate::VzctlMcp::stack_status_tool_attr(),
        ] {
            assert_eq!(tool.annotations.unwrap().read_only_hint, Some(true));
        }
    }

    #[test]
    fn collection_bounds_output_and_terminates_a_hanging_cli() {
        let directory = TemporaryDirectory::new().unwrap();
        let cli = fake_cli(&directory, "exec /usr/bin/head -c 1100000 /dev/zero");
        assert!(collect(&cli, &[]).unwrap_err().contains("output limit"));
        let cli = fake_cli(&directory, "exec /bin/sleep 30");
        let started = Instant::now();
        assert!(collect_with_timeout(&cli, &[], Duration::from_millis(50))
            .unwrap_err()
            .contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn existing_stack_status_preserves_scoped_health_and_additive_fields() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected = json!({"sections":{"health":{"ok":true,"data":{"status":"warn",
            "probes":[{"id":"web","status":"failed"}],"newField":{"kept":true}}},
            "oidc":{"ok":false,"data":{"enabled":true}}}});
        let payload = expected.to_string();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.extend(byte);
            }
            assert!(String::from_utf8(request)
                .unwrap()
                .starts_with("GET /v1/stacks/nested%2Fdemo/status HTTP/1.1"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            )
            .unwrap();
        });
        let server = crate::VzctlMcp {
            api: crate::ApiClient::for_test_tcp(address.to_string()),
            cli: VzctlCli::for_test(PathBuf::from("unused")),
        };
        let output = server
            .stack_status(rmcp::handler::server::wrapper::Parameters(
                crate::StackIdParams {
                    stack_id: "nested/demo".into(),
                },
            ))
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), expected);
        peer.join().unwrap();
    }
}
