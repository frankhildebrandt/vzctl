use std::process::ExitCode;

const EXIT_USAGE: u8 = 2;

/// True when `token` is a help request (`help`, `-h`, `--help`).
pub(crate) fn is_help(token: &str) -> bool {
    matches!(token, "help" | "-h" | "--help")
}

/// True when the first remaining argument asks for help.
pub(crate) fn first_is_help(args: &[String]) -> bool {
    args.first().map(String::as_str).is_some_and(is_help)
}

/// Print topic help to stdout when the first arg is a help token.
pub(crate) fn handled(args: &[String], topic: &str) -> bool {
    if first_is_help(args) {
        print_topic(topic);
        true
    } else {
        false
    }
}

/// Run `run` unless the remaining args are a help request.
pub(crate) fn with_help(
    topic: &str,
    args: impl Iterator<Item = String>,
    run: impl FnOnce(std::vec::IntoIter<String>) -> ExitCode,
) -> ExitCode {
    let rest: Vec<_> = args.collect();
    if handled(&rest, topic) {
        ExitCode::SUCCESS
    } else {
        run(rest.into_iter())
    }
}

/// `vzctl help [topic]`.
pub(crate) fn command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        None | Some("help") | Some("-h") | Some("--help") => {
            print_root();
            ExitCode::SUCCESS
        }
        Some(topic) => {
            if print_topic(topic) {
                ExitCode::SUCCESS
            } else {
                eprintln!("unknown help topic: {topic}");
                eprintln!("see: vzctl help");
                ExitCode::from(EXIT_USAGE)
            }
        }
    }
}

/// Print help for a command name or `exit-codes`. Returns false if unknown.
pub(crate) fn print_topic(topic: &str) -> bool {
    match topic {
        "exit-codes" | "--exit-codes" => print_exit_codes(),
        "stack" => print_stack(),
        "validate" => print_validate(),
        "plan" => print_reconcile("plan"),
        "diff" => print_reconcile("diff"),
        "up" => print_reconcile("up"),
        "apply" => print_reconcile("apply"),
        "down" => print_reconcile("down"),
        "adopt" => print_reconcile("adopt"),
        "image" => print_image(),
        "vm" => print_vm(),
        "ps" => print_ps(),
        "net" => print_net(),
        "route" => print_route(),
        "dns" => print_dns(),
        "docker" => print_docker(),
        "port" => print_port(),
        "doctor" => print_doctor(),
        "version" => print_version(),
        "services" => print_services(),
        "certs" => print_certs(),
        "oidc" => print_oidc(),
        "events" => print_events(),
        "skill" => print_skill(),
        _ => return false,
    }
    true
}

fn print_root() {
    println!(
        "\
vzctl — Environments-as-Code for macOS Virtualization (Alpha)

Usage:
  vzctl <command> [options]
  vzctl <command> help
  vzctl help [command]
  vzctl help exit-codes

Commands:
  stack            Scaffold/mutate hypernetwork.config.yaml
  validate         Schema + semantics (offline)
  plan, diff       Desired vs actual (no mutate)
  up, apply        Create / reconcile
  down, adopt      Stop / report stale helper locks
  image            Pull, bake, seal ARM64 base images
  vm, ps           VM lifecycle and host process list
  net, route       Networks and guest forwarding
  dns, docker, port
  doctor, version, services, certs, oidc, events
  skill            Print or install the agent skill

Most commands accept --format human|json.
Run `vzctl <command> help` for flags. Exit codes: `vzctl help exit-codes`."
    );
}

fn print_exit_codes() {
    println!(
        "\
vzctl exit codes

  0   success (warnings allowed)
  2   usage or unknown command
  3   invalid input or validation
  5   incomplete apply journal
  6   apply lease held
  10  supervisor socket or health is bad
  11  macOS 26 baseline is not met
  12  command backend unavailable or not implemented
  13  image customization failed
  14  image seal invariant failed
  15  image seal state/marker failed
  16  VM root disk preparation failed
  17  network operation failed
  18  route or guest-agent operation failed
  19  resolver operation failed
  20  DNS query failed or returned a non-zero rcode
  21  image download/metadata network failure
  22  image checksum mismatch or invalid checksum metadata
  23  image architecture unsupported
  24  reconciler or VM lifecycle operation failed
  25  host service lifecycle failed

stdout is data; stderr is diagnostics. JSON uses apiVersion vzctl.dev/v1
(except vm.create: vzctl.dev/v2). WARN stays exit 0 unless documented as hard-fail."
    );
}

