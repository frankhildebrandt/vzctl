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
| `12` | Command/Backend noch nicht verfügbar oder implementiert | Alpha |
| `13` | Image-Customization fehlgeschlagen | `image seal` |
| `14` | Image-Invariante/Preservation fehlgeschlagen | `image seal` |
| `15` | Image-Marker oder Read-only-State fehlgeschlagen | `image seal` |
| `16` | VM-Root-/Data-Disk-Vorbereitung fehlgeschlagen | `vm create` |
| `17` | Network-Operation fehlgeschlagen | Konflikt, nicht gefunden, vmnet-Rebuild |
| `18` | Route-/Policy-Operation fehlgeschlagen | Guest-Agent oder nftables |
| `19` | macOS-Resolver-Operation fehlgeschlagen | Rechte, Kollision, unsicherer Pfad |
| `20` | DNS-Query fehlgeschlagen | Timeout, Protokollfehler oder DNS-RCODE ungleich `NOERROR` |
| `21` | Image-Netzwerk-/Metadatenfehler | Download, Release-Metadaten |
| `22` | Image-Checksum fehlgeschlagen | Upstream-/lokaler Digest-Mismatch |
| `23` | Image-Architektur unsupported | `image pull` ist ARM64-only |
| `24` | Reconciler-Step fehlgeschlagen | `up`, `apply`, `down` |

Exitcodes werden innerhalb von v1 nicht wiederverwendet. Ein Command darf nur
Codes aus dieser Tabelle oder aus seiner commandspezifischen Erweiterung
liefern. `apply` verwendet `5` und `6` für Journal/Lease. Ungültige
Reconciler-Flags liefern `2`, widersprüchliche `--resume --abort` liefern `3`.

## Commands v1 (erster Slice)

### `vzctl version --format json`

Payload: `version.cli`. Erfolg ist `status=ok`, Exit `0`.

### `vzctl doctor --format json`

Payload: `checks[]` mit stabilen Check-IDs. Die Summary enthält `ok`,
`warnings` und `failures`. Exitcodes: `0`, `3`, `10`, `11`.

### `vzctl plan|diff|up|apply|down|adopt`

```bash
vzctl plan|diff [-C <directory|config>] [--format human|json]
vzctl up [-C <directory|config>] [--force] [--format human|json]
vzctl apply [-C <directory|config>] [--force|--resume|--abort] [--format human|json]
vzctl down [-C <directory|config>] [--purge] [--format human|json]
vzctl adopt [-C <directory|config>] [--format human|json]
```

Alle Commands verwenden zuerst denselben `hypernetwork/v1`-Validator wie
`vzctl validate`. `plan` und `diff` lesen nur Desired YAML und den bekannten
Actual-State aus Supervisor-SQLite. `up` erzeugt fehlende Ressourcen und
startet gestoppte VMs, löscht aber nichts. `apply` korrigiert Drift; VM- und
Netz-Recreates sowie Deletes sind `breaking` und benötigen interaktive
Bestätigung oder `--force`. `down` stoppt in umgekehrter `dependsOn`-Reihenfolge.
`down --purge` löscht ausschließlich Ressourcen mit `managed-by=vzctl` und
passender Project-/Stack-Zuordnung. `adopt` ist in diesem Slice minimal und
meldet ohne sichere Lockfile-Orphans einen unveränderten Plan.

Das JSON-Envelope enthält `stack_id`, optional `journal` und `actions[]` mit
`action`, `kind`, `name`, `breaking` und `reason`. Ein unveränderter Plan bzw.
ein wiederholtes erfolgreiches `apply` liefert `actions=[]`, Exit `0`.

Ein incomplete Journal blockiert neue Operationen mit Exit `5`.
`apply --resume` setzt beim gespeicherten Step und derselben Desired-Generation
fort; `apply --abort` markiert die Operation als abgebrochen und gibt nur die
Lease frei. Eine aktive Lease eines anderen Holders liefert Exit `6`. Fehler
innerhalb eines Reconciler-Steps bleiben als `failed` resumierbar und liefern
Exit `24`, soweit das aufgerufene Primitive keinen spezifischeren v1-Exitcode
liefert.

### `vzctl validate`

