use std::process::Command;

fn vzctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vzctl"))
}

fn help_stdout(args: &[&str]) -> String {
    let output = vzctl().args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{} => {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn root_help_is_short_and_points_to_topics() {
    let stdout = help_stdout(&["help"]);
    assert!(stdout.contains("vzctl <command> help"));
    assert!(stdout.contains("help exit-codes"));
    assert!(stdout.contains("skill"));
    assert!(stdout.contains("services"));
    assert!(
        !stdout.contains("Stable exit codes:"),
        "exit codes belong on `vzctl help exit-codes`"
    );
}

#[test]
fn help_exit_codes_lists_table() {
    let stdout = help_stdout(&["help", "exit-codes"]);
    assert!(stdout.contains("  0   success"));
    assert!(stdout.contains("  25  host service lifecycle failed"));
    let via_flag = help_stdout(&["help", "--exit-codes"]);
    assert!(via_flag.contains("  10  supervisor"));
}

#[test]
fn namespace_help_matches_help_topic() {
    for command in [
        "net", "vm", "image", "stack", "dns", "docker", "route", "port", "services", "certs",
        "oidc", "events", "skill", "validate", "apply", "doctor",
    ] {
        let via_ns = help_stdout(&[command, "help"]);
        let via_topic = help_stdout(&["help", command]);
        assert_eq!(via_ns, via_topic, "{command} help mismatch");
        assert!(
            via_ns.to_ascii_lowercase().contains(command),
            "{command} help should name the command"
        );
    }
}

#[test]
fn net_help_lists_subcommands() {
    let stdout = help_stdout(&["net", "help"]);
    assert!(stdout.contains("create <name> --cidr CIDR"));
    assert!(stdout.contains("attach <vm>"));
    assert!(stdout.contains("default show"));
}

#[test]
fn stack_vm_help_prints_stack_help() {
    let stdout = help_stdout(&["stack", "vm", "help"]);
    assert!(stdout.contains("vm add"));
    assert!(stdout.contains("net add"));
}

#[test]
fn unknown_help_topic_is_usage() {
    let output = vzctl().args(["help", "nope"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown help topic: nope"));
}
