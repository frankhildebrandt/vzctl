#!/usr/bin/env python3
"""Enrich GitHub issue bodies to implementation-ready quality (aligned with Fable/SOL)."""

from __future__ import annotations

import subprocess
import tempfile
import textwrap
import time
from pathlib import Path

REPO = "frankhildebrandt/vzctl"
DOC = f"https://github.com/{REPO}/blob/main/docs/planing"


def edit(num: int, body: str) -> None:
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
        f.write(body.strip() + "\n")
        path = f.name
    r = subprocess.run(
        ["gh", "issue", "edit", str(num), "--repo", REPO, "--body-file", path],
        capture_output=True,
        text=True,
    )
    Path(path).unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"#{num}: {r.stderr}")
    print(f"updated #{num}")
    time.sleep(0.25)


def src(*files: str) -> str:
    lines = ["## Quellen", ""]
    for f in files:
        lines.append(f"- [{f}]({DOC}/{f})")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Bodies
# ---------------------------------------------------------------------------

B: dict[int, str] = {}

B[1] = f"""\
## Summary

Go/No-Go-Spike **vor** jedem Code-Scaffolding. Beweist vmnet-Topologie, feste IPs, Dual-DNS-Erreichbarkeit, Sleep- und Supervisor-Crash-Verhalten.

## Kontext / Findings

| Quelle | Finding |
|---|---|
| GPT-SOL | Netzwerk-/Entitlement-Spike muss **vor P0**; sonst teurer Rework |
| Fable | Netzwerkisolation ist der schwierigste Teil von P2 — früh beweisen |
| Decision | G0 Gate; Abbruch wenn Isolation unmöglich |

## Scope

- Zwei `shared` vmnet-Netze + Router-VM
- Reproduzierbare Guest-IPs (cloud-init static)
- DNS-Probe: welche Host-IP ist aus Guests erreichbar?
- Sleep/Wake Clock-Drift messen
- Supervisor-Kill: VM/Helper/DNS/vmnet-Verhalten dokumentieren
- ADR-Entwurf macOS-Baseline

## Non-Goals

- Produktions-CLI / Reconciler
- Bridged Networking
- Ingress / OIDC

## Exit-Kriterien

- [ ] Ping/HTTP Cross-Net via Router
- [ ] Host ↔ Guest Erreichbarkeit dokumentiert
- [ ] Guest-DNS-Bind-Adresse spezifiziert (nicht nur `127.0.0.1`)
- [ ] Sleep/Crash-Protokoll in Spike-Notes
- [ ] Go **oder** No-Go mit Begründung
- [ ] Sub-Issues #2–#6 erledigt oder explizit deferred mit Risiko

## Deliverables

- `docs/spikes/g0-network.md` (Protokoll + Messwerte)
- Input für ADR #2 (Baseline) und Ownership-ADR

{src('01-implementation-plan.md','05-gpt-sol-review.md','04-decision-log.md')}
"""

B[2] = f"""\
## Summary

Architecture Decision Record: Mindest-macOS für v0.1 und Bridged-Scope.

## Kontext / Findings

| Quelle | Finding |
|---|---|
| SOL #14 | Empfehlung **macOS 26-only** für v0.1 |
| SOL #15 | Bridged braucht `com.apple.vm.networking` → **out of scope** |
| Fable | Pre-26 Fallback darf kein stiller Modus sein |

## Entscheidung (zu treffen)

1. **Baseline:** macOS 26-only *(Empfehlung)* **oder** exakt spezifizierter Pre-26-Pfad
2. **Bridged:** v0.1 unsupported
3. Kein transparenter Fallback zwischen Modi

## Deliverable

- [ ] Datei `docs/adr/0001-macos-baseline.md` mit:
  - Context / Decision / Consequences
  - Entitlement-Liste
  - Testmatrix Host-OS
  - Was bewusst nicht supported wird

## Acceptance

- [ ] ADR gemerged
- [ ] Issue #1 referenziert die Entscheidung
- [ ] Schema/`doctor` können später Baseline enforcen

{src('04-decision-log.md','05-gpt-sol-review.md')}
"""

B[3] = f"""\
## Summary

Vertikaler Spike: zwei vmnet-`shared`-Netze und Cross-Net-Traffic über eine Router-VM.

## Kontext / Findings

| Quelle | Finding |
|---|---|
| Fable | Custom Topologien / Isolation unbewiesen vor macOS 26 |
| SOL | Größter Blocker; bestimmt P0-Architektur |
| WWDC26 / Plan | `VZVmnetNetworkDeviceAttachment`; Networks nicht persistent |

## Voraussetzungen

- Host idealerweise macOS 26+
- Virtualization + vmnet Entitlements klar

## Umsetzungsschritte

1. Minimales Swift/ObjC Harness (kein voller Supervisor nötig)
2. Netz A `10.80.0.0/24`, Netz B `10.90.0.0/24`
3. VM `router` mit 2 NICs; IP-Forwarding + einfache Forward-Policy
4. Je 1 Client-VM pro Netz
5. Tests: L2/L3 Reachability, TCP :80 optional

## Messpunkte dokumentieren

- [ ] Benötigte Entitlements / Root?
- [ ] DHCP vs. static Verhalten
- [ ] Persistenz: was passiert nach Prozess-Exit?
- [ ] Latenz / Stabilität grob

## Acceptance

- [ ] Cross-Net Ping erfolgreich
- [ ] Spike-Notes mit Repro-Schritten
- [ ] Blocker explizit gelistet

{src('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md')}
"""

