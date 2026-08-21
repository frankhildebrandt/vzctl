# Event Stream v1

Diese Spezifikation definiert den versionierten Event-Stream von `vzctl`.
Sie ergänzt den [CLI Contract v1](cli-contract-v1.md), verwendet aber wegen
des offenen Streams NDJSON statt eines einzelnen JSON-Dokuments.

## Transport und Ausgabe

```bash
vzctl events subscribe [--filter 'vm.*,apply.*']
```

- Die CLI verbindet sich mit dem Supervisor-Socket `vz.sock`.
- stdout enthält ausschließlich Events als NDJSON: exakt ein JSON-Objekt und
  ein Zeilenumbruch pro Event.
- stderr enthält nur Diagnostics und ist kein Maschinenvertrag.
- Es gibt in v1 kein Replay. Der Stream enthält nur Events, die nach der
  erfolgreichen Subscription emittiert werden.
- `Ctrl-C` schließt die Subscription sauber und liefert Exit `0`.
- Ein nicht erreichbarer oder abgebrochener Supervisor-Stream liefert Exit
  `10`. Ungültige Filter liefern Exit `3`, unbekannte Optionen Exit `2`.

## Envelope

Jedes Event besitzt diese Pflichtfelder:

```json
{"v":1,"ts":"2026-07-30T08:30:00.000Z","type":"vm.state","data":{"vm_id":"web","state":"running"}}
```

| Feld | Vertrag |
|---|---|
| `v` | Integer `1`; Major-Version des Event-Envelopes |
| `ts` | RFC3339-Zeitpunkt in UTC |
| `type` | stabiler Event-Name |
| `data` | typspezifisches JSON-Objekt |

Feldreihenfolge ist nicht Teil des Vertrags.

## Compatibility

Innerhalb von v1 dürfen Envelope und `data` nur um optionale Felder erweitert
werden. Bestehende Felder dürfen nicht entfernt, umbenannt oder semantisch
geändert werden. Neue Event-Typen dürfen additiv hinzukommen. Breaking Changes
benötigen eine neue Envelope-Version `v`.

Consumer müssen unbekannte Felder und unbekannte Event-Typen ignorieren.

## Filter

`--filter` ist eine kommaseparierte ODER-Liste. Leerzeichen um Einträge werden
ignoriert:

- `vm.state` trifft exakt `vm.state`.
- `vm.*` trifft jeden Typ mit Präfix `vm.`.
- `vm.*,apply.*` trifft beide Präfix-Gruppen.
- `*` trifft alle Event-Typen und entspricht einem fehlenden Filter.
- `*` ist nur als letztes Zeichen eines Eintrags zulässig. Leere Einträge und
  Muster wie `vm.*.failed` sind ungültig.

Der Supervisor filtert vor der Übertragung. Die Reihenfolge der durchgelassenen
Events bleibt erhalten.

## Event-Typen v1

### `vm.state`

Wird bei `helper.hello` und jeder `helper.state`-Änderung emittiert.

```json
{"vm_id":"web","state":"running","pid":1234,"bundle":"/path/to/web.bundle"}
```

`state` ist `starting`, `running`, `stopped` oder `failed`.

### `vm.clock_corrected`

Wird nach einer vom Guest-Agent bestätigten Uhrkorrektur emittiert.

```json
{"vm_id":"web","reason":"wake","observed_guest_unix_ms":1785398400000,"offset_ms":2400,"action":"stepped"}
```

### `apply.started`

```json
{"invocation_id":"1234-1785398400000","mode":"apply"}
```

`mode` ist `up`, `apply`, `down`, `resume` oder `abort`.

### `apply.step`

```json
{"invocation_id":"7e14728c","step":"ensure_nets","status":"running","error":null}
```

`status` ist `running`, `done` oder `failed`. Die Step-Reihenfolge folgt ADR
0003; ein Resume beginnt beim zuletzt gespeicherten `failed`/`running`-Step.
`await_cloud_init` folgt auf `await_agents` und ist erst `done`, wenn alle neu
erstellten oder ersetzten VMs Cloud-init ohne Exit `1`/`2` beendet haben.

### `apply.finished`

Wird nach Commit des Actual-State und Freigabe der Lease emittiert.

```json
{"invocation_id":"7e14728c","mode":"apply","stack_id":"project:stack","exit_code":0}
```

### `apply.failed`

```json
{"invocation_id":"7e14728c","mode":"apply","step":"ensure_vms","exit_code":24,"error":"helper failed"}
```

### `dns.reloaded`

```json
{"reason":"net.attach","ok":true,"records":3,"zones":1,"listeners":["10.80.0.0:53","127.0.0.1:15353"],"ttl":15,"upstream":"system","error":null}
```

`reason` benennt den Registry-Auslöser oder `startup`. `records` zählt
A-Adressen, nicht nur Namen.

### `dns.reload_failed`

Gleiches Schema wie `dns.reloaded`, aber `ok=false` und `error` enthält die
Snapshot- oder Bind-Ursache. Bereits erfolgreiche Listener bleiben aktiv.

### `vm.systemd.unit`

Emitted when a guest systemd unit changes state. Requires an upgraded guest
agent with capability `systemd` and an active `events.subscribe` filter such as
`vm.systemd.*`.

```json
{
  "vm_id": "lab/web",
  "unit": "nginx.service",
  "unit_type": "service",
  "load": "loaded",
  "active": "active",
  "sub": "running",
  "reason": "properties_changed"
}
```

### Host-Network-Recovery

Alle Events tragen `epoch`. Sensible Pfaddaten wie SSID, öffentliche IP und
Probe-URL werden nie aufgenommen.

| Event | Zusätzliche Daten |
|---|---|
| `host.network_changed` | `path_satisfied`, grobe `interfaces` (`wifi`, `ethernet`, `other`) |
| `host.sleep` | keine |
| `host.wake` | keine |
| `network.recovering` | `attempt` |
| `network.recovered` | `attempt`, optional `fallback` |
| `network.degraded` | `state`, optional sanitisiertes `error` |
| `network.cidr_conflict` | `conflicts[]` mit Netz-CIDR, Hostroute und Interface |
| `network.fallback_restart` | `network`, `vms[]` |

Ein Event ist Diagnose, kein Config-Write. Insbesondere führt
`network.cidr_conflict` niemals zu einer automatischen CIDR-Änderung.

## Reservierte v1-Namen

`vm.net_orphaned`, `vm.agent_ready` und `net.changed` bleiben
Schema-Platzhalter. Ihre Producer und stabilen `data`-Felder folgen in den
jeweiligen Slices.