fn print_stack() {
    println!(
        "\
vzctl stack — write hypernetwork.config.yaml (source of truth)

  init [DIR] --name <project> [--cidr CIDR] [--force] [-C path]
  vm add <name> [-C path] [--from image-key|pull-alias] [--network net] [--ip addr]
       [--disk SIZE] [--cpus N] [--memory SIZE] [--role router|docker] [--cloud-init path]
  vm remove <name> [-C path]
  net add <name> --cidr CIDR [-C path] [--mode shared|host] [--backend vmnet|docker]
       [--nat-egress|--no-nat-egress]
  net remove <name> [-C path]
  volume add <name> <path> [-C path]
  volume remove <name> [-C path]
  mount add <vm> --source <volume> --target <path> [--read-only] [-C path]
  mount remove <vm> --target <path> [-C path]

Mutations validate then write atomically. Default directory is `.`.
`--format human|json` is accepted on every subcommand."
    );
}

fn print_validate() {
    println!(
        "\
vzctl validate — schema + referential checks (offline)

  vzctl validate [-C <directory|config>] [--format human|json]
  vzctl validate --schema

`-C` is a stack directory or the YAML file. `--schema` prints JSON Schema
Draft 7 to stdout and cannot be combined with `-C` or `--format`."
    );
}

fn print_reconcile(mode: &str) {
    let extra = match mode {
        "up" => " [--force] [--progress plain|ui|off]",
        "apply" => " [--force|--resume|--abort] [--progress plain|ui|off]",
        "down" => " [--purge] [--progress plain|ui|off]",
        _ => "",
    };
    let notes = match mode {
        "plan" | "diff" => {
            "Read-only compare of desired YAML vs supervisor state. Needs a running supervisor."
        }
        "up" => {
            "Create missing resources and start stopped VMs. Does not delete. `--force` skips confirmations."
        }
        "apply" => {
            "Reconcile drift. Breaking VM/net recreate needs confirm or `--force`.\nIncomplete journal: `--resume` or `--abort` (exit 5). Lease held: exit 6."
        }
        "down" => {
            "Graceful stop in reverse dependsOn order. `--purge` SIGKILLs helpers and deletes managed resources."
        }
        "adopt" => "Report-only stale helper locks. No lease, journal, or mutate.",
        _ => "",
    };
    println!(
        "\
vzctl {mode} — stack reconcile

  vzctl {mode} [-C <directory|config>]{extra} [--format human|json]

{notes}"
    );
}

fn print_image() {
    println!(
        "\
vzctl image — ARM64 cloud/server disks (not installer ISOs)

  list [--format human|json]
  pull <alias> [--format human|json]
  bake <alias> --tag <tag> [--format human|json]
  seal <name|path> --tag <tag> [--format human|json]

Aliases include ubuntu-latest, ubuntu-26.04, ubuntu-24.04, ubuntu-22.04,
ubuntu-20.04, debian-latest, debian-13, debian-12, debian-11, alpine-latest,
fedora-latest, rocky-latest, alma-latest, arch-latest, opensuse-latest,
coreos-latest, flatcar-latest, photon-latest, opensuse-microos-latest,
talos-latest.

Workflow: pull → bake --tag → seal --tag. Stack YAML pins `spec.images.*.tag`.
Apply skips bake/seal when that tag is already sealed."
    );
}

fn print_vm() {
    println!(
        "\
vzctl vm — imperative VM lifecycle (prefer YAML + apply for stacks)

  create <id> --from <sealed> --disk <GiB> [--cpus N] [--memory SIZE]
       [--network name] [--role router|docker] [--cloud-init PATH]
       [--project P] [--root-password <secret>] [--mount tag=…,source=…,target=…[,ro]]
  list [--format human|json]
  start|stop|restart|delete|inspect <id>
  stop <id> [--wait true|false]
  delete <id> [--force]
  modify <id> [--cpus N] [--memory SIZE]     (no hotplug; restart needed)
  logs <id> [-f|--follow] [--tail N]
  exec <id> [-it] [--cwd PATH] [--env K=V]... [--timeout-ms N] -- <cmd> [args...]
  transfer <id> <src> <dst>                  (max 256 KiB)
  attach <id>                                (detach: Ctrl-P Ctrl-Q)
  services <id> [start|stop|restart <unit>]
  ps <id>
  mount|unmount|mounts ...
  agent upgrade <id>|--all

Stack runtime IDs are {{project}}/{{vm}}. `--format human|json` where noted.
Interactive exec needs -it together (capability exec_tty)."
    );
}

fn print_ps() {
    println!(
        "\
vzctl ps — host-side VM process overview

  vzctl ps [--format human|json]

Guest process list is `vzctl vm ps <id>`."
    );
}

