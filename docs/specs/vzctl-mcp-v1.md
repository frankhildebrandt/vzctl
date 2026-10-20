# vzctl MCP Server v1

Model Context Protocol (stdio) für AI-Agents auf der lokalen vzctl Control-Plane.

## Binary

`vzctl-mcp` — Workspace-Crate `crates/vzctl-mcp`, Transport **stdio** (Cursor/Claude Desktop).

## Voraussetzungen

- macOS 26+, laufender `vz-supervisor serve` (REST auf `api.sock`)
- `vzctl` auf `PATH` (für `vm_exec`, `vm_logs`)
- Gleiche Peer-UID-Sicherheit wie REST-UDS (`0600`, lokaler User)

## Konfiguration

| Env | Default | Beschreibung |
|---|---|---|
| `VZCTL_API_LISTEN` | `unix:$VZCTL_STATE_DIR/api.sock` | Supervisor REST (wie UI) |
| `VZCTL_STATE_DIR` | `~/Library/Application Support/vzctl` | State-Verzeichnis |
| `VZCTL_BIN` | `vzctl` | CLI-Pfad für Exec/Logs, Diagnose, Controller- und Kubernetes-Status |

### Cursor / Claude Desktop

`.cursor/mcp.json` (Projekt) oder User-MCP-Config:

```json
{
  "mcpServers": {
    "vzctl": {
      "command": "/Users/you/.local/bin/vzctl-mcp",
      "env": {
        "VZCTL_BIN": "/Users/you/.local/bin/vzctl"
      }
    }
  }
}
```

Nach `make install` liegt das Binary unter `~/.local/bin/vzctl-mcp`.

## Architektur

```
Agent (MCP client)
    │ stdio JSON-RPC
    ▼
vzctl-mcp (rmcp tools)
    ├─► Supervisor REST /v1/*  (Steuerung, Guest-Services, systemd, Docker)
    └─► vzctl CLI --format json (vm exec, vm logs)
```

Erweiterbarkeit: neue Tool-Gruppen als `impl VzctlMcp { #[tool] … }` in
`crates/vzctl-mcp/src/tools/` (z. B. `nats.rs`, `postgres.rs`) — additive Tools,
kein Breaking Change innerhalb v1.

## Tool-Katalog (v1)

### Debug / Host

| Tool | Backend | Beschreibung |
|---|---|---|
| `doctor` | REST | `GET /v1/doctor` |
| `health` | REST | `GET /v1/health` |
| `dns_status` | REST | `GET /v1/dns/status` |
| `port_list` | REST | `GET /v1/ports` |
| `diagnose` | CLI | Redigierter Host-/VM-/Journal-Report, maximal 64 KiB |

### VM Lifecycle

| Tool | Backend | Beschreibung |
|---|---|---|
| `vm_list` | REST | Runtime-Liste |
| `vm_inspect` | REST | Inspect-Bundle |
| `vm_start` / `vm_stop` / `vm_restart` | REST | Lifecycle |
| `vm_stats` | REST | Guest-Agent Stats |
| `vm_exec` | CLI | One-Shot Guest-Exec (argv, kein Shell) |
| `vm_logs` | CLI | Serial-Log Tail |

### Stacks / Jobs

| Tool | Backend | Beschreibung |
|---|---|---|
| `stack_list` | REST | Registry |
| `stack_status` | REST | Status-Bundle einschließlich `sections.health`, OIDC und Ingress |
| `controller_status` | CLI | Letzter Reconcile-Stand und Conditions für `stack_path` |
| `kubernetes_status` | CLI | Node-/Manifest-Status für `stack_path`, optional `cluster` |
| `stack_validate` | REST | YAML validate |
| `stack_apply` | REST | Apply → `jobId` |
| `job_status` | REST | Job poll + log |

### Guest Services / systemd

| Tool | Backend | Beschreibung |
|---|---|---|
| `guest_services_list` | REST | Agent-Publisher |
| `guest_service_request` | REST | HTTP-Proxy auf Publisher-API |
| `systemd_status` | REST | Capability |
| `systemd_list_units` | REST | Unit-Liste |
| `systemd_start_unit` / `stop` / `restart` | REST | Unit-Control |

### Docker

| Tool | Backend | Beschreibung |
|---|---|---|
| `docker_ps` | REST | `docker ps` auf Project-Docker-VM |

## Konventionen

- VM-IDs: `{project}/{vm}`; in REST-Pfaden URL-encoded (`%2F`).
- Tool-Antworten: pretty-printed JSON (Text-Content) außer `guest_service_request` (HTTP status + body).
- Fehler: Tool-level `Err(String)` — Supervisor/CLI-Fehlertext für den Agent.
- Interaktive Shell (`vm exec -it`): **nicht** im MCP v1 (nur One-Shot exec).

### Begrenzte Betriebsdiagnose

`controller_status`, `kubernetes_status`, `diagnose` und `stack_status` sind mit
`readOnlyHint=true` beschrieben. Die neuen CLI-Tools verwenden feste Argumentlisten,
höchstens 60 Sekunden Laufzeit und 1 MiB Prozessausgabe. Controller-Warnzustände
(auch Exit 24) bleiben auswertbare Daten. Unstrukturierte Fehlerausgaben werden
nicht zurückgegeben.

Controller-Status enthält Claim-Zustände (auch Pending/Retained), Kubernetes-
Quellidentität und Warncodes, sowie CP-Registrierung (`enabled`, `generation`, `lastAttemptAt`, sichere
`lastStatus`-Felder). Öffentliche Bindings erlauben ausschließlich `host`, `port`,
`database`, `user`, `bucket`, `region`, `clientID`, `scopes`, `caFile`, `url`,
`issuer`, `endpoint`; keine Credential-Felder oder beliebigen Zusatzobjekte.
URLs mit Userinfo, Query, Fragment oder Encodings werden vollständig redigiert.
Freie Warntexte werden nicht übertragen. Kubernetes-Status
enthält ausschließlich Ressourcenidentität, Zustand und ausgewählte Conditions;
keine Secret-Daten, Kubeconfig, freien Condition-Meldungen oder Guest-Fehlertexte.
`issues` enthält feste Codes und Operation/Ressourcenidentität einschließlich
Namespace, etwa `NodeInventoryUnavailable`, `ResourceReadFailed`,
`InvalidResourceInventory`, `OwnershipConflict` oder `ResourceDrift`.
Pro Aufruf werden höchstens 256 Claims bzw. 64 Cluster mit jeweils 256 Nodes und
Manifesten übertragen.

`diagnose` erzeugt das CLI-Bundle in einem privaten temporären Verzeichnis und
entfernt es vor Rückgabe. Es entsteht keine dauerhafte Ausgabedatei.
Optionales `stack_path` ergänzt begrenzte Controller-/K3s-Zusammenfassungen über
`diagnose -C`; ohne Parameter werden keine Stack-Pfade automatisch abgefragt. Der Report
enthält ausgewählte Host-/Health-/VM-/Journal-Felder, keine Rohlogs, Konfiguration,
Environment oder Credentials. `stack_status` bleibt das bestehende REST-Bundle;
der Health-Bereich ist stackbezogen und wird nicht durch diese Projektion ersetzt.

## Geplante Erweiterungen

- REST-`vm_exec` wenn Supervisor-Endpoint landet (heute CLI)
- Optional: SSE-Events als MCP Resources/Subscriptions

Debug data sources (`debug_*` tools) sind implementiert — siehe [debug-sources-v1.md](debug-sources-v1.md).

## Compatibility

Additive Tool-Namen innerhalb v1. Breaking Changes → Spec `vzctl-mcp-v2`.
