# CLI Contract v1

Diese Spezifikation definiert den stabilen, maschinenlesbaren Vertrag der
`vzctl`-CLI. Sie ergänzt die commandspezifische Dokumentation, insbesondere
[`docs/doctor.md`](../doctor.md), und richtet die Exitcodes an
[ADR 0003](../adr/0003-apply-state.md) aus.

## Ausgabe

- stdout enthält ausschließlich angeforderte Daten.
- Bei `--format json` ist stdout genau ein gültiges JSON-Dokument. Zusätzliche
  Statuszeilen, Fortschritt und Hinweise sind dort nicht erlaubt.
- stderr enthält Diagnostics für Menschen: Usage-Fehler, Warnungen,
  Fortschritt und Fehlerdetails. Consumers dürfen stderr nicht parsen.
- Ein Fehler darf strukturierte Daten auf stdout liefern. Maßgeblich ist immer
  `exit_code`; Prozess-Exitcode und JSON-`exit_code` müssen identisch sein.
- Human-Ausgabe bleibt der Default. Commands mit Maschinen-Consumer bieten
  `--format json`; Streaming-Commands dürfen stattdessen NDJSON spezifizieren.

## JSON-Envelope

Jedes JSON-Dokument besitzt diese stabilen Felder:

```json
{
  "apiVersion": "vzctl.dev/v1",
  "command": "version",
  "status": "ok",
  "exit_code": 0,
  "summary": {
    "message": "vzctl 0.0.1"
  },
  "version": {
    "cli": "0.0.1"
  }
}
```

| Feld | Vertrag |
|---|---|
| `apiVersion` | exakt `vzctl.dev/v1`; bestimmt die Envelope-Major-Version |
| `command` | kanonischer Command ohne Flags, z. B. `doctor` oder `vm.list` |
| `status` | `ok`, `warn` oder `fail` |
| `exit_code` | Integer, identisch zum Prozess-Exitcode |
| `summary` | kleines Objekt mit stabilen, commandspezifischen Kennzahlen/Texten |
| Payload | commandspezifische Top-Level-Felder, z. B. `checks` oder `version` |

Innerhalb von v1 dürfen optionale Felder additiv ergänzt werden. Bestehende
Felder dürfen nicht umbenannt, entfernt oder semantisch geändert werden.
Breaking Changes benötigen eine neue `apiVersion`. Feldreihenfolge ist nicht
Teil des Vertrags.

## Status und WARN/FAIL

- `ok`: Operation erfolgreich, keine relevante Warnung.
- `warn`: Operation erfolgreich, aber mit behebbaren Hinweisen. WARN führt
  grundsätzlich zu Exit `0`.
- `fail`: Operation nicht erfolgreich oder eine dokumentierte harte
  Voraussetzung ist verletzt. Der Exitcode ist ungleich `0`.
- Ein WARN darf nur dann hart fehlschlagen, wenn die commandspezifische
  Dokumentation ihn ausdrücklich als Hard-Fail klassifiziert. Dann muss der
  Envelope-Status `fail` lauten, nicht `warn`.

Für `doctor` sind nur ein nicht gesunder erreichbarer Supervisor und eine nicht
erfüllte macOS-26-Baseline Hard-Fails. Fehlende lokale Builds, Signaturen,
Resolver oder APFS-Empfehlungen bleiben WARN/Exit `0`, soweit
[`docs/doctor.md`](../doctor.md) nichts Strengeres festlegt.

## Stabile Exitcodes

| Code | Bedeutung | Quelle/Beispiel |
|---|---|---|
| `0` | Erfolg; WARN erlaubt | global, `doctor` |
| `2` | Usage oder unbekannter Command/Flag | aktuelle CLI |
| `3` | ungültige Eingabe oder Validierung | `doctor`-Optionen |
| `5` | unvollständiges Apply-Journal; Resume/Abort nötig | ADR 0003 |
| `6` | Apply-Lease wird von anderem Holder gehalten | ADR 0003 |
| `10` | Supervisor-Socket oder Health fehlerhaft | `doctor` |
| `11` | Host-Baseline macOS 26 nicht erfüllt/nicht ermittelbar | `doctor` |
| `12` | Command/Backend noch nicht verfügbar oder implementiert | Alpha-Stub |
| `13` | Image-Customization fehlgeschlagen | `image seal` |
| `14` | Image-Invariante/Preservation fehlgeschlagen | `image seal` |
| `15` | Image-Marker oder Read-only-State fehlgeschlagen | `image seal` |
| `16` | VM-Root-/Data-Disk-Vorbereitung fehlgeschlagen | `vm create` |
| `17` | Network-Operation fehlgeschlagen | Konflikt, nicht gefunden, vmnet-Rebuild |