fn print_net() {
    println!(
        "\
vzctl net — host networks and VM attachments

  create <name> --cidr CIDR [--mode shared] [--nat-egress true|false]
       [--label key=value] [--project P] [--stack S]
  attach <vm> --network <name> --ip <address>
       [--label key=value] [--project P] [--stack S]
  list [--format human|json]
  detach <vm> --network <name>
  delete <name>
  default show
  default set <name> --cidr CIDR

`--mode` is shared in v0.1 (bridged unsupported). Most commands accept
`--format human|json`. Prefer `spec.networks` in YAML + apply for stacks."
    );
}

fn print_route() {
    println!(
        "\
vzctl route — guest nftables forward policy

  apply|plan [--config <path>] [--router <vm-id>] [--format human|json]
  status [--router <vm-id>] [--format human|json]

Reads spec.policies from the environment file (default ./hypernetwork.config.yaml).
`plan` does not mutate the guest. `via` is required when several routers match."
    );
}

fn print_dns() {
    println!(
        "\
vzctl dns — Hypervisor DNS and macOS resolver

  status [--format human|json]
  query <name> [--type A|AAAA|PTR] [--server IP:port] [--format human|json]
  install-resolver|uninstall-resolver [--project P] [--config <path>] [--format human|json]
  install-bind-helper [--allow-uid <uid>]|uninstall-bind-helper [--format human|json]

Guest nameserver is bridge .0:53 (needs bind-helper). Host resolver is
127.0.0.1:15353. Resolver/bind-helper writes need privileges."
    );
}

fn print_docker() {
    println!(
        "\
vzctl docker — Docker via SSH context vzctl-{{project}} (not TCP 2375)

  vzctl docker [--project P] [--format human|json] ps [--all]
  vzctl docker [--project P] [--format human|json] inspect <id>
  vzctl docker [--project P] [--format human|json] start|stop|restart <id>
  vzctl docker [--project P] [--format human|json] run --image <img> [--name N]
       [-e K=V]... [-p host:guest]... [-- <cmd>...]
  vzctl docker [--project P] [--] <docker-args...>

Structured verbs return a JSON envelope. Unknown args pass through to docker.
VM needs roles: [docker]. Infer project from the stack when `--project` is omitted."
    );
}

fn print_port() {
    println!(
        "\
vzctl port — host TCP forwards (127.0.0.1 only)

  list [--project P] [--stack S] [--format human|json]

Declared in YAML as spec.ports (`8080:web:80`) or spec.vms.*.ports (`8080:80`)."
    );
}

fn print_doctor() {
    println!(
        "\
vzctl doctor — host baseline and supervisor health

  vzctl doctor [--format human|json] [--min-free-gib N]

Hard-fail: unhealthy supervisor (10) or macOS < 26 (11). Other issues are WARN / exit 0.
Override minimum free disk with --min-free-gib or VZCTL_DOCTOR_MIN_FREE_GIB."
    );
}

fn print_version() {
    println!(
        "\
vzctl version

  vzctl version [--format human|json]"
    );
}

fn print_services() {
    println!(
        "\
vzctl services — host LaunchAgents (vz-net, vz-edge, vz-supervisor)

  status [--format human|json]
  start|stop|restart [all|net|edge|supervisor] [--format human|json]

Default target is all. Start order: net → edge → supervisor.
Stop all/net: helpers graceful, then supervisor → edge → net (no SIGKILL on vz-net).
dns-bind (root) is `vzctl dns install-bind-helper`, not this command."
    );
}

fn print_certs() {
    println!(
        "\
vzctl certs — local CA and leaf certs

  ca init [--force] [--format human|json]
  ca install [--format human|json]
  mint <san> [--san alias...] [--format human|json]
  fingerprint [--format human|json]
  rollout [--vm NAME] [--format human|json]
  verify --vm NAME --url URL"
    );
}

fn print_oidc() {
    println!(
        "\
vzctl oidc — stack OIDC status

  status [--project P] [--format human|json]
  clients [--project P] [--format human|json]
  token [--client ID]          (use Auth Code + PKCE in the browser)

Config lives in hypernetwork.config.yaml (`oidc:`). Secrets are file refs, never inline."
    );
}

fn print_events() {
    println!(
        "\
vzctl events — NDJSON event stream

  subscribe [--filter 'vm.*,apply.*']

stdout is one JSON object per line. Filter is a comma-separated glob list."
    );
}

fn print_skill() {
    println!(
        "\
vzctl skill — agent skill for hypercontainers

  (no flags)          Print SKILL.md and attachments to stdout
  --install-local     Write/update ./.agents/skills/vzctl
  --install-global    Write/update ~/.agents/skills/vzctl"
    );
}
