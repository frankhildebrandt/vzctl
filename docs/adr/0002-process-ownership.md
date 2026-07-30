# ADR 0002: Process- & Ressourcen-Ownership

- **Status:** Accepted
- **Date:** 2026-07-30
- **Issues:** [#7](https://github.com/frankhildebrandt/vzctl/issues/7), [#8](https://github.com/frankhildebrandt/vzctl/issues/8), G0 [#6](https://github.com/frankhildebrandt/vzctl/issues/6)
- **Spike:** [docs/spikes/g0-network.md](../spikes/g0-network.md)

## Context

G0 hat gezeigt:

1. `vmnet_network_ref` ist process-local; Lifetime = Reservation.
2. `kill -9` auf den Prozess, der Net+VM hält → **VM tot**, Bridge weg, **Subnet verbrannt** (`network_create` gleicher Range → `FAILURE 1001`); frische Range OK.
3. `vmnet_stop_interface` allein reicht **nicht**, solange der `vmnet_network_ref` retained bleibt.
4. Sauberes Prozessende / Ref-Release ermöglicht Recreate.
5. DNS auf Host-Bridge-`.0` (UDP) muss im Supervisor leben — stirbt mit Supervisor.

Zielbild (Plan): Supervisor + **1 Helper/VM**, damit VMs Supervisor-Crash überleben.

## Decision

| Ressource | Owner | Bei Supervisor-Crash | Cleanup |
|---|---|---|---|
| `VZVirtualMachine` | **VM-Helper** (eigener Prozess) | VM läuft weiter | Helper stoppt VM nur auf Befehl / Purge |
| `vmnet_network_ref` + Serialisierung | **Supervisor** Registry | Net orphaned → Helper meldet `net_orphaned` | Supervisor muss Refs **release** + `stop_interface`; sonst Subnet-Leak |
| DNS Zone + UDP-Listener auf `.0` (+ Host `127.0.0.1`) | **Supervisor** | DNS down bis Restart (Alpha akzeptiert) | Listener schließen |
| Apply-Journal / Lease | **Supervisor** | incomplete → `apply --resume\|--abort` | — |

### Helper-Lifecycle

- launchd Job pro `vm-id`; Start erst nach Net-Attach (Supervisor übergibt serialisierten vmnet-Handle / Attachment-ID).
- Helper hält UDS rückwärts zum Supervisor; Disconnect → Retry + State-Report.
- Doppel-Helper: Lockfile + adopt/kill stale.
- **Monolith (nur Spike):** Kill = VM+Net tot — **nicht** Produktionsmodell.

### Subnet-Lifecycle (G0-Messung)

- Reservation endet erst mit Release des `vmnet_network_ref` (Prozessende oder CFRelease nach Stop).
- Supervisor führt Subnet-Ledger (CIDR → state: reserved/active/orphaned).
- Nach Crash: orphaned CIDRs meiden oder nach Timeout/Reboot recyclen; Alpha: **neue CIDR wählen** + Ledger markieren.

### Sleep / Wake (Follow-up)

- Nicht automatisiert in G0 (Host-Sleep unterbricht Agent).
- Erwartung: Guest-Clock driftet; Agent `time-sync` nach Wake.
- Manuelle Prozedur in Spike-Notes; Alpha: dokumentiertes Risiko bis gemessen.

## Consequences

- P0 muss Helper-Binary + launchd + Net-Serialize (WWDC26 XPC) vor Multi-VM-Alltag haben.
- `down` / Crash-Recovery: immer Net-Refs droppen, sonst CIDR-Exhaustion.
- DNS-Ausfall nach Supervisor-Crash ist akzeptiert und zu dokumentieren (`doctor` warnt).
- Bridged / `com.apple.vm.networking` bleiben out of scope (ADR 0001).

## Alternatives verworfen

- **Monolith Supervisor=Helper:** einfach, aber Kill tötet alle VMs (G0 gemessen) — nur Spike.
- **vmnet pro Helper ohne Supervisor-Registry:** Cross-VM/Router-Topologie und Dual-DNS schwer; Ownership unklar.