Exitcodes werden innerhalb von v1 nicht wiederverwendet. Ein Command darf nur
Codes aus dieser Tabelle oder aus seiner commandspezifischen Erweiterung
liefern. `apply` verwendet nach Anschluss der Journal-Logik `5` und `6`; der
aktuelle syntaktisch gültige Stub liefert bis dahin `12`. Ungültige
`apply`-Flags liefern `2`, widersprüchliche `--resume --abort` liefern `3`.

## Commands v1 (erster Slice)

### `vzctl version --format json`

Payload: `version.cli`. Erfolg ist `status=ok`, Exit `0`.

### `vzctl doctor --format json`

Payload: `checks[]` mit stabilen Check-IDs. Die Summary enthält `ok`,
`warnings` und `failures`. Exitcodes: `0`, `3`, `10`, `11`.

### `vzctl apply` (Stub)

Der Stub validiert `--resume` und `--abort`, führt aber noch keine
Journal-Operation aus. Gültige Aufrufe liefern Diagnostic auf stderr und Exit
`12`. Die spätere Journal-Implementierung muss die ADR-0003-Zustände auf `5`
und `6` abbilden.

### `vzctl image seal <name|path> --format json`

Payload: `image`, `cleanup` und `preserved`; kanonischer Command ist
`image.seal`. Erfolg und idempotentes „already sealed“ liefern Exit `0`.
Usage liefert `2`, ungültiger bzw. nicht auflösbarer Input `3`, fehlende
Linux-Builder-Tools `12` und die commandspezifischen Fehler `13`–`15`.
Details stehen im [Image Seal Contract v1](../images/seal-contract-v1.md).

### `vzctl vm create <id> --from <sealed> --data-disk <GiB> --format json`

Payloads: `vm`, `image`, `disks`, `identity`, `cloud_init` und `warnings`;
kanonischer Command ist `vm.create`. Pro Bundle entstehen eine neue
cloud-init `instance-id`, eine local-admin MAC (`02:…`), Hostname/FQDN,
`cidata.iso` und ein privater Agent-Token. APFS liefert
`disks.root.clone=linked`. Nicht-APFS fällt mit `status=warn`, Exit `0` und
`clone=full` zurück. Usage liefert `2`, ungültige IDs/Größen/Formate `3`,
inkonsistenter Seal-State `15` und Fehler bei `clonefile`, Vollkopie,
Sparse-Image, NoCloud-Seed oder Manifest `16`.

Die Base wird ausschließlich read-only verwendet. `disks.root` und
`disks.data` sind die writable VZ-Attachments. Details und manueller
APFS-Space-Smoke stehen in
[`p1-linked-clone.md`](../spikes/p1-linked-clone.md).
Der Identity-Vertrag und Live-Boot-Nachweis stehen in
[`p1-identity-reset.md`](../spikes/p1-identity-reset.md).

### `vzctl net create|attach|list|detach|delete`

Kanonische Commands sind `net.create`, `net.attach`, `net.list`, `net.detach`
und `net.delete`. Alle unterstützen `--format human|json`; JSON verwendet das
v1-Envelope. `net list` liefert `networks[]` und `attachments[]`.

Ungültige CIDRs/IPs, bridged Mode und ungültige Metadaten liefern Exit `3`.
Socket-/Protokollfehler liefern `10`. Fachliche Konflikte – etwa Delete mit
Attachments, NIC-Änderung an einer laufenden VM, Duplicate-IP oder ein
fehlgeschlagener vmnet-Rebuild – liefern `17`. Labels sind wiederholbare
`--label key=value`; `--project` und `--stack` ergänzen den Desired State.
Details: [`docs/network.md`](../network.md).

Das Event-Envelope und `events subscribe` werden separat in
[#19](https://github.com/frankhildebrandt/vzctl/issues/19) spezifiziert. Events
verwenden NDJSON und sind nicht Teil des Ein-Dokument-Vertrags dieses Slices.
