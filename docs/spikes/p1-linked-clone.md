# P1 Spike: APFS Linked Clone + dataDisk

- **Issue:** #23 unter Epic #21
- **Voraussetzung:** #22 `vzctl image seal`
- **Stand:** Code, Unit-Tests und nativer APFS-Smoke abgeschlossen

## Ergebnis

```bash
vzctl vm create web \
  --from ubuntu-base \
  --data-disk 64 \
  --format json
```

Der Command löst die Base über denselben Images-Pfad und
`*.sealed.json`-Marker wie `image seal` auf. Er akzeptiert für den
VZ-Start nur `raw`, prüft Marker plus read-only Dateimodus erneut und erzeugt:

```text
Application Support/vzctl/vms/web/
├── disk.raw       # APFS clonefile(2), writable Clone
├── dataDisk.raw   # leeres Sparse-Image
└── vm.json        # managed-by=vzctl, Base/Disks/Clone-Modus
```

Die Base wird im Clone-Pfad nie writable geöffnet. `clonefile(2)` erhält nur
Quell- und Zielpfad; nur die neue Root-Disk wird danach auf Modus `0600`
gesetzt. Der Helper erkennt `dataDisk.raw` im Bundle automatisch oder
akzeptiert `--data-disk <raw>` und hängt Root plus Data writable an VZ.
`cidata.iso` bleibt read-only.

## APFS und Fallback

Auf APFS ist `clone=linked`. Ein erfolgreicher `clonefile(2)`-Aufruf ist die
maßgebliche COW-Garantie: die Dateien referenzieren zunächst dieselben
Extents; ein Write auf einen Clone allokiert abweichende Blöcke. Der native
macOS-Test erzeugt zwei Clones einer 4-MiB-Base, schreibt in den ersten und
verifiziert, dass Base und zweiter Clone bytegleich bleiben.

Auf Nicht-APFS bzw. unbekanntem Dateisystem wird bewusst `clone=full`
verwendet. Human-Ausgabe schreibt ein `WARN` auf stderr; JSON liefert
`status=warn`, Exit `0` und `warnings[]`. Der Full-Copy-Code öffnet die Base
nur per `File::open` (read-only).

Für einen manuellen großen APFS-Space-Smoke:

```bash
df -k "$VZCTL_STATE_DIR"
vzctl vm create smoke-a --from ubuntu-base --data-disk 1
vzctl vm create smoke-b --from ubuntu-base --data-disk 1
du -kh "$VZCTL_STATE_DIR"/vms/smoke-{a,b}/disk.raw
df -k "$VZCTL_STATE_DIR"
dd if=/dev/zero of="$VZCTL_STATE_DIR/vms/smoke-a/disk.raw" \
  bs=1m count=256 conv=notrunc
df -k "$VZCTL_STATE_DIR"
```

`du` zeigt die logische Belegung je Datei nicht zuverlässig dedupliziert;
entscheidend sind erfolgreicher `clonefile(2)` und die zusätzliche
APFS-Containerbelegung erst nach Write-Divergenz.

## Fehler- und Lifecycle-Vertrag

- fehlender/inkonsistenter Seal-Marker oder writable Base: Exit `15`;
- `clonefile`, Vollkopie, Sparse-Erzeugung oder Manifest fehlgeschlagen:
  Exit `16`;
- bei APFS-`clonefile`-Fehler gibt es keinen stillen Full-Copy-Fallback;
- ein teilweise neu erzeugtes Bundle wird entfernt;
- `down` behält Clone und Data; `down --purge` darf nur `managed-by=vzctl`
  Clone plus Data löschen, niemals Base oder Seal-Marker.

## Acceptance #23

- [x] Zwei native APFS-Clones starten als COW und divergieren bei Writes.
- [x] Base-Datei wird nie writable geöffnet.
- [x] Injizierter `clonefile`-Fehler liefert Exit `16` und räumt partiell auf.
- [x] Human/JSON folgen CLI Contract v1; Nicht-APFS ist WARN/Exit `0`.
- [x] Helper hängt Root + dataDisk an VZ.

## Folge

Mit dem nachfolgenden, inzwischen umgesetzten #24 ist Epic #21 vollständig:
Identity-Reset für MAC, machine-id, Hostname, SSH Host Keys und cloud-init
instance-id. Details:
[`p1-identity-reset.md`](p1-identity-reset.md).
