# Port Forwards (Alpha)

Host-Port-Forwards auf Guest-Services. Alpha bindet nur `127.0.0.1` (Userspace-Proxy
im Supervisor). `0.0.0.0` ist Validate-Fehler; Ingress bleibt v0.2 Loopback-only.

## YAML

```yaml
spec:
  ports:
    - "8080:web:80"
    - "127.0.0.1:2222:router:22"
  vms:
    web:
      ports: ["8080:80"]   # VM-level: Ziel = diese VM
```

Formate:

| Form | Bedeutung |
|---|---|
| `hostPort:guestPort` | VM-level |
| `bind:hostPort:guestPort` | VM-level mit Bind |
| `hostPort:vm:guestPort` | Stack-level |
| `bind:hostPort:vm:guestPort` | Stack-level mit Bind |

Doppelte Host-`(bind,port)` → Validate Exit `3`. Belegter Port zur Apply-Zeit →
Step-Fail.

## CLI

```bash
vzctl port list
vzctl port list --project edge-dmz --format json
```

## Lifecycle

- Apply-Step `ensure_ports` nach Agents (Guest-IP aus Attachments).
- `down --purge` → `port.purge` (Listener + SQLite).
- Persistenz: Supervisor-Tabelle `port_forwards`; Reload beim Supervisor-Start.
