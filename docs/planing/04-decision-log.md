# Decision Log

Entscheidungen aus [Fable-Review](02-fable-review.md) und [GPT-SOL-Review](05-gpt-sol-review.md) (2026-07-30).

## Fable Must-Fixes (übernommen)

| # | Entscheidung | Default |
|---|---|---|
| 1 | Prozessmodell | **Supervisor + 1 Helper-Prozess pro VM** |
| 2 | Guest control plane | **vsock Guest-Agent** first-class; SSH Fallback |
| 3 | DNS-Anbieter | **Hypervisor** autoritativ für interne Zone |
| 4 | Host-Auflösung | **macOS `/etc/resolver/…`** → Hypervisor-DNS |
| 5 | `*.localhost` | Nur Host-Alias in v0.2 — nie OIDC-Issuer |
| 6 | Ingress / OIDC / CA | **v0.2** |
| 7 | Build vs Embed | **Caddy + Dex embedden** |
| 8 | Positionierung | compose für VM-Topologien, nicht OrbStack-Klon |

## GPT-SOL Must-Fixes (übernommen)

| # | Entscheidung | Default |
|---|---|---|
| 9 | Domain | **`{vm}.{net}.{project}.vz.test`** (nicht `.vz`) |
| 10 | DNS Binding | **Dual Listener**: Host Loopback + Guest-erreichbare IP |
| 11 | DNS Forward | Extern ja (Upstream = system, VPN dokumentieren) |
| 12 | `dns query` | Spricht **direkt** vzctl-DNS (nicht nur libc/dig) |
| 13 | Spike Timing | **G0 vor P0** — Go/No-Go für Netz/Entitlements |
| 14 | macOS Baseline | **Fest: Mindestversion macOS 26**; Pre-26 unsupported |
| 15 | Bridged | v0.1 **out of scope** |
| 16 | IP-Precedence | **cloud-init static** Primär; kein wildes DHCP+static |
| 17 | Router-IP | **`.2`** je Net (Host-Bridge/DNS = **`.0`**; `.1` unused) |
| 18 | Isolation | `routes` + **`policies`** (Forward allow/deny) |
| 19 | Ownership ADR | VZ=Helper; vmnet+DNS+Journal=Supervisor; Reconnect spezifiziert |
| 20 | Apply | **Journal + Resume/Abort** — [ADR 0003](../adr/0003-apply-state.md) |
| 21 | Guest-Agent | **In sealed Base vorinstalliert** (nicht First-Boot-Install) |
| 22 | MVP Label | v0.1 = **Alpha**; virtiofs + Docker-Polish → **v0.1.x** |
| 23 | Guest-DNS-IP | Host-Bridge **`.0`** (UDP gemessen); Host-Listener parallel `127.0.0.1` |
| 24 | G0 Verdict | **Go**; ADR 0002+0003 accepted |

## Offen (Spike / ADR)

- Sleep/Wake Clock-Drift (manuelle Prozedur)
- launchd-Plist-Details / XPC vs. UDS für Helper (P0 Implementierung)
- ~~Ownership ADR~~ → [0002](../adr/0002-process-ownership.md)
- ~~Apply Journal Spec~~ → [0003](../adr/0003-apply-state.md)
- ~~Ob Pre-26 jemals supportet wird~~ → **nein** (ADR 0001)

Protokoll: [g0-network.md](../spikes/g0-network.md)

## Umgesetzt

- Entscheidung 18: #33 rendert deklarative `policies:` als atomare nftables-
  Forward-Tabelle auf der Router-VM. Default bleibt `DROP`; Plan und JSON-Status
  laufen über den vorhandenen Helper→Guest-Agent-Pfad aus #32.