```bash
vzctl validate [-C <directory|config>] [--format human|json]
vzctl validate --schema
```

`validate` prüft `hypernetwork/v1` zuerst gegen das aus den Serde-Typen
erzeugte JSON Schema und danach auf referentielle Integrität. Fehler stehen als
`errors[]` mit `kind`, `path` (JSONPath) und `message` im Envelope. Erfolg
liefert Exit `0`, Config-/Schema-/Referenzfehler Exit `3`, Usage Exit `2`.
`--schema` exportiert das Draft-7-Schema als reines JSON-Dokument nach stdout.
Details: [hypernetwork/v1](hypernetwork-v1.md).

### `vzctl image pull <alias> --format json`

Payloads: `image` und `source`; kanonischer Command ist `image.pull`.
`summary.change` ist `pulled` oder beim idempotenten Re-Pull `unchanged`.
`image` enthält Alias/Kanonik, Release, `architecture=arm64`, den
aufgelösten Raw-Pfad, SHA256, Manifest und den Seal-State. Ein frischer Pull
liefert `sealed=false`; ein unveränderter Re-Pull darf nach separatem Seal
`sealed=true` liefern. `source` enthält URL, Eingabeformat und verifizierten
Upstream-Digest.

Erfolg liefert Exit `0`, Usage `2`, unbekannte Aliase `3`, fehlende lokale
Konvertierungswerkzeuge `12`, Store-/Normalisierungsfehler `15`,
Netzwerk-/Metadatenfehler `21`, Checksumfehler `22` und unsupported Arch `23`.
Details: [Image Pull Contract v1](../images/pull-contract-v1.md).

### `vzctl image seal <name|path> --format json`

Payload: `image`, `cleanup` und `preserved`; kanonischer Command ist
`image.seal`. Erfolg und idempotentes „already sealed“ liefern Exit `0`.
Usage liefert `2`, ungültiger bzw. nicht auflösbarer Input `3`, fehlende
Linux-Builder-Tools `12` und die commandspezifischen Fehler `13`–`15`.
Details stehen im [Image Seal Contract v1](../images/seal-contract-v1.md).

### `vzctl vm create <id> --from <sealed> --data-disk <GiB> [--network <name>] --format json`

Payloads: `vm`, `network`, `image`, `disks`, `identity`, `cloud_init` und `warnings`;
kanonischer Command ist `vm.create`. Pro Bundle entstehen eine neue
cloud-init `instance-id`, eine local-admin MAC (`02:…`), Hostname/FQDN,
`cidata.iso` und ein privater Agent-Token. Ohne `--network` wird das
konfigurierte Default-Netz verwendet; `network` enthält Name, CIDR, IP,
Prefix, Gateway/DNS `.0` und `automatic=true`. `--network` oder ein bestehendes
explizites Attachment gewinnt. APFS liefert
`disks.root.clone=linked`. Nicht-APFS fällt mit `status=warn`, Exit `0` und
`clone=full` zurück. Usage liefert `2`, ungültige IDs/Größen/Formate `3`,
inkonsistenter Seal-State `15` und Fehler bei `clonefile`, Vollkopie,
Sparse-Image, NoCloud-Seed oder Manifest `16`. Fehlende Default-Konfiguration,
unbekannte Netze und IP-/Attachment-Konflikte liefern `17`.

Die Base wird ausschließlich read-only verwendet. `disks.root` und
`disks.data` sind die writable VZ-Attachments. Details und manueller
APFS-Space-Smoke stehen in
[`p1-linked-clone.md`](../spikes/p1-linked-clone.md).
Der Identity-Vertrag und Live-Boot-Nachweis stehen in
[`p1-identity-reset.md`](../spikes/p1-identity-reset.md).

### `vzctl net create|attach|list|detach|delete|default`

Kanonische Commands sind `net.create`, `net.attach`, `net.list`, `net.detach`
und `net.delete`. Alle unterstützen `--format human|json`; JSON verwendet das
v1-Envelope. `net list` liefert `networks[]` und `attachments[]`.

