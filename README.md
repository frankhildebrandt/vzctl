# vzctl

macOS Virtualization.framework–based **devstack supervisor**: Git-native multi-VM environments, custom networks, Hypervisor-DNS, linked clones, Docker context.

> Status: **P0 scaffolding** (G0–G3 gates closed). See [`docs/planing/`](docs/planing/).

## Quick start

```bash
# CLI
cargo run -p vzctl -- doctor

# Daemon stubs (macOS 26+)
cd daemon && swift build
.build/debug/vz-supervisor doctor
.build/debug/vz-helper version
```

## Layout

| Path | Role |
|---|---|
| `crates/vzctl` | Rust CLI (`doctor`, `apply` stub) |
| `daemon/` | Swift `vz-supervisor` + `vz-helper` (ADR 0002) |
| `docs/adr/` | Accepted ADRs (macOS 26, process ownership, apply) |
| `docs/spikes/g0-network.md` | G0 network/DNS/crash spike |
| `spikes/g0/` | Measurement harness |

## Docs

| Document | Description |
|---|---|
| [Planning index](docs/planing/README.md) | Übersicht |
| [Implementation plan](docs/planing/01-implementation-plan.md) | Phasen + Gates |
| [Decision log](docs/planing/04-decision-log.md) | Architektur-Entscheidungen |
| [ADR 0002](docs/adr/0002-process-ownership.md) | Supervisor vs Helper |
| [ADR 0003](docs/adr/0003-apply-state.md) | Apply / Journal |

## License

Private repository. All rights reserved.
