# ADR 0002: Process- & Ressourcen-Ownership

- **Status:** Accepted (amended 2026-08-01)
- **Date:** 2026-07-30
- **Amended:** 2026-08-01 — HyperNetwork Supervisor (`vz-net`) owns vmnet refs
- **Issues:** [#7](https://github.com/frankhildebrandt/vzctl/issues/7), [#8](https://github.com/frankhildebrandt/vzctl/issues/8), G0 [#6](https://github.com/frankhildebrandt/vzctl/issues/6)
- **Spike:** [docs/spikes/g0-network.md](../spikes/g0-network.md)
- **Contract:** [docs/specs/vz-net-v1.md](../specs/vz-net-v1.md)

## Context

G0 hat gezeigt:

1. `vmnet_network_ref` ist process-local; Lifetime = Reservation.
2. `kill -9` auf den Prozess, der Net+VM hält → **VM tot**, Bridge weg, **Subnet verbrannt** (`network_create` gleicher Range → `FAILURE 1001`); frische Range OK.
3. `vmnet_stop_interface` allein reicht **nicht**, solange der `vmnet_network_ref` retained bleibt.
4. Sauberes Prozessende / Ref-Release ermöglicht Recreate.
5. DNS auf Host-Bridge-`.0` (UDP) braucht eine lebende Host-Bridge — Listener können im Control-Plane-Supervisor liegen.

Zielbild: Control-Plane-Supervisor + **1 Helper/VM** + **minimaler `vz-net`**, damit VMs und CIDR-Reservierungen Control-Plane-Crashes überleben.

## Decision

| Ressource | Owner | Bei Control-Plane-Crash | Cleanup |
|---|---|---|---|
| `VZVirtualMachine` | **VM-Helper** (eigener Prozess) | VM läuft weiter | Helper stoppt VM nur auf Befehl / Purge |
| `vmnet_network_ref` + Host-Bridge + Serialize | **`vz-net`** (HyperNetwork Supervisor) | Netze bleiben; CP reconnect via `net.acquire` (idempotent) | `vz-net` releast Refs nur bei `net.release` oder sauberem SIGTERM |
| Desired-State (SQLite networks/attachments) | **Control-Plane** (`vz-supervisor`) | Ledger bleibt; Runtime-Rebuild via `vz-net` | — |
| DNS Zone + UDP-Listener auf `.0` (+ Host `127.0.0.1`) | **Control-Plane** | DNS down bis Restart | Listener schließen; Bridge bleibt bei `vz-net` |
| Apply-Journal / Lease | **Control-Plane** | incomplete → `apply --resume\|--abort` | — |

### HyperNetwork Supervisor (`vz-net`)

- LaunchAgent `com.vzctl.net`, KeepAlive, Entitlement `com.apple.security.virtualization`.
- UDS `net.sock` unter `$VZCTL_STATE_DIR`; Contract [vz-net-v1](../specs/vz-net-v1.md).
- API-Fläche bewusst winzig: `acquire` / `release` / `list` / `serialize` / `health`.
- Kein Apply, kein DNS-Zone-Build, kein SQLite-Desired-State, kein REST.
- Unclean Kill von **`vz-net`** orphaned CIDRs weiterhin bis Host-Reboot (Apple-Limit). Schutz = Stabilität + sauberes Shutdown.

### Helper-Lifecycle

- launchd Job pro `vm-id`; Start erst nach Net-Attach.
- Helper holt Serialize-Blobs weiterhin über Control-Plane `helper.networks`; CP proxy’t `net.serialize` an `vz-net`.
- Helper hält UDS rückwärts zum Control-Plane; Disconnect → Retry + State-Report.
- Doppel-Helper: Lockfile + adopt/kill stale.

### Subnet-Lifecycle

- Reservation endet erst mit Release des `vmnet_network_ref` in **`vz-net`** (sauberes Prozessende oder `net.release`).
- Control-Plane führt Desired-State-Ledger (CIDR → runtime_state: active/orphaned nach acquire-Ergebnis).
- Nach unclean **`vz-net`**-Crash: orphaned CIDRs meiden oder Reboot; Alpha: neue CIDR wählen.
- Control-Plane `shutdown` **releast keine** vmnet-Refs (sonst wäre der Split wirkungslos).

### Sleep / Wake (Follow-up)

- Nicht automatisiert in G0 (Host-Sleep unterbricht Agent).
- Erwartung: Guest-Clock driftet; Agent `time-sync` nach Wake.

## Consequences

- Install bootstrapped `com.vzctl.net` **vor** `com.vzctl.supervisor`.
- `doctor` warnt bei fehlendem `net.sock` / unhealthy `vz-net` und bei orphaned CIDRs.
- DNS-Ausfall nach Control-Plane-Crash bleibt akzeptiert; CIDR-Orphan nach CP-Crash nicht mehr.
- Bridged / `com.apple.vm.networking` bleiben out of scope (ADR 0001).

## Alternatives verworfen

- **Monolith Supervisor=Helper:** einfach, aber Kill tötet alle VMs (G0 gemessen) — nur Spike.
- **vmnet pro Helper ohne zentrale Registry:** Cross-VM/Router-Topologie und Dual-DNS schwer; Ownership unklar.
- **Disk-Tombstone / Serialize-Persist:** Serialization ist Live-Share, kein Reclaim nach Prozessende (WWDC26).
- **vmnet-Refs im Control-Plane belassen:** Feature-Crash orphaned CIDRs — Alpha-Risiko, nicht Zielbild.
