# Implementationsplan: Apple-VZ Hypervisor (vzctl)

> Stand: 2026-07-30 · Fable-Must-Fixes + [GPT-SOL-Must-Fixes](05-gpt-sol-review.md)  
> Canvas-Quelle: [`canvases/vz-hypervisor-implementationsplan.canvas.tsx`](canvases/vz-hypervisor-implementationsplan.canvas.tsx)  
> v0.1 = **Alpha** (Walking Skeleton), kein Alltagsprodukt

## Ziel

Devstack-Supervisor auf **Virtualization.framework**:

- Git-native Environments (`hypernetwork.config.yaml`) — `up` / `down` / `apply`
- Custom Networks + Routing (Router-VM) + Firewall-Policy
- Shared Base + APFS Linked Clones + `dataDisk` + Identity-Reset
- Hypervisor-DNS (intern) + macOS-Resolver
- Docker-Context + Port-Forwards (Polish in v0.1.x)
- v0.2: Ingress (Caddy), Local CA→Guests, OIDC (Dex)

**Nicht VZ:** Windows (QEMU später oder out-of-scope).

**Positionierung:** „compose für VM-Topologien“ — nicht besseres Docker Desktop.

---

## Gates vor Scaffolding (SOL Must)

| # | Gate | Abbruch wenn |
|---|---|---|
| G0 | **Netzwerk-Spike vor P0** (2 Netze, Router, feste IP, Host↔Guest, Sleep, Supervisor-Crash) | Isolation/Entitlements unmöglich |
| G1 | **macOS-Baseline** | **Erledigt:** Mindestversion **macOS 26** ([ADR 0001](../adr/0001-macos-baseline.md), #2) |
| G2 | **Process-/Ownership-ADR** (VZ, vmnet, DNS, Helper-Lifecycle) | — |
| G3 | **State/Apply-Spez** (Journal, Idempotenz, Resume, Löschregeln) | — |
| G4 | MVP-Exit-Kriterien messbar; virtiofs + Docker-Polish → **v0.1.x** | Scope sprengt 8–10 Wo |

Details: [Decision Log](04-decision-log.md).

---

## Architektur

```
Git Env → CLI/UI → Reconciler → Supervisor
                      │              ├─ DNS (Host-Loopback + Guest-fähiger Listener)
                      │              ├─ vmnet Network Registry (Owner: Supervisor)
                      │              └─ launchd: VM Helper (1:1)
                      │                        ⇄ vsock Agent (in sealed Base)
                      └─ apply Journal / Lease
```

### Process- & Ressourcen-Ownership (ADR-Pflicht)

| Ressource | Owner | Nach Supervisor-Crash |
|---|---|---|
| `VZVirtualMachine` | **VM-Helper** | VM läuft weiter (Helper unabhängig) |
| vmnet network refs | **Supervisor** (Registry); Helper bekommt Attachment-Handle/ID beim Start | Net ggf. tot → Helper meldet `net_orphaned`; Reconnect nach Supervisor-Restart |
| DNS Zone + Listener | **Supervisor** | DNS down → Guests/Host resolve fail until restart (akzeptiert in Alpha; dokumentieren) |
| Stack-Lease / Journal | **Supervisor** | Incomplete ops → `apply --resume` |

**Helper-Lifecycle (konkret):**

- Supervisor registriert Helper als launchd-Job pro VM-ID
- Start: Supervisor erstellt/attach Net → spawnt Helper mit Config-Pfad + net-handle
- Reconnect: Helper hält UDS rückwärts zum Supervisor; bei Disconnect Retry + State-Report
- Upgrade: Helper-Binary versioniert; Rolling replace nur gestoppte VMs in Alpha
- Doppel-Helper: VM-ID Lockfile + Supervisor adopt/kill stale

### vsock Guest-Agent

- **Im sealed Base-Image vorinstalliert** (nicht erst per First-Boot cloud-init installieren)
- cloud-init aktiviert/konfiguriert nur (Identity, Hostname)
- Capabilities: exec, IP/Health, Time-Sync nach Sleep, CA-Inject, Log-Tail
- vsock Auth: Token aus NoCloud / shared secret pro VM
- SSH = Fallback; Bootstrap-Fenster: Serial + Agent-ready Event

### Reconcile-Vertrag (Alpha)

- Desired = YAML; Actual = SQLite; Lockfile = lokale Instanz-Map
- Jede `apply`-Op im **Journal** (id, gen, step, status)
- Idempotent; Crash → `apply --resume` / `--abort`
- Drift: `diff` zeigt YAML↔Actual; recreate nur mit `--force`
- Destruktiv: `down` stoppt; `down --purge` löscht nur `managed-by=vzctl`-Ressourcen + Resolver-Dateien des Projekts
- `vzctl adopt` für Orphans

---

## DNS: Hypervisor + macOS-Resolver

### Domain

Kanonisch: `{vm}.{net}.{project}.vz.test`  
(reservierte Test-TLD — nicht `.vz`)

| Kontext | Beispiel |
|---|---|
| Guest / inter-VM / OIDC | `web.dmz.edge-dmz.vz.test` |
| Services | `auth.svc.edge-dmz.vz.test` |
| Host-Alias v0.2 | `web.localhost` → gleicher Upstream |

### Dual Listener

| Listener | Bind | Wer nutzt |
|---|---|---|
| Host | `127.0.0.1:<dnsPort>` | `/etc/resolver/{project}.vz.test` |
| Guest-erreichbar | Hypervisor/Gateway-IP auf vmnet (oder shared DNS-IP) | Guests via cloud-init `nameservers:` |

- Zone autoritativ für `*.{project}.vz.test`
- **Forwarding** für externe Namen (Upstream = System-DNS / konfigurierbar; VPN-Verhalten dokumentieren)
- TTL klein (z. B. 5–30s) für schnelle apply-Updates
- `vzctl dns query` spricht **direkt** den vzctl-DNS (nicht nur libc/`dig`)
- `install-resolver` / Cleanup verwaister `/etc/resolver/*` bei purge

```yaml
spec:
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    # guestListen: abgeleitet aus Gateway / Spike-Ergebnis
    forward:
      enabled: true
      upstream: system
```

---

## Netzwerk & IP (Spike = G0)

| Mode | IP-Vergabe (v0.1) | Hinweis |
|---|---|---|
| shared (vmnet ≥26) | **cloud-init static** Primär; optional DHCP reservation aligned | Kein wildes DHCP+static Mix; **Host ≥ macOS 26** |
| host | wie shared | |
| bridged | **out of scope** (Entitlement) | |
| pre-26 | **unsupported** (ADR 0001) | Kein Compatibility-Layer |

- Router-IPs **nicht** `.1` wenn Gateway `.1` ist — Spike legt Gateway-CIDR-Konvention fest (z. B. Router `.2` / Gateway `.1` oder umgekehrt)
- `routes:` + **`policies:`** (forward allow/deny) für echte DMZ-Semantik
- Sleep/VPN/Crash = Akzeptanztests im Spike

---

## Phasen

| Phase | Name | Zeit | Deliverable |
|---|---|---|---|
| **G0** | Spike | Wo 0–1 | Netz+DNS+Crash Go/No-Go |
| P0 | Foundation | 1–3 | Supervisor+Helper ADR, Agent-in-Base, doctor, Journal-Stub |
| P1 | CLI + Clones | 2–4 | JSON-CLI, Exitcodes, events schema, Seal/clonefile, Identity |
| P2 | Net + DNS | 3–5 | vmnet, IP-Modell, Dual-DNS, macOS-Resolver, Router+Policy |
| P3 | Stacks | 5–7 | hypernetwork up/down/apply + Lease + Resume |
| P4 | Docker + Ports | 7–9 | Docker-Context (SSH), Ports; **kein** virtiofs-Muss |
| P4b | v0.1.x | nach Alpha | virtiofs + Docker-Polish + Logs/Diagnose |
| P5 | Ingress + CA + OIDC | **v0.2** | Caddy, CA→Guests, Dex |
| P6 | Tauri | **v0.2** | nach stabiler CLI |
| P7 | Harden | ongoing | Signing, Snapshots, k3s |

---

## Config-Skizze (v0.1)

```yaml
apiVersion: hypernetwork/v1
kind: Environment
metadata:
  name: edge-dmz
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }

  images:
    ubuntu-base:
      from: ubuntu:24.04
      role: base
      # guest-agent vorinstalliert vor seal

  networks:
    dmz:
      cidr: 10.80.0.0/24
      mode: shared
      # gateway: aus Spike (nicht mit Router kollidieren)
    lan:
      cidr: 10.90.0.0/24
      mode: shared

  routes:
    - name: dmz-to-lan
      from: dmz
      to: lan
      via: router

  policies:                         # Firewall / Isolation
    - name: dmz-default
      network: dmz
      forward: deny-all
      allow:
        - { to: lan, proto: tcp, ports: [5432] }  # Beispiel

  vms:
    router:
      from: ubuntu-base
      clone: linked
      dataDisk: 4G
      networks:
        - { name: dmz, ip: 10.80.0.2 }   # .1 = Gateway-Konvention Spike
        - { name: lan, ip: 10.90.0.2 }
      cloudInit: cloud-init/router.yaml
      roles: [router]

    web:
      from: ubuntu-base
      clone: linked
      dataDisk: 40G
      dependsOn: [router]
      networks:
        - { name: dmz, ip: 10.80.0.10 }
      cloudInit: cloud-init/web.yaml

    docker:
      from: ubuntu-base
      roles: [docker]
      dataDisk: 100G
      networks:
        - { name: dmz, ip: 10.80.0.50 }
```

### Linked Clone / Identity

1. Base mit Agent sealen (`role: base`, immutable)
2. APFS `clonefile` + neues `dataDisk`
3. Auto: MACs, machine-id, Hostname, SSH Host Keys, instance-id
4. Disk-Lifecycle: Seal nie schreiben; purge löscht Clone+dataDisk; Base bleibt

### v0.2 Auszug

```yaml
oidc:
  enabled: true
  issuer: https://auth.svc.edge-dmz.vz.test
  mode: embedded   # Dex
  clients: auto
```

---

## MVP-Schnitt

### v0.1 Alpha (~8–10 Wochen) — Muss

- G0 Spike bestanden
- Supervisor + Helper-pro-VM + Ownership-ADR
- vsock-Agent **in Base**
- CLI JSON + versioniertes Event-Schema + Exitcodes + doctor
- Netze + Router + policies + IP-Modell
- Dual-DNS + macOS `/etc/resolver/*.vz.test`
- Stacks up/down/apply mit Journal/Resume + Locking + Clones
- Docker-Context (SSH) + Ports (basic)
- `vzctl logs` (pro VM) Mindesthilfe

### v0.1.x

- virtiofs + Perf-Messung
- Docker-Polish (BuildKit hints)
- Diagnose-Bundles, Admission/RAM-Warnungen

### v0.2+

- Caddy + Local CA-Rollout + Dex OIDC
- `*.localhost` Host-Aliase
- Tauri, Snapshots, k3s

---

## Kickoff-Tickets

0. **G0 Netz-/DNS-/Crash-Spike + macOS-Baseline ADR** — vor P0  
1. Process-/Ownership-ADR + Helper launchd Lifecycle — P0  
2. State/Apply-Spez (Journal, Resume, Purge-Regeln) — P0  
3. Guest-Agent in Base-Image + vsock ping/exec/report-ip — P0  
4. `vzctl doctor` + UDS health — P0  
5. CLI vm lifecycle `--format json` + events schema + Exitcodes — P1  
6. image seal + APFS linked clone + identity reset — P1  
7. vmnet nets + Router routes + policies — P2  
8. Dual-DNS (Host+Guest Listener) + forward + `dns query` — P2  
9. macOS `/etc/resolver/*.vz.test` install/cleanup — P2  
10. hypernetwork/v1 reconcile up/down/apply + lease + resume — P3  
11. Docker SSH-context + ports (basic) — P4  
12. v0.1.x: virtiofs spike + Docker polish — P4b  
13. v0.2: Caddy + Dex + CA rollout + hostAliases — P5  

---

## CLI (Auszug)

```text
vzctl up|down|apply|diff|ps|validate|adopt
vzctl apply --resume|--abort
vzctl vm create|start|stop|delete|list|info|exec|console|logs
vzctl image pull|seal
vzctl net create|attach|list
vzctl route add|apply
vzctl policy apply
vzctl dns status|query|reload|install-resolver|uninstall-resolver
vzctl docker …
vzctl events subscribe
vzctl doctor
# v0.2: ingress / certs / oidc
```

## Repo-Layout (Ziel)

```text
vzctl/
  crates/ …
  daemon/                 # Supervisor + Helper
  guest-agent/
  docs/planing/
  docs/adr/               # Ownership, Baseline, Apply (nach G0)
  examples/edge-dmz/
```
