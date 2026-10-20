use super::*;

pub(super) fn print_human(command: &str, envelope: &Value) {
    match command {
        "vm.list" | "ps" => {
            let vms = envelope["vms"].as_array().cloned().unwrap_or_default();
            if vms.is_empty() {
                println!("no VMs");
            } else {
                println!(
                    "{:<16} {:<10} {:<8} {:<18} {}",
                    "ID", "STATE", "PID", "IPS", "BUNDLE"
                );
                for vm in vms {
                    let id = vm["id"].as_str().unwrap_or("?");
                    let state = vm["state"].as_str().unwrap_or("?");
                    let pid = vm["pid"]
                        .as_u64()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let ips = vm["ips"]
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "-".to_string());
                    let bundle = vm["bundle"].as_str().unwrap_or("-");
                    println!("{id:<16} {state:<10} {pid:<8} {ips:<18} {bundle}");
                }
            }
            if let Some(warnings) = envelope["warnings"].as_array() {
                for warning in warnings {
                    if let Some(message) = warning.as_str() {
                        eprintln!("warning: {message}");
                    }
                }
            }
        }
        "vm.start" | "vm.stop" | "vm.restart" | "vm.delete" | "vm.transfer" | "vm.attach"
        | "vm.mount" | "vm.unmount" | "vm.modify" | "vm.agent.upgrade" => {
            println!(
                "{}",
                envelope["summary"]["message"].as_str().unwrap_or(command)
            );
        }
        "vm.mounts" => {
            let mounts = envelope["mounts"].as_array().cloned().unwrap_or_default();
            if mounts.is_empty() {
                println!("no mounts");
            } else {
                println!("{:<16} {:<8} {}", "NAME", "MODE", "TARGET");
                for mount in mounts {
                    println!(
                        "{:<16} {:<8} {} ← {}",
                        mount["name"].as_str().unwrap_or("?"),
                        if mount["read_only"].as_bool() == Some(true) {
                            "ro"
                        } else {
                            "rw"
                        },
                        mount["target"].as_str().unwrap_or("?"),
                        mount["source"].as_str().unwrap_or("?")
                    );
                }
            }
        }
        "vm.inspect" => {
            let vm = &envelope["vm"];
            println!(
                "{}  state={}  pid={}",
                vm["id"].as_str().unwrap_or("?"),
                vm["state"].as_str().unwrap_or("?"),
                vm["pid"]
                    .as_u64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!("  bundle: {}", vm["bundle"].as_str().unwrap_or("-"));
            if let Some(serial) = envelope["logs"]["serial"].as_str() {
                println!("  serial log: {serial}");
            }
            if let Some(networks) = envelope["networks"].as_array() {
                for network in networks {
                    println!(
                        "  network: {}  ip={}",
                        network["name"].as_str().unwrap_or("-"),
                        network["ip"].as_str().unwrap_or("-")
                    );
                }
            }
            println!(
                "  agent: {}  health={}",
                envelope["agent"]["state"].as_str().unwrap_or("unavailable"),
                envelope["agent"]["health"].as_str().unwrap_or("-")
            );
            if let Some(warnings) = envelope["warnings"].as_array() {
                for warning in warnings {
                    if let Some(message) = warning.as_str() {
                        eprintln!("warning: {message}");
                    }
                }
            }
        }
        "vm.logs" => {
            if envelope["restarted"].as_bool() == Some(true) {
                println!(
                    "{}",
                    envelope["summary"]["message"]
                        .as_str()
                        .unwrap_or("restarted")
                );
            }
            if let Some(sources) = envelope["sources"].as_array() {
                for source in sources {
                    if let Some(name) = source["name"].as_str() {
                        println!("{name}");
                    } else if let Some(name) = source.as_str() {
                        println!("{name}");
                    }
                }
            }
            if let Some(lines) = envelope["lines"].as_array() {
                for line in lines {
                    println!("{}", format_iwatch_line(line));
                }
            }
            if let Some(redacted) = envelope["summary"]["redacted"].as_u64() {
                if redacted > 0 {
                    eprintln!("note: redacted {redacted} line(s) that looked like secrets");
                }
            }
        }
        "vm.services" => {
            if let Some(units) = envelope["units"].as_array() {
                println!(
                    "{:<36} {:<10} {:<10} {:<12} {}",
                    "UNIT", "LOAD", "ACTIVE", "SUB", "DESCRIPTION"
                );
                for unit in units {
                    println!(
                        "{:<36} {:<10} {:<10} {:<12} {}",
                        unit["name"].as_str().unwrap_or("-"),
                        unit["load"].as_str().unwrap_or("-"),
                        unit["active"].as_str().unwrap_or("-"),
                        unit["sub"].as_str().unwrap_or("-"),
                        unit["description"].as_str().unwrap_or("-"),
                    );
                }
            } else if let Some(unit) = envelope["unit"].as_object() {
                for key in [
                    "name",
                    "type",
                    "load",
                    "active",
                    "sub",
                    "description",
                    "unit_file",
                    "fragment",
                ] {
                    if let Some(value) = unit.get(key).and_then(Value::as_str) {
                        println!("{key}: {value}");
                    }
                }
            } else if let Some(control) = envelope["control"].as_object() {
                println!(
                    "{} {}",
                    control["action"].as_str().unwrap_or("control"),
                    control["unit"].as_str().unwrap_or("-")
                );
            } else if let Some(payload) = envelope.get("services") {
                let stdout = payload["stdout"].as_str().unwrap_or("");
                let stderr = payload["stderr"].as_str().unwrap_or("");
                if !stdout.is_empty() {
                    print!("{stdout}");
                    if !stdout.ends_with('\n') {
                        println!();
                    }
                }
                if !stderr.is_empty() {
                    eprint!("{stderr}");
                    if !stderr.ends_with('\n') {
                        eprintln!();
                    }
                }
            }
        }
        "vm.exec" => {
            let payload = &envelope["exec"];
            let stdout = payload["stdout"].as_str().unwrap_or("");
            let stderr = payload["stderr"].as_str().unwrap_or("");
            if !stdout.is_empty() {
                print!("{stdout}");
                if !stdout.ends_with('\n') {
                    println!();
                }
            }
            if !stderr.is_empty() {
                eprint!("{stderr}");
                if !stderr.ends_with('\n') {
                    eprintln!();
                }
            }
        }
        "vm.ps" => {
            if let Some(processes) = envelope["processes"].as_array() {
                println!(
                    "{:<8} {:<12} {:<6} {:<6} {}",
                    "PID", "USER", "%CPU", "%MEM", "ARGS"
                );
                for process in processes {
                    if let Some(raw) = process["raw"].as_str() {
                        println!("{raw}");
                        continue;
                    }
                    println!(
                        "{:<8} {:<12} {:<6} {:<6} {}",
                        process["pid"].as_u64().unwrap_or(0),
                        process["user"].as_str().unwrap_or("-"),
                        process["pcpu"].as_str().unwrap_or("-"),
                        process["pmem"].as_str().unwrap_or("-"),
                        process["args"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
        "vm.probe" => {
            println!(
                "{}",
                envelope["summary"]["message"].as_str().unwrap_or("probe")
            );
            let probe = &envelope["probe"];
            if let Some(dns) = probe.get("dns") {
                println!(
                    "  dns  {:<4}  {}  {}ms",
                    if dns["ok"].as_bool() == Some(true) {
                        "OK"
                    } else {
                        "FAIL"
                    },
                    dns["error"].as_str().unwrap_or("-"),
                    dns["connect_ms"].as_i64().unwrap_or(0)
                );
            }
            if let Some(ip) = probe.get("ip") {
                println!(
                    "  ip   {:<4}  {}  {}ms",
                    if ip["ok"].as_bool() == Some(true) {
                        "OK"
                    } else {
                        "FAIL"
                    },
                    ip["chosen_ip"]
                        .as_str()
                        .or(ip["error"].as_str())
                        .unwrap_or("-"),
                    ip["connect_ms"].as_i64().unwrap_or(0)
                );
            }
        }
        "vm.health" => {
            let health = &envelope["health"];
            println!(
                "{}  status={}  queue_depth={}  p99_exec_ms={}",
                envelope["summary"]["vm_id"].as_str().unwrap_or("?"),
                health["status"].as_str().unwrap_or("?"),
                health["queue_depth"],
                health["p99_exec_ms"]
            );
        }
        "vm.stats" => {
            let stats = &envelope["stats"];
            println!(
                "{}  cpu={}  load1={}  mem_used_pct={}",
                envelope["summary"]["vm_id"].as_str().unwrap_or("?"),
                stats["cpu"]["percent"],
                stats["load1"],
                stats
                    .get("mem_used_pct")
                    .cloned()
                    .unwrap_or_else(|| stats["memory"]["percent"].clone())
            );
            if let Some(top) = stats.get("top_process").filter(|value| !value.is_null()) {
                println!(
                    "  top {} pid={} pcpu={}",
                    top["name"].as_str().unwrap_or("?"),
                    top["pid"],
                    top["pcpu"]
                );
            }
        }
        _ => println!("{envelope}"),
    }
}

pub(super) fn emit_failure(format: Format, command: &str, failure: &Failure) {
    let envelope = json!({
        "apiVersion": API_VERSION,
        "command": command,
        "status": "fail",
        "exit_code": failure.code,
        "summary": {
            "message": failure.message,
        },
    });
    match format {
        Format::Json => println!("{envelope}"),
        Format::Human => eprintln!("{}", failure.message),
    }
}
