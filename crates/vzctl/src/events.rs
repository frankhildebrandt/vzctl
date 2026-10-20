//! Event subscription and NDJSON streaming contract.

use crate::*;

pub(crate) fn events_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        Some("subscribe") => match parse_events_options(args) {
            Ok(options) => events_subscribe(options),
            Err((message, code)) => {
                eprintln!("{message}");
                ExitCode::from(code)
            }
        },
        Some(command) => {
            eprintln!("unknown events command: {command}");
            ExitCode::from(EXIT_USAGE)
        }
        None => {
            eprintln!("usage: vzctl events subscribe [--filter 'vm.*,apply.*']");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

pub(crate) fn parse_events_options(
    args: impl Iterator<Item = String>,
) -> Result<EventsOptions, (String, u8)> {
    let mut filter = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--filter" => {
                let value = args
                    .next()
                    .ok_or_else(|| ("--filter requires a pattern".to_string(), EXIT_USAGE))?;
                if !valid_event_filter(&value) {
                    return Err((format!("invalid event filter: {value}"), EXIT_INVALID_INPUT));
                }
                filter = Some(value);
            }
            _ => {
                return Err((
                    format!("unknown events subscribe option: {arg}"),
                    EXIT_USAGE,
                ))
            }
        }
    }
    Ok(EventsOptions { filter })
}

pub(crate) fn valid_event_filter(expression: &str) -> bool {
    let patterns = expression.split(',').map(str::trim).collect::<Vec<_>>();
    !patterns.is_empty()
        && patterns.iter().all(|pattern| {
            !pattern.is_empty()
                && pattern.matches('*').count() <= 1
                && (!pattern.contains('*') || pattern.ends_with('*'))
        })
}

pub(crate) fn events_subscribe(options: EventsOptions) -> ExitCode {
    let path = supervisor_socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("supervisor socket {}: {error}", path.display());
            return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
        }
    };
    let timeout = Some(Duration::from_secs(2));
    if let Err(error) = stream.set_write_timeout(timeout) {
        eprintln!("supervisor write timeout setup: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    if let Err(error) = stream.set_read_timeout(timeout) {
        eprintln!("supervisor handshake timeout setup: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    let request = json!({
        "jsonrpc": "2.0",
        "method": "events.subscribe",
        "params": {
            "filter": options.filter,
        },
        "id": 1
    });
    if let Err(error) = writeln!(stream, "{request}") {
        eprintln!("subscribe request: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    if let Err(error) = reader.read_line(&mut response) {
        eprintln!("subscribe response: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }
    let response: Value = match serde_json::from_str(&response) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("invalid subscribe response: {error}");
            return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
        }
    };
    if response["result"]["ok"] != true || response["result"]["v"] != 1 {
        eprintln!("subscribe rejected: {response}");
        return ExitCode::from(EXIT_INVALID_INPUT);
    }

    if let Err(error) = reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(250)))
    {
        eprintln!("supervisor read timeout setup: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    if let Err(error) = ctrlc::set_handler(move || {
        signal_flag.store(true, Ordering::SeqCst);
    }) {
        eprintln!("cannot install Ctrl-C handler: {error}");
        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
    }

    let stdout = io::stdout();
    let mut output = stdout.lock();
    // read_line may append a fragment before a timeout. Retain it until newline.
    let mut line = String::new();
    loop {
        if interrupted.load(Ordering::SeqCst) {
            return ExitCode::SUCCESS;
        }
        match reader.read_line(&mut line) {
            Ok(0) if interrupted.load(Ordering::SeqCst) => return ExitCode::SUCCESS,
            Ok(0) => {
                eprintln!("supervisor closed the event stream");
                return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
            }
            Ok(_) => {
                if !line.ends_with('\n') {
                    eprintln!("incomplete event frame");
                    return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                }
                let event: Value = match serde_json::from_str(&line) {
                    Ok(event) => event,
                    Err(error) => {
                        eprintln!("invalid event JSON: {error}");
                        return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                    }
                };
                if event["v"] != 1
                    || !event["ts"].is_string()
                    || !event["type"].is_string()
                    || !event["data"].is_object()
                {
                    eprintln!("invalid event envelope");
                    return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                }
                if let Err(error) = output
                    .write_all(line.as_bytes())
                    .and_then(|_| output.flush())
                {
                    eprintln!("event output: {error}");
                    return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
                }
                line.clear();
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                eprintln!("event stream: {error}");
                return ExitCode::from(EXIT_SUPERVISOR_UNHEALTHY);
            }
        }
    }
}
