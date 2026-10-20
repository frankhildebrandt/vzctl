use super::*;

pub(super) fn parse(
    mut args: impl Iterator<Item = String>,
    ps_top_level: bool,
) -> Result<Options, Failure> {
    let operation = args
        .next()
        .ok_or_else(|| {
            usage(if ps_top_level {
                "usage: vzctl ps [--format human|json]"
            } else {
                "usage: vzctl vm list|start|stop|restart|delete|inspect|logs|exec|transfer|attach|services|ps|mount|unmount|mounts|modify|probe|health|stats|agent ..."
            })
        })?;
    let rest = args.collect::<Vec<_>>();
    match operation.as_str() {
        "list" if !ps_top_level => parse_list(rest, Operation::List),
        "ps" if ps_top_level => parse_list(rest, Operation::HostPs),
        "ps" if !ps_top_level => parse_guest_ps(rest),
        "start" if !ps_top_level => parse_start(rest),
        "stop" if !ps_top_level => parse_stop(rest),
        "restart" if !ps_top_level => parse_restart(rest),
        "delete" if !ps_top_level => parse_delete(rest),
        "inspect" if !ps_top_level => parse_inspect(rest),
        "logs" if !ps_top_level => parse_logs(rest),
        "exec" if !ps_top_level => parse_exec(rest),
        "transfer" if !ps_top_level => parse_transfer(rest),
        "attach" if !ps_top_level => parse_attach(rest),
        "services" if !ps_top_level => parse_services(rest),
        "mount" if !ps_top_level => parse_mount(rest),
        "unmount" if !ps_top_level => parse_unmount(rest),
        "mounts" if !ps_top_level => parse_mounts_list(rest),
        "modify" if !ps_top_level => parse_modify(rest),
        "probe" if !ps_top_level => parse_probe(rest),
        "health" if !ps_top_level => parse_health(rest),
        "stats" if !ps_top_level => parse_stats(rest),
        "agent" if !ps_top_level => parse_agent(rest),
        other => Err(usage(if ps_top_level {
            format!("unknown ps option: {other}")
        } else {
            format!("unknown vm command: {other}")
        })),
    }
}

pub(super) fn parse_list(args: Vec<String>, operation: Operation) -> Result<Options, Failure> {
    let (format, flags) = parse_flags(&args, &[])?;
    if !flags.is_empty() {
        return Err(usage(format!(
            "{} accepts only --format human|json",
            match operation {
                Operation::HostPs => "ps",
                _ => "vm list",
            }
        )));
    }
    Ok(Options { operation, format })
}

pub(super) fn parse_start(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm start requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm start accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Start { id },
        format,
    })
}

pub(super) fn parse_stop(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm stop requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &["--wait"])?;
    let wait = match flags.get("--wait").map(String::as_str) {
        None => true,
        Some("true" | "1" | "yes") => true,
        Some("false" | "0" | "no") => false,
        Some(other) => return Err(usage(format!("--wait requires true|false (got {other})"))),
    };
    // bare --wait without value is treated as presence via dedicated parse below
    Ok(Options {
        operation: Operation::Stop { id, wait },
        format,
    })
}

pub(super) fn parse_restart(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm restart requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm restart accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Restart { id },
        format,
    })
}

pub(super) fn parse_delete(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm delete requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut force = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--force" => {
                force = true;
                index += 1;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(usage(format!("unsupported vm format: {other}")));
                    }
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm delete option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm delete argument: {other}")));
            }
        }
    }
    Ok(Options {
        operation: Operation::Delete { id, force },
        format,
    })
}

pub(super) fn parse_inspect(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm inspect requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm inspect accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Inspect { id },
        format,
    })
}

pub(super) fn parse_health(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm health requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm health accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Health { id },
        format,
    })
}

pub(super) fn parse_stats(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm stats requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm stats accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Stats { id },
        format,
    })
}

