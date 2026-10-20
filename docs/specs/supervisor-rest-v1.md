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
| `POST` | `/v1/jobs/{jobId}/cancel` | Abbruch anfordern (`202`); unbekannter Job `404`, bereits beendet `409` |

Jobs sind flüchtig und auf die letzten 32 Einträge begrenzt; aktive Jobs werden nie
verdrängt. Höchstens vier Jobs laufen gleichzeitig, weitere Starts erhalten `429`.
Synchron gestartete CLI-Aufrufe haben ein Zeitlimit von 60 Sekunden, Hintergrundjobs
von zwei Stunden. Timeout und Abbruch enden mit `failed` und einer eindeutigen Meldung.
Der CLI-Prozess wird beendet; bereits angewendete VM-/Netzwerk-Änderungen werden nicht
zurückgerollt. Ein unvollständiger Apply bleibt über sein Journal mit `--resume` oder
`--abort` behandelbar. Beide Ausgabekanäle werden während der Laufzeit geleert.
Stdout ist auf 1 MiB begrenzt (Überschreitung beendet den Job als Fehler), stderr auf
die letzten 256 KiB mit Kürzungshinweis. Das Live-Log hält 500 Zeilen zu maximal
4 KiB; überlange Zeilen werden gekürzt.

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
| `GET` | `/v1/vms/{id}/systemd` | Guest systemd capability (`vm.agent.systemd.status`) |
| `GET` | `/v1/vms/{id}/systemd/units?type=&all=` | List units (`vm.agent.systemd.list`) |
| `GET` | `/v1/vms/{id}/systemd/units/{unit}` | Show unit (`vm.agent.systemd.show`) |
| `POST` | `/v1/vms/{id}/systemd/units/{unit}/start\|stop\|restart` | Control unit |

### Networks / Ports

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/nets` | Snapshot |
| `POST` | `/v1/nets` | Create |
| `DELETE` | `/v1/nets/{id}` | Delete by network ID |
| `POST` | `/v1/nets/{id}/attach` | Attach by network ID |
| `POST` | `/v1/nets/{id}/detach` | Detach by network ID |
| `GET`/`PUT` | `/v1/nets/default` | Default show/set |
| `GET` | `/v1/ports` | Port forwards |

Netzwerk-Snapshots enthalten additiv `networks[].id` und
`attachments[].network_id`. `name`/`network` bleiben die kurzen Namen;
`project` beschreibt den Projektkontext. Clients verwenden die ID für
Zuordnung und Aktionen und zeigen das Projekt im Löschdialog an. Eine ID mit
Slash wird vollständig als einzelnes Pfadsegment kodiert, beispielsweise
`/v1/nets/controller-lab%2Flan`; der Server trennt Segmente vor dem Dekodieren.
Bei einem Netz mit `project` sendet DELETE zusätzlich
`?project=controller-lab`, damit auch bestehende kurze Runtime-IDs eindeutig
dem Eigentümer zugeordnet werden.
`GET /v1/nets/default` ergänzt die aufgelöste ID. Alte Antworten ohne ID dürfen
nur bei eindeutigen Namen aufgelöst werden; ein globales Default-Netz darf
kein gleichnamiges Projektnetz markieren. Bei Mehrdeutigkeit bleiben
destruktive Aktionen gesperrt.

### Images

| Method | Path | Beschreibung |
|---|---|---|
| `GET` | `/v1/images` | List |
| `POST` | `/v1/images/{alias}/pull` | Pull → Job |
| `POST` | `/v1/images/{alias}/bake` | Bake → Job; Body `{ "tag": "v1", "force"?: true }` |
| `POST` | `/v1/images/{alias}/seal` | Seal → Job; Body `{ "tag": "v1", "force"?: true }` |
| `POST` | `/v1/images/{alias}/rebake` | Rebake+Seal → Job; Body `{ "tag": "v1" }` |

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

## Status-Bundle, Debug und Jobs (2026-10)

`GET /v1/stacks/{id}/status` enthält neben dem Prozessinventar `sections.health`:
das unveränderte `stack.status`-Envelope aus der CLI. `summary.health` ist
`ok|degraded|critical`; ein Nichtnull-Exitcode löscht die Diagnosedaten nicht.
Prozessphase `running` und Health `degraded` sind gleichzeitig möglich. Eine
fehlgeschlagene Erhebung ist `unknown`, niemals implizit gesund. Nicht
konfiguriertes OIDC wird als `enabled:false,status:disabled` dargestellt.

`sections.apply` ergänzt den gespeicherten unvollständigen CLI-/UI-Vorgang aus
dem vorhandenen `stack.inspect`-Datenmodell. Die Zuordnung benutzt die kanonische
`diff.stack_id`, nicht die ID des UI-Verzeichniseintrags. Bei erfolgreicher
Erhebung enthält `data` die `stack_id`, `journal` und `lease`; `journal:null`
bedeutet keinen unvollständigen Vorgang. Ein Journal enthält ausschließlich
`id`, `generation`, `status`, `step`, `mode`, `error` (maximal 4096 Zeichen) und
`updated_at`; Payload und Ressourcen werden nicht veröffentlicht. Die Lease
enthält `holder` und `expires_at`. Fehlende Identität oder Erhebungsfehler liefern
`ok:false,data:null`. Die Betriebsseite zeigt Ursache und Schritt auch nach
Neustart und verwendet den bestehenden expliziten Resume-/Abort-Pfad.
Aktive oder unlesbare Leases sperren diese UI-Aktionen; der Server prüft bei
jeder Mutation erneut. Das Lesen löst weder Resume noch Abort aus.

`observed_at` ist der Erhebungszeitpunkt. Karten-/Detailabfragen teilen einen
maximal 15 Sekunden gültigen Snapshot; YAML-Änderungen und REST-Mutationen
invalidieren ihn. Fehlgeschlagene Collections werden nicht gecacht. Das ersetzt
keine Events und ist keine Zusage einer verzögerungsfreien Zustandsanzeige.

Debug-Quellen: `GET /v1/stacks/{id}/debug/sources` (optionaler Query `type`);
`/debug/sources/list` bleibt kompatibler Alias. Doctor: `GET .../debug/doctor`.
SQL: `POST .../debug/postgres/query` bzw. `mysql/query` mit `sql` und optional
`source`, `maxRows`, `timeoutMs`. Unbekannte Routen liefern 404.

Jobs: `POST /v1/jobs/{id}/cancel` fordert CLI-Abbruch an. Bereits angewendete
Ressourcen werden nicht zurückgerollt; ein Apply-Journal bleibt für Recovery
sichtbar. Höchstens vier aktive Jobs; Überlast liefert HTTP 429. Bis zu 32 Jobs
werden im RAM aufbewahrt. Nach Supervisor-Neustart ist der Job-Speicher leer;
Apply-Journale in SQLite bleiben erhalten. stdout ist auf 1 MiB, stderr-Tail auf
256 KiB, einzelne Logzeilen auf 4 KiB begrenzt. Grenzen führen zu expliziten
Fehlern bzw. gekennzeichneter Kürzung. Sync-CLI-Aufrufe haben 60 Sekunden,
Hintergrundjobs zwei Stunden Zeitlimit.
