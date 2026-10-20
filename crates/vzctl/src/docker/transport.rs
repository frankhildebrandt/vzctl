//! Docker's management transport is a VM address, independent of app ingress DNS.
use serde_json::{json, Value};
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;

pub(super) fn management_address(
    project: &str,
    state: &Path,
    vm_key: Option<&str>,
) -> Result<Ipv4Addr, String> {
    if !component(project) || vm_key.is_some_and(|key| !component(key)) {
        return Err("invalid Docker project or VM key".into());
    }
    let socket = state.join("vz.sock");
    let inventory = crate::vm::supervisor_rpc_deadline(&socket, "vm.list", json!({}), 5)
        .map_err(|_| "Docker management VM inventory unavailable".to_string())?;
    let selected = select_vm(project, state, vm_key, &inventory)?;
    let attachments = crate::network::list_vm_attachments(&socket, &selected)
        .map_err(|_| "Docker management attachment inventory unavailable".to_string())?;
    select_address(project, &attachments)
}

fn component(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

fn select_vm(
    project: &str,
    state: &Path,
    requested: Option<&str>,
    inventory: &Value,
) -> Result<String, String> {
    let records = inventory
        .as_array()
        .ok_or("invalid Docker management VM inventory")?;
    let prefix = format!("{project}/");
    let mut candidates = Vec::new();
    for record in records {
        let Some(id) = record["vm_id"].as_str() else {
            return Err("VM inventory missing identity".into());
        };
        let Some(key) = id.strip_prefix(&prefix) else {
            continue;
        };
        if !component(key) || requested.is_some_and(|wanted| wanted != key) {
            continue;
        }
        let path = state.join("vms").join(id).join("vm.json");
        let metadata = fs::symlink_metadata(&path).map_err(|_| "Docker VM manifest unavailable")?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err("invalid Docker VM manifest".into());
        }
        let manifest: Value =
            serde_json::from_slice(&fs::read(path).map_err(|_| "Docker VM manifest unreadable")?)
                .map_err(|_| "invalid Docker VM manifest JSON")?;
        if manifest["vm_id"] != id {
            return Err("Docker VM manifest identity mismatch".into());
        }
        if manifest["roles"]
            .as_array()
            .is_some_and(|roles| roles.iter().any(|role| role == "docker"))
        {
            candidates.push((id, record["state"].as_str()));
        }
    }
    if candidates.len() != 1 {
        return Err(
            "Docker management requires exactly one selected Docker VM in this project".into(),
        );
    }
    if candidates[0].1 != Some("running") {
        return Err("selected Docker management VM is not running".into());
    }
    Ok(candidates[0].0.into())
}

