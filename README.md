# vzctl

macOS Virtualization.framework–based **devstack supervisor**: Git-native multi-VM environments, custom networks, Hypervisor-DNS, linked clones, Docker context.

> Status: **P1 Alpha** (CLI/Event-Verträge stehen). See [`docs/planing/`](docs/planing/).

## Quick start

```bash
# Release-Binaries installieren und Supervisor als LaunchAgent aktivieren
make install

# CLI
cargo run -p vzctl -- doctor
cargo run -p vzctl -- events subscribe --filter 'vm.*,apply.*'

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

`make install` installiert `vzctl`, `vz-supervisor` und `vz-helper`
benutzerlokal nach `~/.local/bin`. Der Supervisor wird als
`~/Library/LaunchAgents/com.vzctl.supervisor.plist` registriert und sofort
gestartet. Bei einer bestehenden Installation werden die geprüften Binaries
atomar ersetzt und der Supervisor neu gestartet. Laufende VM-Helper werden
nicht beendet; neue Helper-Prozesse verwenden sofort die aktualisierte Binary.
Falls `~/.local/bin` noch nicht im `PATH` liegt:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Für einen Installations-Test ohne launchd-Aktivierung:

```bash
make install ACTIVATE=0
```

## Layout

| Path | Role |
|---|---|
| `crates/vzctl` | Rust CLI (`doctor`, `events subscribe`, `apply` stub) |
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
| [CLI Contract v1](docs/specs/cli-contract-v1.md) | JSON, stdout/stderr und Exitcodes |
| [Event Stream v1](docs/specs/events-v1.md) | NDJSON-Envelope, Typen und Filter |
| [Router und Policies](docs/routes.md) | nftables Forward-Policies, Plan und Status |

## License

Private repository. All rights reserved.
