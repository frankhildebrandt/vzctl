# ADR 0002: Process- & Ressourcen-Ownership

- **Status:** Accepted (amended 2026-08-01)
- **Date:** 2026-07-30
- **Amended:** 2026-08-01 — `vz-net` owns vmnet refs; `vz-edge` owns host dataplane
- **Issues:** [#7](https://github.com/frankhildebrandt/vzctl/issues/7), [#8](https://github.com/frankhildebrandt/vzctl/issues/8), G0 [#6](https://github.com/frankhildebrandt/vzctl/issues/6)
- **Spike:** [docs/spikes/g0-network.md](../spikes/g0-network.md)
- **Contracts:** [vz-net-v1](../specs/vz-net-v1.md), [vz-edge-v1](../specs/vz-edge-v1.md)

## Context

G0 hat gezeigt:

1. `vmnet_network_ref` ist process-local; Lifetime = Reservation.
2. `kill -9` auf den Prozess, der Net+VM hält → **VM tot**, Bridge weg, **Subnet verbrannt** (`network_create` gleicher Range → `FAILURE 1001`); frische Range OK.
3. `vmnet_stop_interface` allein reicht **nicht**, solange der `vmnet_network_ref` retained bleibt.
4. Sauberes Prozessende / Ref-Release ermöglicht Recreate.
5. DNS auf Host-Bridge-`.0` (UDP) braucht eine lebende Host-Bridge; ein eigener Edge-Prozess hält Listener unabhängig von der Control Plane.

Zielbild: Control Plane + **1 Helper/VM** + minimaler `vz-net` + `vz-edge`, damit VMs, Netze und Host-Dataplane Control-Plane-Crashes überleben.

## Decision

| Ressource | Owner | Bei Control-Plane-Crash | Cleanup |
|---|---|---|---|
| `VZVirtualMachine` | **VM-Helper** (eigener Prozess) | VM läuft weiter | Helper stoppt VM nur auf Befehl / Purge |
| `vmnet_network_ref` + Host-Bridge + Serialize | **`vz-net`** (HyperNetwork Supervisor) | Netze bleiben; CP reconnect via `net.acquire` (idempotent) | `vz-net` releast Refs nur bei `net.release` oder sauberem SIGTERM |
| Desired-State (SQLite networks/attachments) | **Control-Plane** (`vz-supervisor`) | Ledger bleibt; Runtime-Rebuild via `vz-net` | — |
| DNS, Ports, Ingress, Caddy/Dex | **`vz-edge`** | läuft weiter | globaler Reconcile-Snapshot / Last-good Cache |
| Apply-Journal / Lease | **Control-Plane** | incomplete → `apply --resume\|--abort` | — |

### HyperNetwork Supervisor (`vz-net`)

- LaunchAgent `com.vzctl.net`, KeepAlive, Entitlement `com.apple.security.virtualization`.
- UDS `net.sock` unter `$VZCTL_STATE_DIR`; Contract [vz-net-v1](../specs/vz-net-v1.md).
- API-Fläche bewusst winzig: `acquire` / `release` / `list` / `serialize` / `health`.
- Kein Apply, kein DNS-Zone-Build, kein SQLite-Desired-State, kein REST.
- Unclean Kill von **`vz-net`** orphaned CIDRs weiterhin bis Host-Reboot (Apple-Limit). Schutz = Stabilität + sauberes Shutdown.

### Host-Dataplane (`vz-edge`)

- LaunchAgent `com.vzctl.edge`, KeepAlive, ohne Virtualization-Entitlement.
- UDS `edge.sock`; Contract [vz-edge-v1](../specs/vz-edge-v1.md).
- Besitzt DNS- und TCP-Listener sowie Caddy/Dex/oidc-simple-Kindprozesse.
- Hält den letzten erfolgreich angewendeten Runtime-Snapshot über CP-Restarts.
- Kein SQLite-Desired-State, kein Apply-Journal und keine vmnet-Refs.

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

- Install bootstrapped `com.vzctl.net`, dann `com.vzctl.edge`, dann `com.vzctl.supervisor`.
- Ein geplanter `vz-net`-Neustart stoppt zuerst alle VM-Helper graceful. Die
  helperseitig rekonstruierten Refs bleiben nach dem Ende des originalen
  `vz-net`-Owners nicht nutzbar; weiterlaufende VMs wären sonst vom neuen
  Host-Bridge-/vmnet-Handle isoliert. Bei einem hängenden Helper wird der
  `vz-net`-Shutdown ohne SIGKILL abgebrochen.
- `doctor` warnt bei fehlendem `net.sock` / unhealthy `vz-net` und bei orphaned CIDRs.
- DNS, Ports und Ingress überleben einen Control-Plane-Crash.
- Bridged / `com.apple.vm.networking` bleiben out of scope (ADR 0001).

## Alternatives verworfen

- **Monolith Supervisor=Helper:** einfach, aber Kill tötet alle VMs (G0 gemessen) — nur Spike.
- **vmnet pro Helper ohne zentrale Registry:** Cross-VM/Router-Topologie und Dual-DNS schwer; Ownership unklar.
- **Disk-Tombstone / Serialize-Persist:** Serialization ist Live-Share, kein Reclaim nach Prozessende (WWDC26).
- **vmnet-Refs im Control-Plane belassen:** Feature-Crash orphaned CIDRs — Alpha-Risiko, nicht Zielbild.
