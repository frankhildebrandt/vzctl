# P0 Spike: Helper↔Guest-Agent E2E

**Issue:** #15  
**Stand:** #15 closed (Code/Unit-Tests). Live-Boot-Smoke Residual bis Base-Raw vorliegt.

## Ergebnis

- Jede VM-Konfiguration enthält ein `VZVirtioSocketDeviceConfiguration`.
- Der Helper verbindet ausschließlich per virtio-vsock auf Port `21950` und
  spricht u32-LE-framed JSON gemäß Protocol v1.
- `hello` liest den VM-eigenen Host-Token aus `<bundle>/agent.token`. Der
  Helper akzeptiert nur reguläre Dateien mit Mode `0600`, validiert mindestens
  256 Bit base64url und loggt weder Token noch Request-Inhalte.
- `agent-smoke` prüft `hello`, `version`, `ping`, `health`, getrenntes
  stdout/stderr bei `exec`, `report_ip`, Agent-Timeout und Agent-down. Im
  Happy Path gibt es keinen SSH-Aufruf.
- Der Helper setzt Connect-/Handshake-/Method-Deadlines. Bei einer
  Helper-Deadline sendet er best-effort `cancel`, schließt die Verbindung und
  liefert einen stabilen Timeout-Fehler.
- Der Agent implementiert argv-only `exec`, eine kleine sanitisierte Umgebung,
  256-KiB-Caps für stdin/stdout/stderr, fortgesetztes Pipe-Draining,
  Truncation, Prozessgruppen-Abbruch und maximal 600 Sekunden Laufzeit.
- `report_ip` liefert nur aktive Nicht-Loopback-Interfaces. Reservierte
  IPv4-`.0`-Adressen werden Agent- und Helper-seitig verworfen.
- Der damalige #15-Stand ließ `time_hint` Shape-only; #16 implementiert es
  inzwischen vollständig, siehe
  [`p0-agent-time-sync.md`](p0-agent-time-sync.md).

## Lokale Verifikation

```text
GOCACHE=/private/tmp/vzctl-go-cache go test ./...   PASS
cd daemon && swift test                            PASS (7 Tests)
bash -n scripts/smoke-helper-agent-e2e.sh          PASS
```

Die Tests decken Framing, Handshake, Exitcode, getrennte Streams, Output-
Truncation, Agent-Timeout, Helper-Cancel, strukturierte Remote-Fehler,
Token-Mode und `.0`-Ablehnung ab.

## Live-Boot-Smoke

Das Base-Image wird weiterhin auf einem ARM64-Linux-Builder mit
`virt-customize` erzeugt:

```bash
./scripts/build-guest-agent-base.sh
```

Das erzeugte Raw-Image auf den macOS-26-Host übertragen und ausführen:

```bash
./scripts/smoke-helper-agent-e2e.sh \
  artifacts/ubuntu-24.04-vzctl-base.raw
```

Das Script:

1. erstellt per APFS Clone-on-Write eine VM-Disk;
2. generiert einen frischen Token und speichert ihn host-seitig `0600`;
3. baut ein NoCloud-ISO mit derselben Guest-Kopie;
4. baut/signiert den Helper und bootet die VM;
5. wartet auf Agent-ready und prüft alle #15-Acceptance-Pfade;
6. stoppt den Agent mit einem argv-only Prozesssignal und verlangt danach
   einen klaren Agent-unavailable/Timeout-Fehler.

Auf dem aktuellen macOS-Host fehlt `virt-customize`; ein gebautes
`ubuntu-24.04-vzctl-base.raw` liegt nicht vor. Der Live-Smoke wurde deshalb
nicht vorgetäuscht und bleibt als ops Residual.

## Nächster Schritt

#20 (`doctor`) oder P1 #17. Live-Boot- und Sleep-Smoke nachziehen, sobald das
Base-Raw vom Builder da ist.
