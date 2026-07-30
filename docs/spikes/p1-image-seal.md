# P1 Spike: `vzctl image seal`

- **Issue:** #22 unter Epic #21
- **Stand:** Code- und Offline-Smoke abgeschlossen
- **Builder:** Linux mit `qemu-img` und `libguestfs-tools` **oder** macOS mit
  gepinnter Builder-Appliance (`scripts/build-builder-appliance.sh`, Decision 25)

## Ergebnis

Der CLI-Slice erkennt `raw`/`qcow`, prüft den eingebetteten Agent samt
systemd Unit und Metadata, bereinigt Clone-Identität und setzt die Base erst
nach erfolgreicher Nachprüfung read-only. Ein versionierter Marker unter
`Application Support/vzctl/images/` macht den Vorgang idempotent.

Die macOS-Teststrecke mockt das externe Image-Backend. Der reale
Builder-Smoke ist:

```bash
./scripts/build-guest-agent-base.sh
cargo run -p vzctl -- image seal \
  artifacts/ubuntu-24.04-vzctl-base.raw \
  --format json
./scripts/smoke-guest-agent-base.sh \
  artifacts/ubuntu-24.04-vzctl-base.raw
```

Der letzte Smoke liest das Image nur. Schreibzugriffe sind nach dem Seal
nicht erlaubt.

## Acceptance #22

- [x] Seal-Pipeline erzeugt eine clone-safe Base.
- [x] Agent, aktivierte Unit und `image-metadata.json` werden vor und nach
  dem Cleanup geprüft.
- [x] machine-id, dbus machine-id, SSH Host Keys, cloud-init state und
  random seed werden bereinigt.
- [x] Human- und JSON-Ausgabe folgen CLI Contract v1.
- [x] Marker + Read-only-Modus machen den Command idempotent.
- [x] Offline-Tests decken Auflösung, Pipeline, Marker und Golden-JSON ab.
- [ ] Reales großes Ubuntu-Image: Appliance cachen, dann
  `vzctl image bake` + `vzctl image seal` auf macOS (Ops-Smoke)

## Folge

Epic #21 ist nach diesem Slice zu einem Drittel umgesetzt. Als Nächstes folgt
#23: APFS-`clonefile` der sealed Base plus leere Data-Disk. #24 übernimmt
danach die per-Clone Identity.