B[4] = f"""\
## Summary

IP-Vergabe-Semantik und Gateway-/Router-Adress-Konvention festlegen — **keine** konkurrierenden DHCP+static-Welten.

## Kontext / Findings

| Quelle | Finding |
|---|---|
| SOL #16 | cloud-init **static Primär**; kein wildes Mix |
| SOL #17 | Router auf `.1` kann mit vmnet-Gateway kollidieren |
| Decision Log | Konvention offen → dieses Spike schließt sie |

## Zu beantworten

1. Wer ist Gateway in shared-vmnet (Host/vmnet-DHCP)?
2. Welche IP bekommt die Router-VM? (Empfehlung aus Plan: Gateway `.1`, Router `.2`)
3. Schema-Regel: `ip:` setzt static; `dhcp: true` nur ohne `ip` **oder** Reservation aligned

## Deliverable

- [ ] Abschnitt in `docs/spikes/g0-network.md` + Vorschlag für Schema-Validierung
- [ ] Beispiel-YAML-Snippet (edge-dmz) mit finalen IPs

## Acceptance

- [ ] Konvention schriftlich + im Decision Log nachziehbar
- [ ] Kollisionsfall Gateway/Router reproduziert oder widerlegt

{src('04-decision-log.md','05-gpt-sol-review.md','01-implementation-plan.md')}
"""

B[5] = f"""\
## Summary

Herausfinden, **welche IP/Interface** aus einer Guest-VM den Hypervisor-DNS erreichen kann (Dual-Listener-Grundlage).

## Kontext / Findings

| Quelle | Finding |
|---|---|
| SOL | `127.0.0.1` DNS ist aus Guests **unerreichbar** |
| Plan | Host-Listener Loopback + Guest-Listener auf Gateway/Hypervisor-IP |
| Fable | Interne Domain muss Guest-seitig resolven (nicht `*.localhost`) |

## Experiments

1. DNS-Stub auf verschiedenen Host-Adressen binden (Gateway-IP, bridge, vmnet iface)
2. Aus Guest: `dig @<ip> test.{{project}}.vz.test`
3. Privilegien: Port 53 vs. High-Port `15353`
4. Konflikte mit mDNS / Corporate DNS / VPN notieren

## Deliverable

- [ ] Gewählte `guestListen`-Adresse + Begründung
- [ ] Firewall-/Privilege-Hinweise für Implementierung #26

## Acceptance

- [ ] Mindestens ein stabiler Pfad Host→Guest DNS Query
- [ ] Explizites „geht nicht“ für Loopback-only dokumentiert

{src('05-gpt-sol-review.md','01-implementation-plan.md')}
"""

B[6] = f"""\
## Summary

Host-Sleep und Supervisor-Crash als Alpha-Akzeptanzrisiken messen und dokumentieren.

## Kontext / Findings

| Quelle | Finding |
|---|---|
| Fable | Sleep → Clock-Drift bricht TLS/OIDC später |
| SOL | Supervisor-Crash: VMs laufen, DNS/vmnet können tot sein |
| Plan | Alpha akzeptiert DNS-down bis Restart — muss klar kommuniziert sein |

## Tests

### Sleep/Wake

- [ ] Guest-Uhr vor/nach Sleep (Delta)
- [ ] Offene TCP-Verbindungen
- [ ] Bedarf Time-Sync via Agent (#16)

### Supervisor `kill -9`

- [ ] Läuft Helper/VM weiter?
- [ ] Sind vmnet-Attachments noch gültig?
- [ ] DNS erreichbar?
- [ ] Reconnect-Anforderungen für #11

## Deliverable

Tabelle „Ereignis → beobachtetes Verhalten → Alpha-Policy“ in Spike-Notes.

## Acceptance

- [ ] Policies für Alpha schriftlich (was gilt als ok / nicht ok)
- [ ] Tickets #11/#16 referenzieren die Messwerte

{src('02-fable-review.md','05-gpt-sol-review.md')}
"""

B[7] = f"""\
## Summary

Epic für das Prozessmodell: **Supervisor + 1 Helper-Prozess pro VM** (Fable) inkl. Ownership/Reconnect (SOL).

## Architekturziel

```text
CLI/UI → Reconciler → Supervisor
                         ├─ vmnet Registry + DNS + Journal
                         └─ launchd → VM-Helper (1:1) ⇄ vsock Agent
```

## Sub-Issues

Siehe Child-Issues: ADR Ownership, UDS/SQLite, Helper Lifecycle, Reconnect.

## Non-Goals

- Monolith-Daemon der alle `VZVirtualMachine` hält
- UI

## Definition of Done

- [ ] ADR gemerged
- [ ] Zwei parallele VMs: Crash Helper-A lässt VM-B unberührt
- [ ] Supervisor-Restart: Helper reconnecten (Alpha-Pfad)
- [ ] Docs in `docs/adr/0002-process-ownership.md`

{src('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md','04-decision-log.md')}
"""

B[8] = f"""\
## Summary

ADR: Welche Komponente besitzt welche Ressource — und was passiert nach Crashes.

## Ownership-Matrix (Soll)

| Ressource | Owner | Nach Supervisor-Crash |
|---|---|---|
| `VZVirtualMachine` | **VM-Helper** | läuft weiter |
| vmnet network refs | **Supervisor**; Helper bekommt Attachment-Handle | ggf. `net_orphaned` |
| DNS Zone + Listener | **Supervisor** | DNS down bis Restart (Alpha) |
| Stack-Lease / Journal | **Supervisor** | `apply --resume` |

## Helper-Lifecycle (muss spezifiziert sein)

1. Supervisor registriert launchd-Job pro `vm-id`
2. Start-Reihenfolge: Net create/attach → spawn Helper mit Config + handle
3. Helper öffnet Rückkanal (UDS) → State-Reports
4. Disconnect → Retry; bei Net-Tod Event `net_orphaned`
5. Doppel-Helper: Lockfile + adopt/kill stale
6. Alpha-Upgrade: nur gestoppte VMs rolling replace

## Offene Punkte aus Decision Log (schließen)

- [ ] launchd-Plist vs. XPC im Detail
- [ ] Exact handle-passing API für vmnet attachments

## Deliverable

- [ ] `docs/adr/0002-process-ownership.md`

## Acceptance

- [ ] Keine Ambiguität mehr „wer hält VZ/vmnet“
- [ ] Sequenzdiagramm Start/Stop/Crash im ADR

{src('01-implementation-plan.md','05-gpt-sol-review.md','04-decision-log.md')}
"""