fn select_address(
    project: &str,
    attachments: &[crate::network::VmNetworkSelection],
) -> Result<Ipv4Addr, String> {
    let candidates = attachments
        .iter()
        .filter(|a| !a.is_docker_backend())
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err("Docker management requires one unambiguous vmnet attachment".into());
    }
    let selected = candidates[0];
    if selected.backend != "vmnet" || selected.project.as_deref() != Some(project) {
        return Err("Docker management attachment project/backend mismatch".into());
    }
    let address = selected
        .ip
        .parse::<Ipv4Addr>()
        .map_err(|_| "invalid Docker management IPv4 address")?;
    let subnet = selected
        .cidr
        .parse::<ipnet::Ipv4Net>()
        .map_err(|_| "invalid Docker management subnet")?;
    if !address.is_private()
        || !subnet.contains(&address)
        || address == subnet.network()
        || address == subnet.broadcast()
        || selected.gateway == address.to_string()
    {
        return Err("Docker management address is not a private guest address".into());
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn probe(
        records: Value,
        network: Value,
        requested: Option<&str>,
        requests: usize,
    ) -> Result<Ipv4Addr, String> {
        let root = PathBuf::from(format!(
            "/tmp/vzdt-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        for vm in records.as_array().unwrap() {
            let id = vm["vm_id"].as_str().unwrap();
            let path = root.join("vms").join(id).join("vm.json");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json!({"vm_id":id,"roles":["docker"]}).to_string()).unwrap();
        }
        let listener = UnixListener::bind(root.join("vz.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            for _ in 0..requests {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing inventory request");
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "vm.list" => &records,
                    "net.list" => &network,
                    other => panic!("unexpected mutation {other}"),
                };
                writeln!(
                    stream,
                    "{}",
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                )
                .unwrap();
            }
        });
        let result = management_address("controller-lab", &root, requested);
        worker.join().unwrap();
        fs::remove_dir_all(root).unwrap();
        result
    }

    fn records() -> Value {
        json!([{"vm_id":"controller-lab/ops","state":"running"},
            {"vm_id":"other/ops","state":"running"}])
    }

    fn networks() -> Value {
        json!({"networks":[
            {"id":"controller-lab/lan","name":"lan","cidr":"10.123.0.0/24","gateway":"10.123.0.0","dns":"10.123.0.0","backend":"vmnet","project":"controller-lab"},
            {"id":"controller-lab/containers","name":"containers","cidr":"10.124.0.0/24","gateway":"10.124.0.2","dns":"10.124.0.2","backend":"docker","project":"controller-lab"}],
            "attachments":[
            {"vm_id":"controller-lab/ops","network_id":"controller-lab/containers","network":"containers","ip":"10.124.0.2","project":"controller-lab"},
            {"vm_id":"controller-lab/ops","network_id":"controller-lab/lan","network":"lan","ip":"10.123.0.10","project":"controller-lab"},
            {"vm_id":"other/ops","network_id":"controller-lab/lan","network":"lan","ip":"10.123.0.99","project":"other"}]})
    }

    #[test]
    fn live_inventory_selects_project_vmnet_address_not_logical_bridge_or_foreign_vm() {
        assert_eq!(
            probe(records(), networks(), None, 2).unwrap(),
            Ipv4Addr::new(10, 123, 0, 10)
        );
        assert_eq!(
            probe(records(), networks(), Some("ops"), 2).unwrap(),
            Ipv4Addr::new(10, 123, 0, 10)
        );
    }

    #[test]
    fn multiple_docker_vms_require_an_explicit_vm_key() {
        let mut vms = records();
        vms.as_array_mut()
            .unwrap()
            .push(json!({"vm_id":"controller-lab/second","state":"running"}));
        assert!(probe(vms.clone(), networks(), None, 1).is_err());
        assert_eq!(
            probe(vms, networks(), Some("ops"), 2).unwrap(),
            Ipv4Addr::new(10, 123, 0, 10)
        );
        assert!(probe(records(), networks(), Some("missing"), 1).is_err());
    }

    #[test]
    fn missing_ambiguous_cross_project_or_non_guest_addresses_fail_closed() {
        for fault in [
            "missing",
            "ambiguous",
            "project",
            "loopback",
            "outside",
            "gateway",
            "backend",
        ] {
            let mut net = networks();
            match fault {
                "missing" => {
                    net["attachments"].as_array_mut().unwrap().remove(1);
                }
                "ambiguous" => {
                    let extra = net["attachments"][1].clone();
                    net["attachments"].as_array_mut().unwrap().push(extra);
                }
                "project" => net["attachments"][1]["project"] = json!("other"),
                "loopback" => net["attachments"][1]["ip"] = json!("127.0.0.1"),
                "outside" => net["attachments"][1]["ip"] = json!("10.125.0.10"),
                "gateway" => net["attachments"][1]["ip"] = json!("10.123.0.0"),
                "backend" => net["networks"][0]["backend"] = json!("unknown"),
                _ => unreachable!(),
            }
            assert!(probe(records(), net, None, 2).is_err(), "accepted {fault}");
        }
        let mut stopped = records();
        stopped[0]["state"] = json!("stopped");
        assert!(probe(stopped, networks(), None, 1).is_err());
        assert!(management_address("../other", Path::new("/nonexistent"), None).is_err());
        assert!(management_address(
            "controller-lab",
            Path::new("/nonexistent"),
            Some("../other")
        )
        .is_err());
    }
}