pub(super) fn parse_probe(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm probe requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut target = None;
    let mut via = ProbeVia::Both;
    let mut timeout_ms = 5_000_u64;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--target requires host:port"))?;
                if value.is_empty() || !value.contains(':') {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        format!("invalid --target: {value} (expected host:port)"),
                    ));
                }
                target = Some(value.clone());
                index += 2;
            }
            "--via" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--via requires dns, ip, or both"))?;
                via = ProbeVia::parse(value)?;
                index += 2;
            }
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--timeout-ms requires an integer"))?;
                timeout_ms = value.parse::<u64>().map_err(|_| {
                    Failure::new(EXIT_INVALID, format!("invalid --timeout-ms: {value}"))
                })?;
                if !(100..=30_000).contains(&timeout_ms) {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        "--timeout-ms must be 100...30000",
                    ));
                }
                index += 2;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm probe option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm probe argument: {other}")));
            }
        }
    }
    let target = target.ok_or_else(|| usage("vm probe requires --target HOST:PORT"))?;
    Ok(Options {
        operation: Operation::Probe {
            id,
            target,
            via,
            timeout_ms,
        },
        format,
    })
}

pub(super) fn parse_modify(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm modify requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut cpus = None;
    let mut memory_mib = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--cpus" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--cpus requires a positive integer"))?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| Failure::new(EXIT_INVALID, format!("invalid --cpus: {value}")))?;
                if parsed == 0 {
                    return Err(Failure::new(
                        EXIT_INVALID,
                        "--cpus must be greater than zero",
                    ));
                }
                cpus = Some(parsed);
                index += 2;
            }
            "--memory" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--memory requires a size"))?;
                memory_mib = Some(
                    crate::parse_memory_mib(value)
                        .map_err(|message| Failure::new(EXIT_INVALID, message))?,
                );
                index += 2;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(usage(format!("unsupported vm format: {other}")));
                    }
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm modify option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm modify argument: {other}")));
            }
        }
    }
    if cpus.is_none() && memory_mib.is_none() {
        return Err(usage(
            "vm modify requires at least one of --cpus or --memory",
        ));
    }
    Ok(Options {
        operation: Operation::Modify {
            id,
            cpus,
            memory_mib,
        },
        format,
    })
}

pub(super) fn parse_agent(args: Vec<String>) -> Result<Options, Failure> {
    let sub = positional(&args, "vm agent requires a subcommand (upgrade)")?;
    if sub != "upgrade" {
        return Err(usage(format!("unknown vm agent subcommand: {sub}")));
    }
    let mut format = Format::Human;
    let mut all = false;
    let mut id = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                all = true;
                index += 1;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => {
                        return Err(usage(format!("unsupported vm format: {other}")));
                    }
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm agent option: {other}")));
            }
            other => {
                if id.is_some() {
                    return Err(usage(format!("unexpected vm agent argument: {other}")));
                }
                validate_vm_id(other)?;
                id = Some(other.to_string());
                index += 1;
            }
        }
    }
    if all && id.is_some() {
        return Err(usage("vm agent upgrade accepts either --all or a VM id"));
    }
    if !all && id.is_none() {
        return Err(usage("vm agent upgrade requires a VM id or --all"));
    }
    Ok(Options {
        operation: Operation::AgentUpgrade { all, id },
        format,
    })
}

pub(super) fn parse_mount(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm mount requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut source = None;
    let mut target = None;
    let mut name = None;
    let mut read_only = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                source = Some(PathBuf::from(require_value(&args, &mut index, "--source")?));
            }
            "--target" => {
                target = Some(require_value(&args, &mut index, "--target")?);
            }
            "--tag" | "--name" => {
                name = Some(require_value(&args, &mut index, "--tag")?);
            }
            "--ro" | "--read-only" => {
                read_only = true;
                index += 1;
            }
            "--format" => {
                format = parse_format_value(&require_value(&args, &mut index, "--format")?)?;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm mount option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm mount argument: {other}")));
            }
        }
    }
    let source = source.ok_or_else(|| usage("vm mount requires --source PATH"))?;
    let target = target.ok_or_else(|| usage("vm mount requires --target PATH"))?;
    Ok(Options {
        operation: Operation::Mount {
            id,
            source,
            target,
            name,
            read_only,
        },
        format,
    })
}

