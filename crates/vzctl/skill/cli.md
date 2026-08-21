# vzctl CLI

Discover live usage from the binary (do not invent flags):

```bash
vzctl help
vzctl help exit-codes
vzctl <command> help          # same as: vzctl help <command>
```

Namespaces with their own help: `stack`, `image`, `vm`, `net`, `route`, `dns`,
`docker`, `port`, `services`, `certs`, `oidc`, `events`, `skill`, plus
`validate`, `plan`, `diff`, `up`, `apply`, `down`, `adopt`, `doctor`, `ps`,
`version`.

stdout = data, stderr = diagnostics. Human output is default; `--format json`
uses envelope `apiVersion: vzctl.dev/v1`. Host baseline: macOS 26, Apple Silicon.


## Hypercontainer lifecycle

```bash
vzctl stack init [DIR] --name <project> [--cidr CIDR] [--force]
vzctl stack vm add <name> [--from image-key|alias] [--network net] [--ip addr] \
  [--disk SIZE] [--cpus N] [--memory SIZE] [--role router|docker] [--cloud-init path]
vzctl stack vm remove <name>
vzctl stack net add <name> --cidr CIDR [--mode shared|host] [--backend vmnet|docker] \
  [--nat-egress|--no-nat-egress]
vzctl stack net remove <name>
vzctl stack volume add <name> <path>
vzctl stack volume remove <name>
vzctl stack mount add <vm> --source <volume> --target <path> [--read-only]
vzctl stack mount remove <vm> --target <path>

vzctl stack status [-C dir] [--format human|json] [--verbose]
vzctl stack watch [-C dir] [--filter glob] [--interval sec]
vzctl status -C dir                                 # alias of stack status

vzctl validate [-C <dir|file>]          # offline schema + refs
vzctl validate --schema                 # JSON Schema to stdout
vzctl plan|diff [-C path]
vzctl up [-C path] [--force] [--progress plain|ui|off]
vzctl apply [-C path] [--force|--resume|--abort] [--progress plain|ui|off]
vzctl down [-C path] [--purge]
vzctl adopt [-C path]                   # report-only stale helper locks
```

`-C` / `--config` is a stack directory or the YAML file. Default: `.`.
`stack` mutates only `hypernetwork.config.yaml` (atomic write after validate).

| Command | Effect |
|---|---|
| `validate` | schema + semantics, no supervisor |
| `plan` / `diff` | desired vs actual, no mutate |
| `up` | create missing, start stopped; no deletes |
| `apply` | reconcile drift; breaking VM/net recreate needs confirm or `--force` |
| `down` | graceful stop (reverse dependsOn) |
| `down --purge` | SIGKILL helpers, delete managed resources |
| `adopt` | report stale locks only |
| `stack status` | aggregate VM/agent/DNS/route health; exit 0 ok, 1 degraded, 2 critical |

Incomplete apply journal → `--resume` or `--abort` (exit 5). Lease held → exit 6.

Typical first run:

```bash
vzctl stack init ./lab --name lab
# edit lab/hypernetwork.config.yaml
vzctl validate -C ./lab
vzctl image pull ubuntu-latest
vzctl up -C ./lab
```

## Images

```bash
vzctl image list
vzctl image pull <alias>
vzctl image bake <alias> --tag <tag>
vzctl image seal <name|path> --tag <tag>
```

ARM64 cloud/server disks only (not installer ISOs). Apply bakes/seals the
pinned tag when it is not already sealed.

## VMs (imperative)

Prefer the YAML + apply. Imperative create is for one-off VMs:

```bash
vzctl vm create <id> --from <sealed> --disk <GiB> [--cpus N] [--memory SIZE] \
  [--network name] [--role router|docker] [--cloud-init PATH] [--project P] \
  [--root-password <secret>]
vzctl vm list|start|stop|restart|delete|inspect|logs|ps|services <id>
vzctl vm modify <id> [--cpus N] [--memory SIZE]   # no hotplug; restart needed
vzctl vm exec <id> [-it] [--cwd PATH] [--env K=V]... -- <cmd> [args...]
vzctl vm probe <id> --target HOST:PORT [--via dns|ip|both]
vzctl vm health <id>
vzctl vm stats <id>
vzctl vm transfer <id> <src> <dst>                # max 256 KiB
vzctl vm attach <id>                              # detach: Ctrl-P Ctrl-Q
vzctl vm mount|unmount|mounts ...
vzctl vm agent upgrade <id>|--all
vzctl ps
```

Stack runtime IDs are `{project}/{vm}`. Config keys stay short (`web`).
`--project` prefixes a flat create id. Interactive `exec -it` needs guest
capability `exec_tty`.

## Network, DNS, Docker, host

```bash
vzctl net create|attach|list|detach|delete
vzctl net default show|set <name> --cidr CIDR
vzctl route apply|plan|status [--router <vm-id>]
vzctl dns status|query <name>
vzctl dns install-resolver|uninstall-resolver [--project P]
vzctl dns install-bind-helper|uninstall-bind-helper
vzctl docker [--project P] ps|inspect|start|stop|restart|run ...
vzctl port list
vzctl services status|start|stop|restart [all|net|edge|supervisor]
vzctl doctor [--stack|-C dir]
vzctl certs ca init|install
vzctl oidc status|clients [--project P]
```

DNS resolver and bind-helper need privileges (`Permission denied` otherwise).
`apply`/UI elevate via the admin dialog. Guest DNS is `.0:53` via `vz-dns-bind`.

Start order: `vz-net` → `vz-edge` → `vz-supervisor`.
`docker` uses SSH context `vzctl-{project}` (not TCP 2375). Host forwards listen
on `127.0.0.1`.

## Agent skill

```bash
vzctl skill                     # print SKILL.md + attachments to stdout
vzctl skill --install-local     # ./.agents/skills/vzctl
vzctl skill --install-global    # ~/.agents/skills/vzctl
vzctl skill help
```

## Exit codes

Use `vzctl help exit-codes`. Short map: `0` ok (warnings allowed), `1` stack degraded,
`2` usage or stack critical, `3` invalid, `5` journal, `6` lease, `10` supervisor, `11` host, `12` unavailable,
`13-16` image/disk, `17` net, `18` route/agent, `19` resolver, `20` DNS query,
`21-23` image fetch, `24` reconciler, `25` services.
