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

`mode` ist `apply`, `resume` oder `abort`.

### `apply.step`

```json
{"invocation_id":"1234-1785398400000","mode":"apply","step":"reconcile","status":"unavailable"}
```

### `apply.finished`

Reserviert für einen erfolgreichen Reconciler-Abschluss.

```json
{"invocation_id":"1234-1785398400000","mode":"apply","exit_code":0}
```

### `apply.failed`

```json
{"invocation_id":"1234-1785398400000","mode":"apply","exit_code":12,"error":"not_implemented"}
```

Der Alpha-Stub emittiert `apply.started`, `apply.step` und `apply.failed`, bevor
er wie im CLI Contract v1 mit Exit `12` endet.

## Reservierte v1-Namen

`vm.net_orphaned`, `vm.agent_ready`, `net.changed` und `dns.reloaded` sind
Schema-Platzhalter. Ihre Producer und stabilen `data`-Felder folgen in den
jeweiligen Netzwerk-/DNS-Slices.