pub(super) fn parse_unmount(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm unmount requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut target = None;
    let mut name = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                target = Some(require_value(&args, &mut index, "--target")?);
            }
            "--tag" | "--name" => {
                name = Some(require_value(&args, &mut index, "--tag")?);
            }
            "--format" => {
                format = parse_format_value(&require_value(&args, &mut index, "--format")?)?;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm unmount option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm unmount argument: {other}")));
            }
        }
    }
    if target.is_none() && name.is_none() {
        return Err(usage("vm unmount requires --target or --tag"));
    }
    Ok(Options {
        operation: Operation::Unmount { id, target, name },
        format,
    })
}

pub(super) fn parse_mounts_list(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm mounts requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm mounts accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::Mounts { id },
        format,
    })
}

pub(super) fn require_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<String, Failure> {
    if *index + 1 >= args.len() {
        return Err(usage(format!("{flag} requires a value")));
    }
    let value = args[*index + 1].clone();
    *index += 2;
    Ok(value)
}

pub(super) fn parse_format_value(value: &str) -> Result<Format, Failure> {
    match value {
        "human" => Ok(Format::Human),
        "json" => Ok(Format::Json),
        other => Err(usage(format!("unsupported format: {other}"))),
    }
}

pub(super) fn parse_logs(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm logs requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut follow = false;
    let mut tail = DEFAULT_LOG_TAIL;
    let mut source = "serial".to_string();
    let mut list_sources = false;
    let mut q = None;
    let mut min_level = None;
    let mut group_field = None;
    let mut group_value = None;
    let mut filters = Vec::new();
    let mut restart = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "-f" | "--follow" => {
                follow = true;
                index += 1;
            }
            "--tail" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--tail requires a line count"))?;
                tail = value
                    .parse::<usize>()
                    .map_err(|_| usage(format!("invalid --tail value: {value}")))?;
                index += 2;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            "--source" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--source requires a name"))?;
                source = value.clone();
                index += 2;
            }
            "--list-sources" => {
                list_sources = true;
                index += 1;
            }
            "--restart" => {
                restart = true;
                index += 1;
            }
            "--q" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--q requires a query"))?;
                q = Some(value.clone());
                index += 2;
            }
            "--min-level" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--min-level requires a level"))?;
                min_level = Some(value.clone());
                index += 2;
            }
            "--group-field" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--group-field requires a field"))?;
                group_field = Some(value.clone());
                index += 2;
            }
            "--group-value" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--group-value requires a value"))?;
                group_value = Some(value.clone());
                index += 2;
            }
            "--filter" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--filter requires k=v"))?;
                let (key, val) = value
                    .split_once('=')
                    .ok_or_else(|| usage(format!("invalid --filter {value}: expected k=v")))?;
                if key.is_empty() {
                    return Err(usage("invalid --filter: empty key"));
                }
                filters.push((key.to_string(), val.to_string()));
                index += 2;
            }
            other => {
                return Err(usage(format!("unknown vm logs option: {other}")));
            }
        }
    }
    if follow && format == Format::Json {
        return Err(Failure::new(
            EXIT_INVALID,
            "vm logs --follow supports only --format human",
        ));
    }
    let iwatch_filters = q.is_some()
        || min_level.is_some()
        || group_field.is_some()
        || group_value.is_some()
        || !filters.is_empty();
    if iwatch_filters && (source == "serial" && !list_sources) {
        return Err(Failure::new(
            EXIT_INVALID,
            "iwatch filters require --source <name>",
        ));
    }
    if restart && (source == "serial" || list_sources) {
        return Err(Failure::new(
            EXIT_INVALID,
            "vm logs --restart requires --source <name>",
        ));
    }
    Ok(Options {
        operation: Operation::Logs {
            id,
            follow,
            tail,
            source,
            list_sources,
            q,
            min_level,
            group_field,
            group_value,
            filters,
            restart,
        },
        format,
    })
}

