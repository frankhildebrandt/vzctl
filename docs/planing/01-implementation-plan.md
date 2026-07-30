# Implementationsplan: Apple-VZ Hypervisor (vzctl)

> Stand: 2026-07-30 · inkl. Must-Fixes aus der Fable-Review  
> Canvas-Quelle: [`canvases/vz-hypervisor-implementationsplan.canvas.tsx`](canvases/vz-hypervisor-implementationsplan.canvas.tsx)

## Ziel

Devstack-Supervisor auf **Virtualization.framework**:

- Git-native Environments (`hypernetwork.config.yaml`) — `up` / `down` / `apply`
- Custom Networks + manuelles Routing (Router-VM)
- Shared Base + APFS Linked Clones + `dataDisk` + Identity-Reset
- Native Docker-Context, Port-Forwards
- Hypervisor-DNS (intern) + macOS-Resolver
- v0.2: Ingress, Local CA→Guests, Embedded OIDC (Dex)

**Nicht VZ:** Windows (QEMU später oder out-of-scope).

**Positionierung:** „compose für VM-Topologien“ — nicht besseres Docker Desktop.

---

## Architektur (Must-Fix)

```
Git Env → CLI/UI → Reconciler → Supervisor
                                    ├─ DNS + macOS Resolver
                                    └─ VM Helper (1:1) ⇄ vsock Agent ⇄ Guest
```

### Supervisor (Swift, schlank)

- UDS-RPC, SQLite, Stack-Locks
- Spawnt/überwacht VM-Helper (launchd/XPC)
- Besitzt **vmnet-Network-Refs** + **DNS-Service** (interne Zone)
- Crash/Update darf laufende Helpers nicht killen

### VM-Helper (1 Prozess / VM)

- Hält genau eine `VZVirtualMachine` + vsock zum Guest-Agent
- Crash = nur diese eine VM betroffen

### vsock Guest-Agent

- Install via cloud-init in Base/Seal
- Capabilities: exec, IP/Health, Time-Sync nach Sleep, CA-Inject, Log-Tail
- SSH = Fallback, nicht Control-Plane

### Reconciler + Locking

- Shared Rust-Crate in CLI + Tauri
- `stack.apply`-Lease im Supervisor
- `vzctl adopt` für verwaiste Instanzen

---

## DNS: Hypervisor intern + macOS-Resolver

### Warum nicht `auth.localhost` in Guests?

`*.localhost` resolved im Guest auf **dessen** Loopback (RFC 6761) → OIDC/Ingress-Issuer würden brechen.

### Namensschema

| Kontext | Beispiel |
|---|---|
| Kanonisch (Guests, OIDC, inter-VM) | `web.dmz.edge-dmz.vz` |
| System-Services | `auth.svc.edge-dmz.vz` |
| Host-Alias (v0.2, Browser) | `web.localhost` → gleicher Upstream |

### Hypervisor-DNS

- Supervisor = autoritativ für `*.{project}.vz`
- Records aus Actual State (VM-Attachments, Services)
- Guests: DNS = Gateway/Hypervisor-IP via cloud-init / DHCP option 6
- Search-Domain z. B. `dmz.{project}.vz`

### macOS-Resolver

- `/etc/resolver/{project}.vz` → Hypervisor-DNS (Loopback-Listener)
- `vzctl dns install-resolver` / `uninstall-resolver` / `reload` / `query`

```yaml
spec:
  domain: edge-dmz.vz
  dns:
    enabled: true
    hostResolver: true
```

---

## IP-Vergabe (Spike P2)

| Mode | Wer vergibt IP? | Hinweis |
|---|---|---|
| shared (vmnet ≥26) | vmnet DHCP + Reservations **oder** cloud-init static | Precedence klar spezifizieren |
| host | vmnet / Daemon | |
| bridged | LAN-DHCP | braucht `com.apple.vm.networking` |
| pre-26 Fallback | NAT + Router-VM | testen oder Baseline = 26 |

---

## Phasen

| Phase | Name | Zeit | Deliverable |
|---|---|---|---|
| P0 | Foundation | 1–3 Wo | Supervisor + Helper, vsock-Agent, doctor |
| P1 | CLI + Clones | 2–4 Wo | JSON-CLI, Seal/clonefile, Identity-Reset, events |
| P2 | Net + DNS | 3–6 Wo | vmnet, IP-Modell, Hypervisor-DNS, macOS-Resolver |
| P3 | Stacks | 5–8 Wo | `hypernetwork.config.yaml` up/down/apply + Locking |
| P4 | Docker + Ports | 7–9 Wo | Docker-Context, Ports, virtiofs |
| P5 | Ingress + CA + OIDC | **v0.2** | Caddy, CA→Guests, Dex, `*.localhost` Aliase |
| P6 | Tauri UI | **v0.2** | Stack-Browser |
| P7 | Harden | ongoing | Signing, Snapshots, k3s-Rolle |

