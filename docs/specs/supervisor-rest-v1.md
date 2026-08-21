# Supervisor REST API v1

Lokale Control-Plane für UI und Tools. Ergänzt JSON-RPC über `vz.sock`
([events-v1](events-v1.md), CLI Contract). CLI bleibt auf JSON-RPC; die UI
nutzt REST.

## Transport

| Mode | Spec | Default |
|---|---|---|
| Unix HTTP | `unix:<path>` | `unix:$VZCTL_STATE_DIR/api.sock` |
| TCP HTTP | `tcp:<host>:<port>` | — |

Konfiguration:

- Env `VZCTL_API_LISTEN` (Default: `unix:$VZCTL_STATE_DIR/api.sock`)
- Flag `vz-supervisor serve --api-listen <spec>` (überschreibt Env)

Sicherheit:

- UDS: Mode `0600`, `LOCAL_PEERCRED` Peer-UID muss `geteuid()` sein
- TCP: nur Loopback (`127.0.0.1` / `::1`) in v1; keine Tokens

Die Tauri-UI proxyt immer über einen ephemeral Loopback-Listener zur
Supervisor-REST (WebView kann kein UDS).

## Konventionen

- Prefix `/v1`
- JSON bodies; `Content-Type: application/json`
- Erfolg: `200` / `201` / `202` mit JSON-Body
- Fehler:

```json
{"error":{"code":"not_found","message":"stack missing","details":{}}}
```

Stabile Error-Codes: `bad_request`, `not_found`, `conflict`, `unauthorized`,
`failed_precondition`, `internal`, `not_implemented`.

- Resource-IDs mit `/` (z. B. `project/vm`) sind URL-encoded (`project%2Fvm`).
- Long-running Ops antworten mit `202` und `{ "jobId": "…" }`. Status unter
  `GET /v1/jobs/{jobId}`; Fortschritt auch über SSE Events.

## Endpoints

### Daemon

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/health` | Health (wie `daemon.health`) |
| `GET` | `/v1/version` | Supervisor-Version |

`/v1/health` und `daemon.health` enthalten additiv `network_resilience` mit
`state`, monotoner `epoch`, `internal_ok`, Host-/Netz-Egress,
`cidr_conflicts`, letztem Event/Fehler und Transition-Zeitpunkt. `vzctl doctor`
meldet Offline/Captive/Konflikt als WARN und nur `internal_ok=false` als FAIL.

### Events

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/events?filter=` | SSE (`text/event-stream`); `data:` = Event-Envelope JSON |

### Jobs

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/jobs/{jobId}` | Job-Status (`queued`/`running`/`succeeded`/`failed`) plus `log[]` (bisherige Console-Zeilen) |
| `GET` | `/v1/jobs/{jobId}/log` | SSE Log-Zeilen (Console); live während `running` |

Job-Worker setzen `VZCTL_PROGRESS=1` und starten Stack-Jobs explizit mit
`--progress plain`. Dadurch landen Uhrzeit, Prozent, Jobhierarchie sowie
Image-/Cloud-init-Phasen auch bei `--format json` auf stderr und in `log[]` /
SSE. stdout bleibt das JSON-Ergebnis.

### Stacks

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/stacks` | Registry-Liste |
| `POST` | `/v1/stacks` | Registrieren/anlegen `{path,name?}` |
| `DELETE` | `/v1/stacks/{id}` | Aus Registry entfernen (kein FS-Delete) |
| `GET` | `/v1/stacks/{id}` | Meta + path |
| `GET`/`PUT` | `/v1/stacks/{id}/config` | `hypernetwork.config.yaml` (text/plain oder JSON `{content}`) |
| `GET`/`PUT` | `/v1/stacks/{id}/diagram` | `.vzctl/diagram.json` |
| `POST` | `/v1/stacks/{id}/validate` | `vzctl validate` |
| `GET` | `/v1/stacks/{id}/diff` | `vzctl diff` |
| `GET` | `/v1/stacks/{id}/status` | Status-Bundle |
| `POST` | `/v1/stacks/{id}/up` | Body `{force?,resume?,abort?}` → Job |
| `POST` | `/v1/stacks/{id}/apply` | Body `{force?,resume?,abort?}` → Job |
| `POST` | `/v1/stacks/{id}/down` | Body `{purge?}` → Job |

Stack-`id` ist ein stabiler Key (Default: Directory-Basename); Registry in SQLite.