B[9] = f"""\
## Summary

Schlanker Supervisor-Kern: Unix-Domain-Socket RPC, Health, SQLite-Stub.

## Socket

- Pfad: `~/Library/Application Support/vzctl/vz.sock` (User-owned)
- Peer-Cred prüfen (gleiche UID)
- Framing: JSON-RPC 2.0 + NDJSON Events (später)

## Minimale RPCs (v0)

| Method | Zweck |
|---|---|
| `daemon.health` | liveness + version |
| `daemon.version` | semver/git |
| (stub) `vm.list` | empty ok |

## SQLite Stub

Tabellen grob:

- `resources(id, kind, name, labels_json, state, …)`
- `journal(id, stack_id, gen, step, status, payload, …)`
- `locks(stack_id, holder, expires)`

Pfad: Application Support / konfigurierbar.

## Acceptance

- [ ] `vzctl doctor` / raw client erreicht `daemon.health`
- [ ] Restart erhält DB
- [ ] Unit-Test ohne Hypervisor für RPC framing

## Non-Goals

- volle vm/net Implementierung (kommt mit Helper/Net-Epics)

{src('01-implementation-plan.md')}
"""

B[10] = f"""\
## Summary

Ein Helper-Prozess hält **genau eine** `VZVirtualMachine` (Ubuntu NAT-Boot zuerst).

## Finding

Fable: Monolith-Daemon ⇒ Crash/Update killt alle VMs. Deshalb 1:1 Helper (wie Tart/Lima-Idee).

## Responsibilities

- Boot/Stop/Kill VM
- Serial console stream an Supervisor weiterreichen
- vsock zum Guest-Agent
- Heartbeat/State an Supervisor

## Start-Contract (vom Supervisor)

```json
{{
  "vm_id": "edge-dmz/web",
  "bundle_path": ".../web/",
  "net_attachments": [{{"network_id":"dmz","handle":"…"}}],
  "supervisor_uds": "…/vz.sock"
}}
```

## Acceptance

- [ ] Ubuntu Cloud Image headless start/stop
- [ ] Crash-Test: Helper-A kill → Helper-B unberührt
- [ ] launchd plist pro vm-id
- [ ] Logs unter `~/Library/Logs/vzctl/`

## Tech notes

- Swift Package bevorzugt für VZ Bindings
- Entitlements: Virtualization (+ später vmnet am Supervisor)

{src('02-fable-review.md','01-implementation-plan.md')}
"""

B[11] = f"""\
## Summary

Nach Supervisor-Restart Helper wieder anbinden und Net-Zustand reparieren/melden.

## Finding (SOL)

Crash-Isolation ist nur teilweise: VMs können laufen, während DNS/vmnet tot sind.

## Verhalten

1. Helper erkennt UDS-Disconnect → exponential backoff reconnect
2. Re-register bei Supervisor (`helper.hello` mit vm_id, pid, state)
3. Wenn Attachments invalid: State `net_orphaned`
4. Supervisor Alpha-Pfad: Networks rebuild → re-issue handles → Helper reconfigure **oder** dokumentierter Reboot der VM

## Acceptance

- [ ] Automatischer Reconnect ohne Guest-Neustart (wenn Net noch ok)
- [ ] Klarer Event `vm.net_orphaned`
- [ ] Spike-Messwerte aus #6 referenziert
- [ ] Kein Doppel-Helper nach Restart

{src('05-gpt-sol-review.md','01-implementation-plan.md')}
"""

B[12] = f"""\
## Summary

vsock Guest-Agent als Control-Plane (exec, IP, health, time-sync, CA-inject, logs). **In sealed Base vorinstalliert** (SOL #21).

## Warum nicht nur SSH?

Fable: IP-Discovery, CA-Rollout, exec und Bootstrap hängen sonst an DHCP-Raterei und Henne-Ei.

## Capabilities (Roadmap)

| Capability | Alpha | Später |
|---|---|---|
| ping / version | ✓ | |
| exec | ✓ | |
| report-ip / health | ✓ | |
| time-sync | ✓ | |
| log-tail | ✓ | |
| ca-inject | | v0.2 |

## Non-Goals Alpha

- Volles Package-Management
- Windows Guests

## DoD

- [ ] Agent in Base
- [ ] E2E exec ohne SSH
- [ ] Time-sync nach Sleep angebunden

{src('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md')}
"""

B[13] = f"""\
## Summary

Protokoll- und Auth-Spec für den vsock Guest-Agent.

## Transport

- `VZVirtioSocketDevice` / Virtio socket
- Length-prefixed JSON messages (Alpha) — protobuf optional später

## Auth

- Shared secret / token pro VM aus NoCloud (`meta-data` oder write_files)
- Token nur über vsock, nie ins Git
- Version handshake vor Commands

## Message-Skizze

```json
{{"v":1,"id":"…","method":"exec","params":{{"cmd":["uname","-a"],"timeout_ms":5000}}}}
{{"v":1,"id":"…","ok":true,"result":{{"exit":0,"stdout":"…","stderr":""}}}}
```

## Methods (Alpha)

`ping`, `version`, `exec`, `report_ip`, `health`, `time_hint`

## Acceptance

- [ ] Spec unter `docs/specs/guest-agent-v1.md`
- [ ] Fehlercodes stabil
- [ ] Timeout/Cancellation definiert

{src('05-gpt-sol-review.md')}
"""

