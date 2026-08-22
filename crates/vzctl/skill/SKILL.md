---
name: vzctl
description: >-
  Create and operate vzctl hypercontainers (macOS Virtualization stacks)
  using the vzctl CLI, MCP server (vzctl-mcp), and hypernetwork.config.yaml.
  Use when the user wants a new vzctl stack, hypercontainer, hypernetwork,
  VM lab, agent/MCP integration, or help with vzctl commands, flags, or YAML.
---

# vzctl hypercontainer

A **hypercontainer** is a vzctl stack: one directory with
`hypernetwork.config.yaml` that declares images, networks, and VMs on Apple
Virtualization (macOS 26+, Apple Silicon). That YAML is the source of truth.
Do not invent sidecar formats.

## Discover CLI usage

Do not guess flags. Ask the binary:

```bash
vzctl help                  # command list
vzctl help exit-codes       # stable exit codes
vzctl <command> help        # namespace help (also: vzctl help <command>)
```

Examples: `vzctl net help`, `vzctl vm help`, `vzctl stack help`,
`vzctl image help`, `vzctl apply help`. stdout is data, stderr is diagnostics.
`--format json` uses envelope `apiVersion: vzctl.dev/v1`. Bundled cheat sheet:
[cli.md](cli.md). YAML fields: [yaml.md](yaml.md). Minimal file:
[example.yaml](example.yaml).

## Workflow

Copy this checklist and keep it updated:

```
- [ ] Confirm macOS host + `vzctl doctor` is healthy
- [ ] Scaffold or write `hypernetwork.config.yaml`
- [ ] `vzctl validate -C <dir>` until it passes
- [ ] Pull the base image if missing (`vzctl image pull <alias>`)
- [ ] `vzctl up` or `vzctl apply -C <dir>`
```

1. Ask only for missing intent: project name, VMs, networks, Docker/router,
   ports, mounts, ingress/OIDC. Default to a single `lan` net and one VM.
2. Prefer writing `hypernetwork.config.yaml` directly. Use `vzctl stack …`
   only as a bootstrap (`stack init`) or small mutation.
3. Always `vzctl validate -C <dir>` before apply. Unknown YAML keys fail.
4. Apply bakes/seals the pinned image tag when it is not sealed yet. Pull
   the alias first. Do not skip validate.
5. Use imperative `vzctl vm|net|…` for one-offs and debugging, not as the
   stack source of truth.
6. Guest systemd units: `vzctl vm services <id>` (list/status/start/stop/restart;
   `--type service|timer|socket`). Requires systemd guest + `vm agent upgrade`.
   Unit changes: `vzctl events subscribe --filter 'vm.systemd.*'`.

## MCP server (`vzctl-mcp`)

Stdio MCP server for AI agents (Cursor, Claude Desktop). Stateless tool facade
over Supervisor REST + `vzctl` CLI. Spec: repo `docs/specs/vzctl-mcp-v1.md`.

### Prerequisites

- macOS 26+, `vz-supervisor serve` running (REST on `api.sock`)
- `vzctl` on `PATH` (needed for `vm_exec`, `vm_logs`)
- `vzctl doctor` healthy before relying on MCP tools

### Install

From the vzctl repo:

```bash
make install          # installs ~/.local/bin/vzctl and ~/.local/bin/vzctl-mcp
# or build only:
cargo build -p vzctl-mcp --release
# binary: target/release/vzctl-mcp
```

### Cursor / Claude Desktop config

Project `.cursor/mcp.json` or user MCP settings. Example in
`examples/mcp/cursor-mcp.json`:

```json
{
  "mcpServers": {
    "vzctl": {
      "command": "vzctl-mcp",
      "env": {
        "VZCTL_BIN": "vzctl"
      }
    }
  }
}
```

Use absolute paths if binaries are not on the agent's `PATH`:

```json
"command": "/Users/you/.local/bin/vzctl-mcp",
"env": { "VZCTL_BIN": "/Users/you/.local/bin/vzctl" }
```

Optional env (same as UI):

| Env | Default |
|---|---|
| `VZCTL_API_LISTEN` | `unix:$VZCTL_STATE_DIR/api.sock` |
| `VZCTL_STATE_DIR` | `~/Library/Application Support/vzctl` |
| `VZCTL_BIN` | `vzctl` |

### When to use MCP vs CLI

| Prefer MCP | Prefer CLI |
|---|---|
| Agent needs VM list/inspect/start/stop | Human TTY workflows (`-it`, progress UI) |
| `vm_exec` one-shot in guest (argv array) | `stack up/apply/down` with job dashboard |
| Guest HTTP services (`guest_service_request`) | File transfer, attach, bake/seal |
| `doctor` / `health` / stack status poll | One-off scripting in shell |

MCP tools do **not** replace YAML authoring — still write and `validate`
`hypernetwork.config.yaml` as source of truth.

### MCP conventions

- VM ids: `{project}/{vm}` — pass literally; REST encodes `/` as `%2F`.
- Each tool call is independent (no server-side “current VM”).
- `stack_apply` returns `jobId` — poll with `job_status`.
- `vm_exec`: `command` is argv (no shell); no interactive TTY in MCP v1.
- Tool output is pretty-printed JSON text unless noted.

### Tool groups (v1)

- **Debug:** `doctor`, `health`, `dns_status`, `port_list`
- **VMs:** `vm_list`, `vm_inspect`, `vm_start`, `vm_stop`, `vm_restart`,
  `vm_stats`, `vm_exec`, `vm_logs`
- **Stacks:** `stack_list`, `stack_status`, `stack_validate`, `stack_apply`,
  `job_status`
- **Guest:** `guest_services_list`, `guest_service_request`, `systemd_*`
- **Docker:** `docker_ps`

Protocol: rmcp negotiates MCP version with the client (supports `2026-07-28`
when the client requests it); transport is stdio, not Streamable HTTP.

## Hard rules

- **IPs:** network/broadcast, `.0`, and `.1` are reserved. Router/docker-backend
  owners use `.2`. Guests start at `.10`.
- **DNS:** guest nameserver is bridge `.0:53`. Host resolver is
  `127.0.0.1:15353`. Domain must end with `.vz.test`.
  FQDN: `{vm}.{net}.{project}.vz.test`.
- **Images:** ARM64 cloud disks only. Pin `spec.images.*.tag`. Workflow is
  `pull → bake --tag → seal --tag`. Config `from` is a pull alias; VM `from`
  is an image key.
- **Roles:** only `router` and `docker`. No default passwords. Optional
  `cloudInit` is a path relative to the YAML; system NoCloud wins on scalar
  conflicts.
- **Docker-backend nets:** `backend: docker`, `natEgress: false`, exactly one
  VM with `roles: [docker, router]` at `.2`, plus at least one vmnet NIC.
- **Ports:** Alpha binds `127.0.0.1` only. `0.0.0.0` is invalid.
- **Destructive:** `down --purge` SIGKILLs VMs and deletes managed resources.
  Normal `down` is graceful.
- **Multi-router:** set `policies.*.via` when more than one router could apply.

## Output

Write valid YAML. Then run validate. Fix errors (they include a JSON path)
instead of explaining around them. Do not commit secrets; OIDC secrets live
in host files, never inline `clientSecret`.