Ungültige CIDRs/IPs, bridged Mode und ungültige Metadaten liefern Exit `3`.
Socket-/Protokollfehler liefern `10`. Fachliche Konflikte – etwa Delete mit
Attachments, NIC-Änderung an einer laufenden VM, Duplicate-IP oder ein
fehlgeschlagener vmnet-Rebuild – liefern `17`. Labels sind wiederholbare
`--label key=value`; `--project` und `--stack` ergänzen den Desired State.
Details: [`docs/network.md`](../network.md).

```bash
vzctl net default show [--format human|json]
vzctl net default set <name> --cidr <CIDR> [--format human|json]
```

Kanonische Commands sind `net.default.show` und `net.default.set`. Das
JSON-Envelope enthält `default_network`; ohne Konfiguration ist der Wert
`null`. Bei Konfiguration enthält er `mode=shared`, `access=full`,
`nat_egress=true` und den Zustand der zugehörigen Network-Row.

### `vzctl route apply|plan|status`

```bash
vzctl route apply|plan [--config <path>] [--router <vm-id>] [--format human|json]
vzctl route status [--router <vm-id>] [--format human|json]
```

`route apply` und `route plan` lesen `spec.policies` aus der angegebenen
Environment-Datei beziehungsweise aus `./hypernetwork.config.yaml`.
Sie validieren Router-Rolle, mindestens zwei Attachments, `.2` je Netz,
`forward: deny-all`, Zielnetze, Protokolle und Ports. `plan` verändert den Gast
nicht. `status` liest den aktiven nftables-Status über Helper und Guest-Agent.

Das v1-Ergebnis enthält `routers[]`, `summary.changed` und pro Router
`active`, `forward_policy`, `policies[]`, `rules[]` und `policy_changes[]`.
Exit `18` steht für Route-/Guest-Apply-/Statusfehler, Exit `3` für ungültige
Konfiguration, Rollen oder Topologien.

### `vzctl dns query|install-resolver|uninstall-resolver`

```bash
vzctl dns query <name> \
  [--type A|AAAA] [--server <IP:port>] [--format human|json]
```

`dns.query` sendet ein UDP-DNS-Paket direkt an den angegebenen Server. Der
Default ist `127.0.0.1:15353`; der Command hängt nicht von `/etc/resolver` oder
der libc-Namensauflösung ab. Der Default-Typ ist `A`, zusätzlich wird `AAAA`
unterstützt.

Das JSON-Envelope enthält `query`, `rcode`, `rcode_code`, `authoritative`,
`truncated` und `answers[]`. Jeder Answer enthält `name`, `type`, `class`,
`ttl` und `data`. `NOERROR` liefert Exit `0`, auch bei leerer Answer-Liste.
Timeouts, ungültige/truncated Antworten und RCODES ungleich `NOERROR` liefern
Exit `20`; soweit eine DNS-Antwort vorliegt, bleiben deren RCODE und Answers im
Fail-Envelope erhalten. Usage liefert `2`, ungültige Namen, Typen und
Server-Endpunkte liefern `3`.

Der v1-Client ist UDP-only. Eine Antwort mit gesetztem `TC`-Bit wird deshalb
als Exit `20` gemeldet; TCP-Fallback ist nicht Teil dieses Slices.

```bash
vzctl dns install-resolver|uninstall-resolver \
  [--project <name>] [--config <path>] [--format human|json]
```

Die kanonischen Commands heißen `dns.install-resolver` und
`dns.uninstall-resolver`. Ohne `--project` wird `spec.project` aus
`hypernetwork.config.yaml` gelesen. Das JSON-Envelope enthält `resolver` mit
`project`, `domain`, `path`, `nameserver`, `port` und `managed`;
`summary.change` ist `installed`, `updated`, `unchanged`, `removed` oder
`absent`.

Ungültige Projekte/Configs liefern Exit `3`. Fehlende Rechte, fremde Dateien,
Symlinks und Projekt-/Config-Kollisionen liefern Exit `19`. Idempotente
No-op-Installationen und -Deinstallationen liefern Exit `0`.

Das Event-Envelope und `events subscribe` werden separat in
[#19](https://github.com/frankhildebrandt/vzctl/issues/19) spezifiziert. Events
verwenden NDJSON und sind nicht Teil des Ein-Dokument-Vertrags dieses Slices.