B[14] = f"""\
## Summary

Guest-Agent-Binary + systemd Unit in Ubuntu Base **vor** `image seal` einbetten.

## Finding

SOL: First-Boot cloud-init Install erzeugt Bootstrap-Zirkel (Control-Plane braucht Agent, Agent kommt erst nach cloud-init).

## Pipeline

1. Start von Ubuntu Cloud Image
2. Install `vzctl-agent` after + enable systemd
3. cloud-init clean / identity wipe (Teil von seal)
4. `vzctl image seal ubuntu-base` → immutable

## cloud-init danach nur

- Hostname, SSH keys, token path, network-config
- **Kein** Agent-Download

## Acceptance

- [ ] Frischer Clone hat Agent listening auf vsock ohne Extra-Install
- [ ] Seal-Doku aktualisiert
- [ ] Version des Agents im Image metadatiert

{src('04-decision-log.md','01-implementation-plan.md')}
"""

B[15] = f"""\
## Summary

End-to-End: Helper spricht Agent — `ping`, `exec`, `report-ip`.

## CLI

```bash
vzctl vm exec <name> -- uname -a
vzctl vm info <name> --format json   # enthält agent.ips
```

## Acceptance

- [ ] exec Exitcode/stdout/stderr korrekt
- [ ] report-ip liefert IPs passend zu Attachments
- [ ] Timeout greift
- [ ] Wenn Agent down: klarer Fehler + Hinweis Serial/SSH Fallback
- [ ] Kein SSH im Happy Path

## Testplan

1. Boot linked clone aus sealed Base
2. Warte Agent-ready Event
3. exec + report-ip
4. Agent stoppen → Fehlerpfad

{src('01-implementation-plan.md')}
"""

B[16] = f"""\
## Summary

Nach Host-Sleep Guest-Uhr korrigieren (sonst TLS/OIDC später kaputt).

## Finding

Fable/SOL: Clock-Drift nach Sleep ist klassisches VZ-Problem.

## Ansatz

- Supervisor/Helper sendet `time_hint` (UTC epoch ns) nach Wake
- Agent stellt Uhr (chrony step / `timedatectl` / `date -s`) gemäß Policy
- Nur wenn Drift > Schwellwert (z.B. 1s)

## Acceptance

- [ ] Akzeptanztest Sleep 5–10 min → Drift korrigiert
- [ ] Event `vm.clock_corrected`
- [ ] Messwerte aus #6 als Baseline

{src('02-fable-review.md','05-gpt-sol-review.md')}
"""

B[17] = f"""\
## Summary

Agent-/Script-first CLI: JSON, Exitcodes, Events, doctor.

## CLI-Oberfläche (Zielbild aus Canvas)

```text
vzctl version
vzctl daemon status|start|stop
vzctl vm create|start|stop|restart|delete|list|info|exec|shell|console|logs
vzctl image pull|seal|list
vzctl net create|attach|detach|list|delete
vzctl route add|apply|list
vzctl policy apply|list
vzctl dns status|query|reload|install-resolver|uninstall-resolver
vzctl up|down|apply|diff|ps|validate|adopt
vzctl apply --resume|--abort
vzctl events subscribe
vzctl doctor
vzctl docker …          # später
```

## DoD Epic

- [ ] JSON überall relevant
- [ ] Event-Schema v1
- [ ] doctor früh nutzbar

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[18] = f"""\
## Summary

Maschinenlesbare CLI-Ausgabe und stabile Exitcodes.

## Exitcodes (verbindlich)

| Code | Bedeutung |
|---|---|
| 0 | ok |
| 2 | not found |
| 3 | invalid input / validation |
| 4 | daemon down / unreachable |
| 5 | guest / agent error |
| 6 | conflict / locked (apply lease) |
| 10+ | reserved |

## JSON

- `--format json` (Default für Agent-Mode später)
- stabile Feldnamen versionieren (`apiVersion` im Envelope optional)

## Acceptance

- [ ] Spec `docs/specs/cli-contract-v1.md`
- [ ] `vm list|info` JSON golden tests
- [ ] stderr nur Diagnostics, stdout Daten (Dokumentieren)

{src('01-implementation-plan.md')}
"""

B[19] = f"""\
## Summary

NDJSON Event-Stream für Agents — **früh** (Fable Gap; nicht auf P7 schieben).

## Event-Typen (v1)

- `vm.state` / `vm.net_orphaned` / `vm.agent_ready` / `vm.clock_corrected`
- `net.changed`
- `apply.started` / `apply.step` / `apply.finished` / `apply.failed`
- `dns.reloaded`

## Subscribe

```bash
vzctl events subscribe --filter 'vm.*,apply.*'
```

Envelope:

```json
{{"v":1,"ts":"…","type":"vm.state","data":{{…}}}}
```

## Acceptance

- [ ] Schema-Datei + Compatibility-Regel (additive fields)
- [ ] Stream endet sauber bei Ctrl-C
- [ ] Mindestens vm.state + apply.* implementiert

{src('02-fable-review.md','05-gpt-sol-review.md')}
"""

B[20] = f"""\
## Summary

`vzctl doctor` diagnostiziert Setup-Blocker **früh** (Fable: nicht erst P7).

## Checks (Alpha)

- [ ] Supervisor-Socket erreichbar + health
- [ ] macOS Version vs. Baseline-ADR
- [ ] Virtualization entitlement / API availability
- [ ] APFS volume (clonefile)
- [ ] DNS listener ports frei / resolver files
- [ ] Disk space für images
- [ ] (später) vmnet capability

## Output

- human + `--format json`
- Exit ≠0 wenn hard-fail

## Acceptance

- [ ] Auf frischem Mac hilfreiche Meldungen
- [ ] Docs: Interpretationshilfe

{src('02-fable-review.md')}
"""

B[21] = f"""\
## Summary

Shared Base Images, APFS Linked Clones, per-VM `dataDisk`, Identity-Reset.

## Plattenmodell (Canvas)

1. **Base** — sealed, immutable, Agent vorinstalliert  
2. **Root Linked Clone** — `clonefile(2)` COW  
3. **dataDisk** — neues leeres Image („alles weitere“)

## CLI

```bash
vzctl image pull ubuntu:24.04 --name ubuntu-base
vzctl image seal ubuntu-base
vzctl vm create web --from ubuntu-base --data-disk 40G
```

