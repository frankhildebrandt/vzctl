# `vzctl doctor`

`vzctl doctor` prüft den macOS-Host, ohne Ressourcen anzulegen:

```bash
vzctl doctor
vzctl doctor --format json
vzctl doctor --min-free-gib 40
```

`VZCTL_DOCTOR_MIN_FREE_GIB` setzt das Disk-Limit (Default: 20 GiB).
`VZCTL_IMAGES_DIR`, `VZCTL_HELPER_PATH`, `VZCTL_SUPERVISOR_PATH`,
`VZCTL_DNS_PORT` und `VZCTL_STATE_DIR` überschreiben die geprüften Pfade bzw.
den DNS-Port.

## Interpretation

| Status | Bedeutung |
|---|---|
| `OK` | Check bestanden |
| `WARN` | Setup fehlt, ist noch nicht aktiv oder sollte korrigiert werden; Exit bleibt 0 |
| `FAIL` | Harte Baseline verletzt |

Wichtige Hinweise:

- `vz-helper` und `vz-supervisor` sollen signiert sein und
  `com.apple.security.virtualization` enthalten. Für lokale Builds genügt
  ad-hoc Codesigning mit `daemon/VzHelper.entitlements`.
- Das Images-Verzeichnis soll auf APFS liegen; nur dann steht `clonefile` für
  Linked Clones zur Verfügung. Seal/Bake laufen lokal per `virt-customize` oder
  über die Builder-Appliance (siehe `image.backend`).
- Freier Port `127.0.0.1:15353` ist vor dem DNS-Start normal. Existiert zugleich
  eine passende `/etc/resolver/*.vz.test`, ist sie wahrscheinlich verwaist.
- vmnet wird nicht live angelegt. `doctor` bestätigt nur die macOS-26-Baseline
  und erinnert an die G0-Konvention: Host/DNS `.0`, Router `.2`, Gäste `.10+`.
- `image.backend` prüft offline, ob lokales `virt-customize`/`qemu-img` oder
  eine gecachte Builder-Appliance unter `images/builder/` verfügbar ist.
  Fehlender Cache ist WARN (First-Use-Download bzw.
  `scripts/build-builder-appliance.sh`), kein FAIL.
- Ist der Supervisor erreichbar, meldet `doctor` WARN, sobald persistierte
  vmnet-Netze nach einem Restart nicht rekonstruiert werden konnten. Nach einem
  unclean Exit kann die CIDR bis zum Host-Reboot als `orphaned` blockiert sein;
  Details stehen in [`network.md`](network.md).
- Ein nicht gestarteter Supervisor ist eine Warnung. Ein erreichbarer, aber
  defekter Socket bzw. eine schlechte DB-Health ist ein Fehler.

## Exitcodes

| Code | Bedeutung |
|---|---|
| `0` | OK oder nur Warnungen |
| `3` | Ungültige CLI-Option |
| `10` | Supervisor-Socket oder `daemon.health` fehlerhaft |
| `11` | macOS 26 wird nicht erfüllt bzw. nicht erkannt |

Bei `--format json` stehen ausschließlich strukturierte Daten auf stdout. Das
Envelope enthält `apiVersion`, `command`, `status`, `exit_code`, `summary` und
stabile Check-IDs. Der vollständige Vertrag steht in
[`docs/specs/cli-contract-v1.md`](specs/cli-contract-v1.md).
