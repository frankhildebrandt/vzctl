# GPT 5.6 SOL Review: vzctl Implementationsplan

> Reviewer: GPT 5.6 SOL Medium  
> Datum: 2026-07-30  
> Agent: e128f917-e3a4-48e8-a25b-37a6e043fb42  
> Kontext: Plan **nach** Fable-Must-Fixes

Die Must-Fixes aus dieser Review sind im [Decision Log](04-decision-log.md) und im [Implementation Plan](01-implementation-plan.md) übernommen.

---

# Bewertung

Die Must-Fixes treffen die richtigen Probleme: Helper-pro-VM, vsock-Agent, internes DNS und der v0.2-Schnitt verbessern den Plan deutlich. Der Plan ist aber noch kein belastbarer Implementierungsplan, weil zentrale Ownership-, Netzwerk- und Fehlerszenarien offen sind.

## Scores

| Dimension | Score | Kommentar |
|---|---|---|
| Architektur | 4/5 | Schichtung stimmt, aber Network-Refs, DNS und VM-Lifecycle sind noch keinem Prozess technisch eindeutig zugeordnet |
| Sequencing | 3/5 | Clones vor Stacks ok; Netzwerk-/Plattform-Spike kommt weiterhin zu spät |
| MVP-Realismus | 2/5 | 8–10 Wochen solo eher Alpha als alltagstauglicher v0.1-Core |
| Risiko-Ehrlichkeit | 3/5 | Risiken benannt, aber noch nicht in Gates/Abbruchkriterien übersetzt |
| DX Alltag | 3/5 | up/apply/diff/doctor stark; Recovery, Logs, Mount-Perf, destruktive Änderungen undefiniert |
| Agentic/Scriptability | 4/5 | JSON/Events/Stacks richtig; stabile Schemas, Exitcodes, Idempotenzverträge fehlen |

---

# Kritische offene Löcher

## 1. Prozessmodell entschieden, aber technisch nicht geschlossen

- Supervisor soll vmnet-Refs besitzen, Helper die `VZVirtualMachine` — unklar, wie Netzwerkobjekte an den Helper gelangen bzw. nach Supervisor-Crash weiterleben.
- „launchd/XPC“ ist keine Lifecycle-Entscheidung: Spawn, Adopt, Reconnect, Doppel-Helper, Binary-Upgrades.
- Stirbt der Supervisor und DNS/vmnet weg → VMs laufen, sind aber praktisch unbenutzbar. Crash-Isolation nur teilweise erfüllt.

## 2. DNS-Konzept logisch richtig, operativ unvollständig

- Loopback-DNS für macOS ist aus Guests **nicht** erreichbar → separater Listener auf Host-/Gateway-Adresse nötig.
- Bindings, UDP/TCP 53, Firewall, Privilegien, Konflikte mit bestehenden DNS-Diensten fehlen.
- Autoritativer Server allein reicht nicht als Guest-DNS → Forwarding/Recursion + Upstream-/VPN-Semantik.
- `/etc/resolver`: Port, Install, Cleanup verwaister Dateien, Projektkollisionen.
- `dig` umgeht oft den Systemresolver → `vzctl dns query` muss den vzctl-DNS direkt prüfen.
- `.vz` ist keine reservierte private TLD → robuster `{project}.vz.test`.
- DNS-Ausfall bei Supervisor-Restart, Cache/TTL bei apply undefiniert.

## 3. Netzwerkmodell = größter Blocker

- Woche-1-Spike liegt formal in P2, bestimmt aber schon P0-Architektur.
- DHCP / Reservations / cloud-init static ohne festgelegte Semantik.
- `10.80.0.1` / `10.90.0.1` für Router-VM kann mit vmnet-Gateway kollidieren.
- `routes:` sagt nicht, ob auch Firewall-Policy entsteht („DMZ“ suggeriert Isolation).
- Pre-26 vs. macOS-26 dürfen kein transparenter Fallback sein.