## YAML

```yaml
images:
  ubuntu-base:
    from: ubuntu:24.04
    role: base
vms:
  web:
    from: ubuntu-base
    clone: linked   # default
    dataDisk: 40G
```

## DoD

- [ ] Seal + Clone + Identity E2E
- [ ] Purge löscht Clone+dataDisk, Base bleibt

{src('01-implementation-plan.md')}
"""

B[22] = f"""\
## Summary

Base clone-safe machen und als immutable markieren.

## Seal-Pipeline (aus Canvas)

1. Optional customize
2. `cloud-init clean --logs`
3. truncate `/etc/machine-id`, dbus machine-id
4. SSH host keys entfernen
5. **Agent bleibt installiert**
6. Datei read-only + Label `sealed=true`
7. Snapshot-Ref für clonefile

## Niemals

- Bereits „persönliche“ laufende VM als Base missbrauchen

## Acceptance

- [ ] `vzctl image seal` idempotent
- [ ] doctor/image info zeigt sealed
- [ ] Docs Seal-Checklist

{src('01-implementation-plan.md')}
"""

B[23] = f"""\
## Summary

Pro VM: APFS-`clonefile` der Base + neues dataDisk; an Helper/VZ anhängen.

## Details

- Root-Disk: COW Clone unter VM-Datenpfad
- dataDisk: Sparse/ASIF leer; cloud-init formatiert z.B. `/data` oder `/var/lib/docker`
- Fallback `clone: full` wenn kein APFS

## Disk-Lifecycle

| Aktion | Base | Clone | dataDisk |
|---|---|---|---|
| create | keep | create | create |
| down | keep | keep | keep |
| purge | keep | delete | delete |

## Acceptance

- [ ] Zwei VMs teilen physische Blöcke bis Writes divergieren (verifizieren via `du`/space)
- [ ] Base-Datei nie writable öffnen
- [ ] Fehlerpfad bei clonefile-fail

{src('01-implementation-plan.md')}
"""

B[24] = f"""\
## Summary

Automatischer Identity-Reset bei jedem Clone — **nie** aus Base übernehmen.

## Matrix (Canvas)

| Feld | Quelle | Verhalten |
|---|---|---|
| MAC pro NIC | Daemon/Helper | Neue local-admin MAC (`02:…`) |
| NICs | VZ + cloud-init | Frische Devices; network-config mit IP/MAC |
| machine-id | First-boot | `/etc/machine-id` + dbus leeren → regenerate |
| Hostname | YAML vm key | cloud-init `hostname`/`fqdn` |
| SSH Host Keys | cloud-init | `ssh_deletekeys` + `ssh_genkeytypes` |
| cloud-init instance-id | Reconciler | Neue UUID → NoCloud läuft |
| Persistent net rules | Seal | Keine `70-persistent-net`; predictable names folgen MAC |

## Acceptance

- [ ] Zwei Clones haben unterschiedliche machine-id, MAC, SSH keys
- [ ] Tests automatisiert wo möglich
- [ ] Keine Identical-Host-Key Warnings zwischen VMs

{src('01-implementation-plan.md','02-fable-review.md')}
"""

B[25] = f"""\
## Summary

Hypervisor-DNS für `*.{{project}}.vz.test` + macOS `/etc/resolver` + Guest-Konfiguration.

## Kritische Korrekturen (Fable/SOL)

- ❌ Kanonischer Issuer/`auth.localhost` in Guests (Loopback!)
- ✅ Domain `{{vm}}.{{net}}.{{project}}.vz.test`
- ✅ Dual Listener (Host Loopback + Guest-IP)
- ✅ Forward für externe Namen
- ✅ `dns query` spricht vzctl-DNS **direkt**

## Namensschema

| Kontext | Beispiel |
|---|---|
| VM | `web.dmz.edge-dmz.vz.test` |
| Service | `auth.svc.edge-dmz.vz.test` |
| Host-Alias v0.2 | `web.localhost` → gleicher Upstream |

## DoD

- [ ] Resolve von Host und Guest
- [ ] install-resolver + purge cleanup
- [ ] Forward + VPN-Hinweis dokumentiert

{src('01-implementation-plan.md','05-gpt-sol-review.md','04-decision-log.md')}
"""

B[26] = f"""\
## Summary

Autoritativer DNS im Supervisor + Recursion/Forward.

## Bindings

| Listener | Bind | Client |
|---|---|---|
| Host | `127.0.0.1:15353` | `/etc/resolver` |
| Guest | aus Spike #5 | Guests |

## Zone

- Autoritative Records aus Actual State (VM attachments, services)
- TTL 5–30s
- Forward `upstream: system` (konfigurierbar)
- VPN-Verhalten: dokumentieren (split DNS Grenzen)

## Tech

- Eingebettet (z.B. hickory-dns) oder sorgfältig gewrappt
- Reload bei apply ohne Full-Restart wenn möglich

## Acceptance

- [ ] A-Record für VMs korrekt
- [ ] Externe Namen via Forward (google.com o.ä. in Lab)
- [ ] Supervisor-Restart: DNS down erwartet (Alpha) — Event/health zeigt es
- [ ] Unit tests für Zone builder

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[27] = f"""\
## Summary

macOS Resolver-Dateien für `*.{{project}}.vz.test`.

## Implementierung

```bash
# /etc/resolver/edge-dmz.vz.test
nameserver 127.0.0.1
port 15353
```

## CLI

```bash
vzctl dns install-resolver    # sudo
vzctl dns uninstall-resolver
# purge stack entfernt verwaiste resolver files des Projekts
```

## Acceptance

- [ ] `curl http://web.dmz.edge-dmz.vz.test` nutzt Systemresolver (Browser/libc)
- [ ] Cleanup idempotent
- [ ] Kollision zweier Projekte dokumentiert/behandelt
- [ ] Hinweis: `dig` ohne `@server` kann Resolver umgehen → nutze `vzctl dns query`

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[28] = f"""\
## Summary