### VMs

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/vms` | Runtime-Liste (+ optional Query) |
| `POST` | `/v1/vms` | Create via Worker |
| `GET` | `/v1/vms/{id}` | Inspect via Worker |
| `POST` | `/v1/vms/{id}/start` | Start |
| `POST` | `/v1/vms/{id}/stop` | Stop |
| `POST` | `/v1/vms/{id}/restart` | Restart (`stop --wait` + `start`) |
| `GET` | `/v1/vms/{id}/stats` | Guest-Agent CPU/RAM/IOPS (`vm.agent.stats`) |
| `GET` | `/v1/vms/{id}/guest-services` | Named guest publishers (`vm.agent.services.list`) |
| `*` | `/v1/vms/{id}/guest-services/{name}/api/...` | Proxy onto the published loopback API (`services.http`; SSE via `services.stream`). Includes iwatch `POST /api/restart`. |
| `DELETE` | `/v1/vms/{id}?force=` | Delete/purge |
| `PATCH` | `/v1/vms/{id}` | Modify resources |
| `GET` | `/v1/vms/{id}/mounts` | Mounts |
| `POST` | `/v1/vms/{id}/mounts` | Add mount |
| `DELETE` | `/v1/vms/{id}/mounts/{tag}` | Remove mount |

### Networks / Ports

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/nets` | Snapshot |
| `POST` | `/v1/nets` | Create |
| `DELETE` | `/v1/nets/{name}` | Delete |
| `POST` | `/v1/nets/{name}/attach` | Attach |
| `POST` | `/v1/nets/{name}/detach` | Detach |
| `GET`/`PUT` | `/v1/nets/default` | Default show/set |
| `GET` | `/v1/ports` | Port forwards |

### Images

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/images` | List |
| `POST` | `/v1/images/{alias}/pull` | Pull → Job |
| `POST` | `/v1/images/{alias}/bake` | Bake → Job; Body `{ "tag": "v1" }` Pflicht |
| `POST` | `/v1/images/{alias}/seal` | Seal → Job; Body `{ "tag": "v1" }` Pflicht |

### Docker

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/projects/{project}/containers` | `docker ps` |
| `GET` | `/v1/projects/{project}/containers/{id}` | Inspect |
| `POST` | `…/start\|stop\|restart` | Lifecycle |
| `POST` | `/v1/projects/{project}/containers` | Run |

### Host / Doctor / Certs / DNS / OIDC

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/doctor` | Doctor report |
| `POST` | `/v1/services/edge/restart` | Restart `com.vzctl.edge` LaunchAgent |
| `POST` | `/v1/certs/ca/init` | Init CA |
| `POST` | `/v1/certs/ca/install` | Install into Keychain (elevated path) |
| `GET` | `/v1/certs/fingerprint` | CA fingerprint |
| `GET` | `/v1/dns/status` | DNS health |
| `POST` | `/v1/dns/bind-helper` | Install bind helper |
| `POST` | `/v1/dns/resolver` | Install macOS `/etc/resolver` (`config` and/or `project`) |
| `DELETE` | `/v1/dns/resolver` | Uninstall resolver (query/body: `config` and/or `project`) |
| `GET`/`PUT` | `/v1/oidc/uplink` | Host OIDC uplink |
| `GET` | `/v1/oidc/status` | OIDC status |
| `PUT` | `/v1/projects/{project}/oidc/secret` | Project OIDC secret |
| `POST` | `/v1/host/reboot` | Request host reboot (osascript) |

## Compatibility

Innerhalb von v1 nur additive Änderungen. Breaking Changes brauchen `/v2`.
Clients müssen unbekannte JSON-Felder ignorieren.

Stack-/Image-/Doctor-/Docker-Ops werden in v1 über einen Supervisor-internen
`vzctl`-Worker ausgeführt (JSON-Envelope). Runtime-Ops (`vm`/`net`/`dns`/`port`)
gehen direkt an die bestehenden Supervisor-Handler.

### Interne JSON-RPC-Agent-Route

`vm.agent.ca_inject` ist eine additive lokale JSON-RPC-Route für Apply. Params:
`{vm_id,pem,fingerprint,name?}`. Der Supervisor verlangt einen laufenden Helper
und leitet als `agent.ca_inject` weiter; Erfolg liefert mindestens
`{installed:true,fingerprint,name}`. Helper-, Agent-, Installations- und
Verifikationsfehler werden als Fehler an den Aufrufer zurückgegeben.

`vm.agent.stats` ist analog: Helper leitet `agent.stats` weiter (CPU/RAM/IOPS).
