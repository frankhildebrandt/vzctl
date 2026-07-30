# P0 VM-Helper Lifecycle (#10)

Stand: 2026-07-30

## Ergebnis

- `vz-helper run` hält genau eine `VZVirtualMachine`.
- EFI, writable raw boot disk, optionales cidata ISO, NAT und Serial-Log.
- `SIGTERM` versucht zuerst den Guest-Stop und erzwingt nach fünf Sekunden den
  VZ-Stop.
- Ein `flock` pro VM-ID verhindert Doppelstarts; ein Crash gibt den Lock im
  Kernel frei und der nächste Helper überschreibt die stale PID.
- `helper.hello`/`helper.state` melden den Zustand an den Supervisor. Ein
  UDS-Ausfall führt zu Retry statt Prozessabbruch.
- Per-VM-launchd-Plists haben begrenztes Crash-Restart-Verhalten und schreiben
  nach `~/Library/Logs/vzctl/`.

## Manuelle Acceptance

Auf macOS 26 mit den bestehenden G0-Ubuntu-Assets ausgeführt:

```sh
cd daemon
swift test
scripts/codesign-helper.sh
scripts/smoke-helper-rpc.sh
scripts/smoke-helper-isolation.sh
scripts/smoke-helper-launchd.sh
scripts/smoke-helper-two-vms.sh \
  ../spikes/g0/assets/frontend.raw \
  ../spikes/g0/assets/cidata-fe.iso
```

Gemessen:

- Zwei echte Ubuntu-VMs starteten headless und schrieben `Ubuntu 24.04.4 LTS`
  auf ihre getrennten Serial-Logs.
- `kill -9` von Helper A ließ Helper B und dessen VM laufen.
- Helper B stoppte über `SIGTERM`.
- launchd `bootstrap` und `bootout` liefen erfolgreich.
- `vm.list` zeigte den vom Helper gemeldeten Zustand `running`.

## Reconnect (#11) — verifiziert

`scripts/smoke-helper-reconnect.sh`:

1. Supervisor `serve` + Helper `--mock`
2. `vm.list` enthält die VM
3. Supervisor `kill -9` → Helper-PID bleibt
4. Stale `vz.sock` entfernen, neuer `serve`
5. Innerhalb der Heartbeats (5s) erneut `helper.hello` → `vm.list` wieder `running`

Helper-Records sind **in-memory** (Alpha): nach Restart nur via Re-Hello sichtbar.
SQLite-Persistenz der Sightings ist optional später.

### Alpha-Fenster (DNS / vmnet)

Während der Supervisor tot ist:

| Ressource | Verhalten |
|---|---|
| Helper / `VZVirtualMachine` | läuft weiter |
| UDS / `vm.list` | down |
| DNS (Dual-Listener, später #26) | down bis Supervisor-Restart |
| vmnet refs (später #31) | orphaned bis Restore; G0: Ref-Release Pflicht |

`vzctl doctor`: Socket fehlt → WARN (kein Hard-Fail). Siehe ADR 0002.

## Grenze dieses Slices

Der erste Boot nutzt `VZNATNetworkDeviceAttachment`. Supervisor-owned
vmnet-Attachment-Handles und vsock-Agent bleiben Folge-Issues (#31, #12).