pub(super) fn parse_attach(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm attach requires a VM id")?;
    validate_vm_id(&id)?;
    if args.len() > 1 {
        return Err(usage("vm attach accepts only a VM id"));
    }
    Ok(Options {
        operation: Operation::Attach { id },
        format: Format::Human,
    })
}

pub(super) fn parse_guest_ps(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm ps requires a VM id")?;
    validate_vm_id(&id)?;
    let (format, flags) = parse_flags(&args[1..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm ps accepts only --format human|json"));
    }
    Ok(Options {
        operation: Operation::GuestPs { id },
        format,
    })
}

pub(super) fn parse_services(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm services requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut action = ServicesAction::List;
    let mut unit_type = "service".to_string();
    let mut all = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            "--type" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--type requires service, timer, or socket"))?;
                unit_type = match value.as_str() {
                    "service" | "timer" | "socket" => value.to_string(),
                    other => {
                        return Err(usage(format!("unsupported vm services unit type: {other}")));
                    }
                };
                index += 2;
            }
            "--all" => {
                all = true;
                index += 1;
            }
            "status" => {
                let unit = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| usage("vm services status requires a unit"))?
                    .clone();
                action = ServicesAction::Status(unit);
                index += 2;
            }
            "start" | "stop" | "restart" => {
                let verb = args[index].clone();
                let unit = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| usage(format!("vm services {verb} requires a unit")))?
                    .clone();
                action = match verb.as_str() {
                    "start" => ServicesAction::Start(unit),
                    "stop" => ServicesAction::Stop(unit),
                    _ => ServicesAction::Restart(unit),
                };
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm services option: {other}")));
            }
            other => {
                return Err(usage(format!("unexpected vm services argument: {other}")));
            }
        }
    }
    Ok(Options {
        operation: Operation::Services {
            id,
            action,
            unit_type,
            all,
        },
        format,
    })
}

pub(super) fn parse_exec(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm exec requires a VM id")?;
    validate_vm_id(&id)?;
    let mut format = Format::Human;
    let mut cwd = None;
    let mut env = BTreeMap::new();
    let mut timeout_ms = 30_000_u64;
    let mut interactive = false;
    let mut tty = false;
    let mut index = 1;
    let mut cmd = Vec::new();
    while index < args.len() {
        match args[index].as_str() {
            "--" => {
                cmd.extend(args[index + 1..].iter().cloned());
                break;
            }
            "-i" | "--interactive" => {
                interactive = true;
                index += 1;
            }
            "-t" | "--tty" => {
                tty = true;
                index += 1;
            }
            "-it" | "-ti" => {
                interactive = true;
                tty = true;
                index += 1;
            }
            "--format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--format requires human or json"))?;
                format = match value.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unsupported vm format: {other}"))),
                };
                index += 2;
            }
            "--cwd" => {
                cwd = Some(
                    args.get(index + 1)
                        .ok_or_else(|| usage("--cwd requires a path"))?
                        .clone(),
                );
                index += 2;
            }
            "--env" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--env requires KEY=VALUE"))?;
                let (key, value) = value
                    .split_once('=')
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or_else(|| usage(format!("--env must be KEY=VALUE: {value}")))?;
                env.insert(key.to_string(), value.to_string());
                index += 2;
            }
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage("--timeout-ms requires a value"))?;
                timeout_ms = value.parse::<u64>().map_err(|_| {
                    Failure::new(EXIT_INVALID, format!("invalid --timeout-ms: {value}"))
                })?;
                if timeout_ms == 0 {
                    return Err(Failure::new(EXIT_INVALID, "--timeout-ms must be > 0"));
                }
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown vm exec option: {other}")));
            }
            _ => {
                cmd.extend(args[index..].iter().cloned());
                break;
            }
        }
    }
    if cmd.is_empty() {
        return Err(usage("vm exec requires a command"));
    }
    if interactive != tty {
        return Err(Failure::new(
            EXIT_INVALID,
            "vm exec interactive tty requires both -i/--interactive and -t/--tty (use -it)",
        ));
    }
    Ok(Options {
        operation: Operation::Exec {
            id,
            cmd,
            cwd,
            env,
            timeout_ms,
            interactive,
            tty,
        },
        format,
    })
}