CLI-Query **direkt** gegen den vzctl-DNS (SOL: dig/getaddrinfo unzuverlässig als Test).

## UX

```bash
vzctl dns query web.dmz.edge-dmz.vz.test
vzctl dns query --type A --server 127.0.0.1:15353 …
vzctl dns status --format json
```

## Acceptance

- [ ] JSON output mit answers/rcode
- [ ] Exitcodes laut CLI-Contract
- [ ] Funktioniert auch ohne `/etc/resolver`

{src('05-gpt-sol-review.md')}
"""

B[29] = f"""\
## Summary

Guests bekommen Hypervisor-DNS und Search-Domain via cloud-init network-config.

## cloud-init Skizze

```yaml
network:
  version: 2
  ethernets:
    enp0s1:
      dhcp4: false
      addresses: [10.80.0.10/24]
      gateway4: 10.80.0.1   # Konvention aus #4
      nameservers:
        addresses: [<guestListenIP>]
        search: [dmz.edge-dmz.vz.test, edge-dmz.vz.test]
```

## Acceptance

- [ ] Aus Guest: Resolve `web.dmz.edge-dmz.vz.test` und kurzer Name `web` (search)
- [ ] Externe Namen via Forward
- [ ] Reconcile schreibt network-config aus Attachments

{src('01-implementation-plan.md')}
"""

B[30] = f"""\
## Summary

vmnet Networks, Attachments, Router-Routing und **Firewall-Policies** (DMZ-Semantik).

## YAML-Kern

```yaml
networks:
  dmz: {{ cidr: 10.80.0.0/24, mode: shared }}
  lan: {{ cidr: 10.90.0.0/24, mode: shared }}
routes:
  - {{ name: dmz-to-lan, from: dmz, to: lan, via: router }}
policies:
  - name: dmz-default
    network: dmz
    forward: deny-all
    allow:
      - {{ to: lan, proto: tcp, ports: [5432] }}
```

## DoD

- [ ] CRUD + persist Desired State (vmnet nicht persistent!)
- [ ] Router apply
- [ ] Policies apply
- [ ] Labels `managed-by=vzctl`

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[31] = f"""\
## Summary

Network CRUD im Supervisor; Desired State in SQLite; Rebuild nach Restart.

## Finding

Fable/WWDC: vmnet Networks sind **nicht** persistent — App muss Settings speichern und rekonstruieren.

## RPCs / CLI

```bash
vzctl net create dmz --cidr 10.80.0.0/24 --mode shared
vzctl net attach web --network dmz --ip 10.80.0.10
vzctl net list --format json
vzctl net delete dmz   # failt wenn VMs attached
```

## Regeln

- Live-Reconfig fragil → MVP: NIC/Net-Änderungen brauchen VM-Stop (Plan)
- Bridged mode: v0.1 unsupported

## Acceptance

- [ ] create/attach/list/detach/delete
- [ ] Supervisor-Restart stellt Netze aus DB wieder her
- [ ] Labels + project/stack metadata

{src('01-implementation-plan.md','02-fable-review.md')}
"""

B[32] = f"""\
## Summary

Router-VM-Rolle + deklaratives `route apply` (nicht Host-`pf` als Default).

## Finding

Plan/Fable: Cross-Net über Router-VM ist robuster gegen Sleep/VPN als Host-Routing.

## roles: [router]

cloud-init:

- `net.ipv4.ip_forward=1`
- sysctl persist
- Basis-nftables/iptables aus policies (#33)
- Optional FRR später (Nice)

## CLI

```bash
vzctl route apply
```

Pusht Config via Guest-Agent (nicht SSH Happy Path).

## Acceptance

- [ ] Cross-Net Traffic nur wenn Route+Policy es erlauben
- [ ] Gateway-IPs folgen Konvention aus #4
- [ ] Idempotent

{src('01-implementation-plan.md')}
"""

B[33] = f"""\
## Summary

`policies:` erzeugen echte Forward-Filter — „DMZ“ ist sonst Etikettenschwindel (SOL).

## Schema

```yaml
policies:
  - name: dmz-default
    network: dmz
    forward: deny-all
    allow:
      - {{ to: lan, proto: tcp, ports: [5432] }}
      - {{ to: dmz, proto: icmp }}
```

## Implementierung

- Render nftables auf Router-VM
- apply via Agent
- Diff/plan zeigt Policy-Changes

## Acceptance

- [ ] Default deny blockt Cross-Net außer allow
- [ ] JSON status der aktiven Rules
- [ ] Beispiel in edge-dmz

{src('05-gpt-sol-review.md','04-decision-log.md')}
"""

B[34] = f"""\
## Summary

Git-native Environments: `hypernetwork.config.yaml` → `up` / `down` / `apply` / `diff`.

## Compose-Semantik

| Command | Bedeutung |
|---|---|
| `up` | create missing + start; keine destruktiven Deletes |
| `apply` | Drift korrigieren; Breaking Changes Prompt/`--force` |
| `down` | stop (reverse dependsOn) |
| `down --purge` | delete nur managed Ressourcen + resolver files |
| `diff` | Plan anzeigen |
| `adopt` | Orphans übernehmen |

## Reconcile Order

Bases → Networks → VMs (clone+dataDisk, dependsOn) → Ports → Docker → DNS reload → Routes/Policies → Hooks

## DoD

- [ ] Schema + Reconciler + Example
- [ ] Lease + Journal/Resume
- [ ] Lockfile `.vzctl/stack.lock.json`

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[35] = f"""\
## Summary

