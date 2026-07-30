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

`vzctl doctor` prüft macOS, Codesigning/Entitlements, APFS, freien Disk-Space,
DNS-Port/Resolver und Supervisor-Health. Warnungen bleiben Exit 0; harte
Exitcodes und Abhilfen stehen in der
[Doctor-Interpretationshilfe](docs/doctor.md). Supervisor-State liegt
standardmäßig unter `~/Library/Application Support/vzctl/`;
`VZCTL_STATE_DIR` überschreibt den Pfad für Entwicklung und Tests.

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
| [`vzctl doctor`](docs/doctor.md) | Checks, Konfiguration und Exitcodes |

## License

Private repository. All rights reserved.