pub(super) fn parse_transfer(args: Vec<String>) -> Result<Options, Failure> {
    let id = positional(&args, "vm transfer requires a VM id")?;
    validate_vm_id(&id)?;
    let src_raw = args
        .get(1)
        .ok_or_else(|| usage("vm transfer requires <src> <dst>"))?
        .clone();
    let dst_raw = args
        .get(2)
        .ok_or_else(|| usage("vm transfer requires <src> <dst>"))?
        .clone();
    let (format, flags) = parse_flags(&args[3..], &[])?;
    if !flags.is_empty() {
        return Err(usage("vm transfer accepts only --format human|json"));
    }
    let src = parse_transfer_path(&id, &src_raw)?;
    let dst = parse_transfer_path(&id, &dst_raw)?;
    match (&src, &dst) {
        (TransferPath::Host(_), TransferPath::Guest(_))
        | (TransferPath::Guest(_), TransferPath::Host(_)) => {}
        _ => {
            return Err(Failure::new(
                EXIT_INVALID,
                "vm transfer requires one host path and one <id>:<guest-path>",
            ));
        }
    }
    Ok(Options {
        operation: Operation::Transfer { id, src, dst },
        format,
    })
}

pub(super) fn parse_transfer_path(vm_id: &str, value: &str) -> Result<TransferPath, Failure> {
    if let Some((prefix, path)) = value.split_once(':') {
        if prefix == vm_id {
            if path.is_empty() {
                return Err(Failure::new(EXIT_INVALID, "guest path must not be empty"));
            }
            return Ok(TransferPath::Guest(path.to_string()));
        }
    }
    Ok(TransferPath::Host(PathBuf::from(value)))
}

pub(super) fn parse_flags(
    args: &[String],
    valued: &[&str],
) -> Result<(Format, BTreeMap<String, String>), Failure> {
    let mut format = Format::Human;
    let mut values = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        if flag == "--wait" && valued.contains(&"--wait") {
            // Optional value: bare --wait means true.
            let next = args.get(index + 1).map(String::as_str);
            match next {
                Some(value) if !value.starts_with('-') => {
                    values.insert("--wait".to_string(), value.to_string());
                    index += 2;
                }
                _ => {
                    values.insert("--wait".to_string(), "true".to_string());
                    index += 1;
                }
            }
            continue;
        }
        if flag != "--format" && !valued.contains(&flag.as_str()) {
            return Err(usage(format!("unknown vm option: {flag}")));
        }
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with('-'))
            .ok_or_else(|| usage(format!("{flag} requires a value")))?
            .clone();
        if flag == "--format" {
            format = match value.as_str() {
                "human" => Format::Human,
                "json" => Format::Json,
                other => return Err(usage(format!("unsupported vm format: {other}"))),
            };
        } else if values.insert(flag.clone(), value).is_some() {
            return Err(usage(format!("duplicate option: {flag}")));
        }
        index += 2;
    }
    Ok((format, values))
}

pub(super) fn positional(args: &[String], message: &str) -> Result<String, Failure> {
    match args.first() {
        Some(value) if !value.starts_with('-') && !value.is_empty() => Ok(value.clone()),
        _ => Err(usage(message)),
    }
}

pub(super) fn validate_vm_id(id: &str) -> Result<(), Failure> {
    if crate::valid_vm_id(id) {
        Ok(())
    } else {
        Err(Failure::new(
            EXIT_INVALID,
            format!("invalid VM id: {id} (flat label or project/vm, alphanumeric/._-)"),
        ))
    }
}

pub(super) fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format")
        .and_then(|pair| (pair[1] == "json").then_some(Format::Json))
        .unwrap_or(Format::Human)
}