---

## Config-Skizze (v0.1 relevant)

```yaml
apiVersion: hypernetwork/v1
kind: Environment
metadata:
  name: edge-dmz
spec:
  project: edge-dmz
  domain: edge-dmz.vz
  dns:
    enabled: true
    hostResolver: true

  images:
    ubuntu-base:
      from: ubuntu:24.04
      role: base

  networks:
    dmz:
      cidr: 10.80.0.0/24
      mode: shared
    lan:
      cidr: 10.90.0.0/24
      mode: shared

  routes:
    - name: dmz-to-lan
      from: dmz
      to: lan
      via: router

  vms:
    router:
      from: ubuntu-base
      clone: linked
      dataDisk: 4G
      networks:
        - { name: dmz, ip: 10.80.0.1 }
        - { name: lan, ip: 10.90.0.1 }
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

1. Base sealed (`role: base`)
2. Pro VM: APFS `clonefile` der Base + neues `dataDisk`
3. Auto: neue MACs, machine-id, Hostname, SSH Host Keys, cloud-init instance-id

### v0.2 Auszug

```yaml
ca:
  name: vzctl-local
  rollout:
    enabled: true
    targets: all
    stores: [system]

ingress:
  enabled: true
  bind: 127.0.0.1
  hostAliases: true
  routes:
    - host: web.svc.edge-dmz.vz
      to: web:80
    - host: auth.svc.edge-dmz.vz
      to: oidc:5556

oidc:
  enabled: true
  issuer: https://auth.svc.edge-dmz.vz   # nie *.localhost
  mode: embedded                         # Dex
  clients: auto
  autoconfig:
    inject: cloud-init
```

VMs mit `requires: [oidc]` bekommen Issuer/Client/CA per Autoconfig.

---

## MVP-Schnitt

### v0.1 Muss (~8–10 Wochen)

- Supervisor + Helper-pro-VM + vsock-Agent
- CLI JSON + events + doctor
- Netze + Router-Routing + IP-Modell
- Hypervisor-DNS + macOS `/etc/resolver`
- Stacks up/down/apply + Locking + Clones/Identity
- Docker-Context + Ports (+ virtiofs Basis)

### v0.2+

- Caddy Ingress + Local CA + Guest-Rollout
- Dex OIDC + `requires` Autoconfig
- `*.localhost` Host-Aliase
- Tauri UI, Stack-Snapshots, k3s-Rolle

---

## Kickoff-Tickets

1. Supervisor↔Helper Protokoll + Crash-Isolation-Test — P0  
2. vsock Guest-Agent: ping/exec/report-ip + cloud-init — P0  
3. `vzctl doctor` + UDS RPC health — P0  
4. CLI vm lifecycle `--format json` + events schema — P1  
5. image seal + APFS linked clone + identity reset — P1  
6. Spike: IP/DHCP Precedence + macOS 26 vs Fallback — P2  
7. vmnet nets + Router-VM routes — P2  
8. Hypervisor-DNS Zone `*.project.vz` — P2  
9. macOS `/etc/resolver` Installer + dns query — P2  
10. hypernetwork/v1 + reconcile + stack lease — P3  
11. Docker context + ports + virtiofs spike — P4  
12. v0.2: Caddy + Dex + CA rollout + hostAliases — P5  

---

## CLI (Auszug)

```text
vzctl up|down|apply|diff|ps|validate|adopt
vzctl vm create|start|stop|delete|list|info|exec|console
vzctl image pull|seal
vzctl net create|attach|list
vzctl route add|apply
vzctl dns status|query|reload|install-resolver|uninstall-resolver
vzctl docker …          # Context-Wrapper
vzctl events subscribe
vzctl doctor
# v0.2:
vzctl ingress up|down|status
vzctl certs ca init|install|rollout|verify
vzctl oidc status|users|clients|token
```

## Repo-Layout (Ziel)

```text
vzctl/
  crates/
    vzctl/
    vzctl-client/
    vzctl-schema/
    vzctl-reconcile/
  daemon/                 # Swift Supervisor + Helper
  guest-agent/            # vsock agent
  ui/                     # Tauri (v0.2)
  examples/edge-dmz/
  docs/planing/           # dieses Dokument
```
