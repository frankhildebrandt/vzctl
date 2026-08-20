---
name: vzctl
description: >-
  Create and operate vzctl hypercontainers (macOS Virtualization stacks)
  using the vzctl CLI and hypernetwork.config.yaml. Use when the user wants
  a new vzctl stack, hypercontainer, hypernetwork, VM lab, or help with
  vzctl commands, flags, or YAML.
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