ADR/Spec für Apply-Zustandsmaschine — Lease allein reicht nicht (SOL #4/#20).

## Muss spezifizieren

1. **Journal:** id, stack_id, generation, step, status, payload, timestamps
2. **Resume / Abort** Semantik + CLI flags
3. **Idempotenz** pro Step
4. **Drift:** YAML desired vs SQLite actual vs Lockfile
5. **Destruktiv:** was löscht `purge` (VMs, nets, resolver, contexts) — nur `managed-by=vzctl`
6. **Parallelität:** Lease holder, Timeout, Exitcode 6
7. **Wahrheitsquellen-Priorität** bei Konflikt

## Deliverable

- [ ] `docs/adr/0003-apply-state.md`
- [ ] State machine Diagramm

## Acceptance

- [ ] Review durch Implementierer von #37 ohne Ambiguität

{src('05-gpt-sol-review.md','01-implementation-plan.md')}
"""

B[36] = f"""\
## Summary

`hypernetwork/v1` Schema + Validation Errors die Menschen und Agents verstehen.

## Pflichtfelder (Auszug)

- `metadata.name`, `spec.project`, `spec.domain` (`.vz.test`)
- `dns`, `images`, `networks`, `routes`, `policies`, `vms`
- VM: `from`, `clone`, `dataDisk`, `networks[].ip`, `cloudInit`, `dependsOn`, `roles`, `requires` (v0.2)

## validate

```bash
vzctl validate -C ./examples/edge-dmz
```

Checks: JSON Schema + referentielle Integrität (route.via existiert, CIDR, IP in CIDR, dependsOn DAG, keine DHCP+static Kollision).

## Acceptance

- [ ] serde types + json schema export
- [ ] Fixture-tests positive/negative
- [ ] Fehler zeigen JSON-path

{src('01-implementation-plan.md')}
"""

B[37] = f"""\
## Summary

Reconcile-Engine implementieren gemäß ADR #35.

## CLI

```bash
vzctl up -C ./env
vzctl diff
vzctl apply --resume
vzctl apply --abort
vzctl down
vzctl down --purge
vzctl ps --format json
```

## Instance Isolation

- `project` + instance (Default: Hash aus Pfad)
- Physical names: `{{project}}-{{instance}}-{{vm}}`
- Lockfile gitignored

## Acceptance

- [ ] Zweites `up` = no-op
- [ ] Parallel apply → einer scheitert mit lock
- [ ] Crash mid-apply → resume/abort
- [ ] Events `apply.*`
- [ ] Golden plan tests

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[38] = f"""\
## Summary

Referenz-Environment `examples/edge-dmz` + CI validate/diff.

## Inhalt

```text
examples/edge-dmz/
  hypernetwork.config.yaml
  cloud-init/router.yaml
  cloud-init/web.yaml
  cloud-init/docker.yaml
  README.md
```

## YAML

Entspricht Plan-Skizze (domain `.vz.test`, dns dual, policies, linked clones, router `.2`).

## CI

- [ ] `vzctl validate`
- [ ] `vzctl diff` dry (ohne Daemon: schema-only mode ok)
- [ ] Keine Secrets committen

{src('01-implementation-plan.md')}
"""

B[39] = f"""\
## Summary

Docker-Engine in Stack-VM + Host-Context; Ports basic. **virtiofs → v0.1.x** (SOL Scope).

## Non-Goals Alpha

- BuildKit-Optimierung
- Registry-Cache
- TCP 2375 ohne SSH

## DoD

- [ ] `vzctl docker ps` gegen Context
- [ ] Port list + collisions
- [ ] DNS `docker.svc.{{project}}.vz.test`

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[40] = f"""\
## Summary

Docker-Host-VM + SSH-basierter Docker Context.

## Finding

Plan/SOL: `socketForward: 2375` auch loopback nur mit Warning — **SSH Default**.

## Umsetzung

1. `roles: [docker]` → dockerd, dataDisk für `/var/lib/docker`
2. SSH user/key aus cloud-init
3. `docker context create vzctl-{{project}} --docker host=ssh://…`
4. Wrapper:

```bash
vzctl docker -- ps
# setzt Context und reicht args durch
```

## Acceptance

- [ ] compose/build smoke von Host
- [ ] Context wird bei `down --purge` entfernt
- [ ] doctor prüft Context ping

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[41] = f"""\
## Summary

Host-Port-Forwards auf Guest-Services + Kollisionserkennung.

## YAML

```yaml
# VM-level
ports: ["8080:80"]
# Stack-level
ports:
  - "2222:router:22"
  - "5432:db:5432"
```

## Backend

1. Primär: vmnet port-forward (macOS 26+) wenn verfügbar
2. Fallback: userspace proxy oder `ssh -L`

## CLI

```bash
vzctl port list --format json
```

## Acceptance

- [ ] Collision → apply fail mit klarer Message
- [ ] Ingress bleibt Loopback-only (v0.2); Roh-Ports warnen bei `0.0.0.0`
- [ ] Purge entfernt Forwards

{src('01-implementation-plan.md')}
"""

B[42] = f"""\
## Summary

virtiofs Mounts + Perf-Benchmark — **v0.1.x**, nicht Alpha-Muss (SOL G4 / Fable Feedback-Loop).

## YAML

```yaml
volumes:
  web-src: ../app
vms:
  web:
    mounts:
      - {{ source: web-src, target: /srv/app }}
```

## Acceptance

- [ ] Mount read/write
- [ ] Benchmark vs. `multipass mount` grob dokumentiert
- [ ] Kohärenz-/Edge-Cases in Docs (sleep, rename, …)

{src('02-fable-review.md','05-gpt-sol-review.md')}
"""

B[43] = f"""\
## Summary

v0.2 Epic: **Caddy** Ingress, Local CA→Guests, **Dex** OIDC, optionale Tauri UI.

## Harte Regeln (Fable/SOL)

- Keinen eigenen IdP/Proxy schreiben → embedden
- OIDC Issuer = `https://auth.svc.{{project}}.vz.test` — **nie** `*.localhost`
- `*.localhost` nur Host-Alias auf dieselben Upstreams
- CA-Rollout in Guests für Trust

## Sub-Issues

Caddy, CA, Dex, Tauri — siehe Children.

{src('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md')}
"""

B[44] = f"""\
## Summary

Caddy als eingebetteter Reverse Proxy auf Loopback.

## Bind

- `127.0.0.1:80` / `:443`
- Config aus `ingress.routes`
- reload bei apply

## YAML

```yaml
ingress:
  enabled: true
  bind: 127.0.0.1
  hostAliases: true   # web.localhost → gleicher Upstream
  routes:
    - host: web.svc.edge-dmz.vz.test
      to: web:80
    - host: auth.svc.edge-dmz.vz.test
      to: oidc:5556
```

## Features

- HTTP→HTTPS redirect optional
- WebSocket/gRPC pass-through
- Certs von Local CA (#45)

## Acceptance

- [ ] https://web.svc.… vom Host
- [ ] Alias https://web.localhost wenn hostAliases
- [ ] Kein Guest braucht *.localhost

{src('01-implementation-plan.md')}
"""

B[45] = f"""\
## Summary

Local CA + automatisches Trust in Guests (system store).

## Host

```bash
vzctl certs ca init
vzctl certs ca install          # optional Keychain
vzctl certs mint web.svc.…
vzctl certs rollout
vzctl certs verify --vm web --url https://auth.svc.…
```

CA unter `~/Library/Application Support/vzctl/ca/` — **nicht** committen.

## Guest Rollout

1. NoCloud write_files → `/usr/local/share/ca-certificates/vzctl-local.crt`
2. `update-ca-certificates`
3. Live: Agent `ca_inject` für laufende VMs
4. Fingerprint im Lockfile; Drift → reinject

## Acceptance

- [ ] curl in Guest ohne `-k` zu Ingress/OIDC
- [ ] Rotate-Pfad dokumentiert (`onRotate: reinject`)
- [ ] Java-Store **Nice/out of Alpha** (SOL: Randfall)

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[46] = f"""\
## Summary

**Dex** als embedded OIDC Provider + Autoconfig für VMs mit `requires: [oidc]`.

## Finding

Fable: Eigenen OIDC-Provider schreiben = Scope-Falle. Dex/Rauthy embedden.

## Kanonischer Issuer

```text
https://auth.svc.{{project}}.vz.test
```

Niemals `https://auth.localhost` als Issuer (Guest-Resolve + Token `iss` Claim).

## Autoconfig

Für jede VM/Route mit `requires: [oidc]`:

- Client anlegen (`clients: auto`)
- Redirect URIs aus Ingress-Hosts ableiten
- Inject:

```bash
OIDC_ISSUER=https://auth.svc.edge-dmz.vz.test
OIDC_CLIENT_ID=web
OIDC_CLIENT_SECRET=…
OIDC_REDIRECT_URI=https://web.svc.edge-dmz.vz.test/oauth2/callback
OIDC_CA_PATH=/etc/ssl/certs/ca-certificates.crt
```

Secrets in `.vzctl/oidc/` gitignored.

## Dev Users

Aus YAML / `passwordFile` (keine Prod-Passwörter in Git).

## Acceptance

- [ ] Discovery `/.well-known/openid-configuration`
- [ ] Auth Code + PKCE Login über Browser am Host
- [ ] Guest-App validiert Token gegen Issuer+CA
- [ ] `vzctl oidc status|clients|token`

{src('02-fable-review.md','04-decision-log.md','01-implementation-plan.md')}
"""

B[47] = f"""\
## Summary

Tauri 2 UI **nach** stabiler CLI — gleiche Reconcile-Engine, keine zweite Logik (SOL).

## Views

- Open Environment (Ordner mit `hypernetwork.config.yaml`)
- Up / Down / Apply / Diff / Purge
- Topologie-Graph (nets/vms/routes)
- DNS/OIDC/CA Status
- Deep link `vzctl://…` optional

## Regeln

- [ ] Kein Feature ohne CLI-Äquivalent
- [ ] Kein direktes VZ im UI-Prozess
- [ ] Events für Live-State

## Acceptance

- [ ] edge-dmz open → up → Status grün
- [ ] Fehler aus CLI/Daemon verständlich angezeigt

{src('01-implementation-plan.md','05-gpt-sol-review.md')}
"""

B[48] = f"""\
## Summary

DX-Epic: Logs, Diagnose, Docs-Tracking.

## Alpha Minimum

- `vzctl vm logs`
- doctor (siehe #20)
- Issue↔Plan Mapping aktuell halten (#50)

## v0.1.x

- Diagnose-Bundles
- Admission/RAM Warnungen (SOL Should)

{src('05-gpt-sol-review.md')}
"""

B[49] = f"""\
## Summary

VM-Logs fürs Debugging ohne volles Observability-System.

## Quellen

1. Serial console buffer (Helper)
2. Agent log-tail (wenn verfügbar)

## CLI

```bash
vzctl vm logs web
vzctl vm logs web -f
vzctl vm logs web --format json
```

## Acceptance

- [ ] Follow-Mode
- [ ] Klare Fehler wenn VM aus/agent down
- [ ] Keine Passwörter aus cloud-init in Default-Logs leaken (Filter Hinweis)

{src('01-implementation-plan.md')}
"""

B[50] = f"""\
## Summary

Dokumentation: GitHub Epics/Stories ↔ Plan-Abschnitte synchron halten.

## Tasks

- [ ] `docs/planing/06-github-tracking.md` bei neuen Issues aktualisieren
- [ ] README Lesereihenfolge / „Nächster Schritt“ pflegen
- [ ] Nach G0: Spike-Notes verlinken

## Acceptance

- [ ] Neue Contributor finden in <5 min den Einstieg (#1)

{src('06-github-tracking.md','README.md')}
"""


def main() -> None:
    missing = [n for n in range(1, 51) if n not in B]
    if missing:
        raise SystemExit(f"missing bodies for: {missing}")
    for n in range(1, 51):
        edit(n, B[n])
    print("done", len(B))


if __name__ == "__main__":
    main()