## 4. Reconcile-/Zustandsmodell zu dünn

Lease verhindert paralleles apply, aber nicht inkonsistente Zwischenzustände. Fehlen:

- Idempotenz und Operation-Journal
- Crash-Recovery während apply / Resume / Rollback
- Generation/Revision des Desired State
- Drift-Regeln YAML ↔ SQLite ↔ Helper ↔ Lockfile
- Destruktionsregeln für Disks, Netze, Resolverdateien
- Semantik von `down`, `delete`, `adopt`

## 5. Guest-Agent-Bootstrap zirkulär

- „Install via cloud-init in Base/Seal“ vermischt Image-Build und First-Boot.
- Agent erst nach cloud-init → exec/Health/IP während Bootstrap noch nicht da.
- Versionierung, Upgrade, vsock-Auth, Recovery bei defektem Agent fehlen.

## 6. MVP und Phasen widersprechen sich

- P0–P4 überlappen trotz Abhängigkeiten.
- virtiofs „Muss“ vs. Kickoff nur Spike.
- Events mal P1, mal P7.
- Docker/Ports/virtiofs erhöhen Scope; Nische ist VM-Topologien.
- 8–10 Wo = Walking Skeleton/Alpha, nicht Signing/Recovery/Sleep/VPN/Installer.

---

# Verbesserungen (Review)

### Must

- Netzwerk-/Entitlement-Spike **vor P0**, Go/No-Go-Gate
- ADR Prozess-/Ressourcenbesitz (VZ, vmnet, DNS, Helper-Lifecycle, Reconnect, Upgrade)
- DNS End-to-End: Host- + Guest-Listener, Forwarder, Ports, Privilegien, TTL, VPN, Cleanup
- Reservierte Zone (`.vz.test`) oder `.vz`-Risiko explizit akzeptieren
- Reconcile-Vertrag: Journal, Idempotenz, Resume, Drift, destruktive Änderungen
- v0.1 als **Alpha** deklarieren; virtiofs + Docker-Polish → v0.1.x

### Should

- Router-Rolle: Forwarding + Firewall/Isolation-Policy
- CLI-Vertrag: versioniertes JSON/Event-Schema, Exitcodes, Timeouts, `--dry-run`
- Disk-Lifecycle: Seal-Immutability, Data-Disk-Retention, Clone-GC
- Sleep/Reboot/VPN als Akzeptanztests
- `vzctl logs`, Diagnose-Bundles, Recovery-Hinweise in v0.1
- Ressourcenlimits / Admission-Warnungen

### Nice

- Stack-Snapshots nach stabiler Disk-Semantik
- MCP über stabiler RPC/Event-API
- Tauri nach belastbarer CLI-DX
- k3s / Registry-Cache nach Core-MVP

---

# Top 5 vor dem Scaffolding

1. **macOS-Baseline:** 26-only oder exakt definierter Pre-26-Modus  
2. **Vertikaler Netzwerk-Spike:** zwei Netze, Router, feste IP, Hostzugriff, Guest-DNS, Sleep, Supervisor-Crash  
3. **Process-/Ownership-ADR:** Supervisor, Helper, vmnet, DNS, launchd, Reconnect  
4. **State-/Apply-Spez:** Wahrheit, Journal, Idempotenz, Recovery, Löschregeln  
5. **MVP-Gates:** messbare Exit-Kriterien; virtiofs/Docker-Polish verschieben wenn 8–10 Wo bindend  

---

# Gesamturteil

Der Plan hat eine tragfähige Richtung und vermeidet die größten ursprünglichen Designfehler. Vor dem Scaffolding müssen Netzwerkfähigkeit, DNS-Erreichbarkeit und Prozess-Ownership praktisch bewiesen sowie Apply-/Recovery-Semantik spezifiziert werden; sonst entsteht früh teurer Rework. Als 8–10-Wochen-**Alpha** ist v0.1 plausibel, als zuverlässiges Alltagsprodukt noch nicht.
