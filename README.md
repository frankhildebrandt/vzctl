# vzctl

macOS Virtualization.framework–based **devstack supervisor**: Git-native multi-VM environments, custom networks, Hypervisor-DNS, linked clones, Docker context.

> Status: **P3 Alpha** (`hypernetwork/v1` Schema + Validate stehen). See [`docs/planing/`](docs/planing/).

## Quick start

```bash
# Release-Binaries installieren und Supervisor als LaunchAgent aktivieren
make install

# CLI
cargo run -p vzctl -- doctor
cargo run -p vzctl -- events subscribe --filter 'vm.*,apply.*'
cargo run -p vzctl -- validate -C ./examples/edge-dmz

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
Liegen Caddy/Dex unter `daemon/Vendor/` (`make vendor`), kopiert `install`
sie zusätzlich nach `~/Library/Application Support/vzctl/bin/` für Ingress/OIDC.
Falls `~/.local/bin` noch nicht im `PATH` liegt:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Für einen Installations-Test ohne launchd-Aktivierung:

```bash
make install ACTIVATE=0
```

v0.2 Ingress/OIDC-Vendor und UI:

```bash
make vendor          # Caddy + Dex nach daemon/Vendor/
make install-vendor  # → ~/Library/Application Support/vzctl/bin/
make validate        # examples/edge-dmz Schema
make ui-install && make ui-dev
```

## Layout

| Path | Role |
|---|---|
| `crates/vzctl` | Rust CLI (`doctor`, stacks, certs, oidc, …) |
| `daemon/` | Swift `vz-supervisor` + `vz-helper` (ADR 0002) |
| `daemon/Vendor/` | Gepinnte Caddy/Dex-Binaries (`make vendor`) |
| `apps/vzctl-ui/` | Tauri 2 UI (CLI-Wrapper) |
| `guest-agent/` | vsock Guest-Agent |
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
| [hypernetwork/v1](docs/specs/hypernetwork-v1.md) | Config-Schema, Validierung und JSON-Pfade |
| [Event Stream v1](docs/specs/events-v1.md) | NDJSON-Envelope, Typen und Filter |
| [Router und Policies](docs/routes.md) | nftables Forward-Policies, Plan und Status |

## License

[vzctl Public License v1](LICENSE) — AGPL-basierte Source-Available-Lizenz:

- Änderungen müssen veröffentlicht werden (Copyleft / Affero: Corresponding Source bei Distribution und bei Netz-Nutzung einer modifizierten Version)
- kein Verkauf (inkl. Enterprise-/Pro-Builds, bezahlte Forks)
- kein Bundling / OEM / White-Label in kommerzieller Software
- kein bezahltes Hosting / SaaS von vzctl selbst (keine Thin-Wrapper)

Erlaubt: interne Nutzung, auch kommerziell; echte Beratungs-/Support-Dienste.
Rein private interne Mods ohne Weitergabe/Netz-Interaktion für Dritte müssen nicht veröffentlicht werden.
Basis-Copyleft: [GNU AGPLv3](LICENSE.AGPL-3.0) (per Referenz). Keine OSI-/FSF-Open-Source-Lizenz. Sonderrechte nur per kommerzieller Vereinbarung.
