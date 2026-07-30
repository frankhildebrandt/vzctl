# Image Seal Contract v1

`vzctl image seal <name|path>` macht ein bereits gebautes Linux-Base-Image
clone-safe. Der Command installiert keine Pakete und verändert den
vorinstallierten Guest-Agent nicht.

## Eingabe und Auflösung

Unterstützt werden lokale `raw`-, `qcow`- und `qcow2`-Images:

```bash
vzctl image seal artifacts/ubuntu-24.04-vzctl-base.raw
vzctl image seal ubuntu-base
vzctl image seal ubuntu-base --format json
```

Ein Pfad wird direkt verwendet. Ein Name wird lokal unter
`VZCTL_IMAGES_DIR` oder standardmäßig unter
`~/Library/Application Support/vzctl/images/` gesucht. Dabei werden der
exakte Name sowie `.raw`, `.qcow`, `.qcow2` und `.img` geprüft. Zusätzlich
werden durch [`image pull`](pull-contract-v1.md) registrierte
`aliases/<name>.json` auf content-addressed Raw-Objekte aufgelöst.

Die Offline-Anpassung benötigt auf einem Linux-Builder `qemu-img` und
`virt-customize` aus `libguestfs-tools`. Fehlt das Backend auf macOS, endet
der Command kontrolliert mit Exit `12`.

## Seal-Pipeline

Vor und nach der Bereinigung müssen diese Artefakte existieren:

- `/usr/local/sbin/vzctl-agent` und ausführbar;
- `/etc/systemd/system/vzctl-agent.service`;
- aktivierter Link unter `multi-user.target.wants`;
- `/usr/lib/vzctl-agent/image-metadata.json`.

Danach führt der Command dieselbe Bereinigung wie
[`build-guest-agent-base.sh`](../../scripts/build-guest-agent-base.sh) aus:

1. `cloud-init clean --logs --machine-id`;
2. `/etc/machine-id` leeren;
3. `/var/lib/dbus/machine-id` entfernen;
4. `/etc/ssh/ssh_host_*` entfernen;
5. `/var/lib/systemd/random-seed` entfernen;
6. alle Clone-safe- und Preservation-Invarianten erneut prüfen.

Erst nach erfolgreichen Prüfungen wird das Image read-only gesetzt. Ein
versionierter Marker
`<name>-<path-hash>.sealed.json` wird atomar im Images-Verzeichnis
geschrieben. Er enthält Quellpfad, Image-Format, `sealed=true`, Cleanup- und
Preservation-Felder. Existieren ein passender Marker und ein nicht
beschreibbares Image, ist ein erneuter Aufruf idempotent und führt keine
weitere Guest-Anpassung aus.

## CLI- und JSON-Vertrag

Human-Ausgabe ist der Default. `--format json` nutzt das
[`vzctl.dev/v1`](../specs/cli-contract-v1.md)-Envelope mit
`command=image.seal`. Die Payloads sind:

- `summary.sealed` und `summary.already_sealed`;
- `image.{name,path,format,sealed,read_only,marker}`;
- `cleanup.{machine_id,dbus_machine_id,ssh_host_keys,cloud_init,random_seed}`;
- `preserved.{agent,systemd_unit,image_metadata}`.

| Exit | Bedeutung |
|---|---|
| `0` | erfolgreich oder bereits sealed |
| `2` | Usage/Flag ungültig |
| `3` | Image fehlt, ist mehrdeutig oder hat ein nicht unterstütztes Format |
| `12` | `qemu-img`/`virt-customize` nicht verfügbar |
| `13` | Guest-Cleanup fehlgeschlagen |
| `14` | Agent-/Clone-safe-Invariante fehlgeschlagen |
| `15` | Marker/Dateirechte inkonsistent oder nicht schreibbar |

Bei JSON-Fehlern stimmen Prozess-Exitcode und `exit_code` im Envelope
überein. Diagnostics bleiben auf stderr.

## Lifecycle und Sicherheit

- Das Seal arbeitet in-place und darf nur auf einem gestoppten, bewusst als
  Base ausgewählten Image laufen.
- Die Base wird nach dem Seal nicht mehr writable geöffnet.
- `down --purge` darf Marker und Base nie löschen. Es löscht später nur
  verwaltete Linked Clones und Data-Disks gemäß
  [ADR 0003](../adr/0003-apply-state.md).
- Vor #23 soll [`vzctl doctor`](../doctor.md) bestätigen, dass das
  Images-Verzeichnis auf APFS liegt.
- #23 konsumiert Marker und Read-only-State mit `vzctl vm create`; der
  Clone-Pfad öffnet die Base nie writable. Siehe
  [`p1-linked-clone.md`](../spikes/p1-linked-clone.md).
