# P0 Spike: Guest-Agent im Ubuntu Base

**Issue:** #14<br>
**Stand:** Boot-proof implementiert; Helper↔Agent E2E bleibt #15.

## Ergebnis

- `vzctl-agent` baut statisch für Linux ARM64 und bindet AF_VSOCK Port `21950`.
- Protocol-v1-Minimum ist getestet: Framing, `hello`/Token, `ping`, `version`
  und `health`.
- `exec`, `report_ip` und `time_hint` antworten stabil mit `unsupported` und
  werden nicht als Capability beworben.
- systemd startet den Agent als dedizierten unprivilegierten User nach
  `cloud-final.service`, sobald das NoCloud-Token mit Mode `0600` vorliegt.
- Die Offline-Pipeline installiert Binary und Unit vor dem Seal, schreibt
  Versionsmetadaten und entfernt clone-spezifische Identität.
- Das NoCloud-Beispiel enthält keinen Download, kein Paket und keinen
  Installationsbefehl.

## Verifikation

Lokal ausgeführt:

```text
go test ./...                                      PASS
CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build     PASS
file vzctl-agent                                   ARM aarch64, statically linked
bash -n scripts/build-guest-agent-base.sh          PASS
bash -n scripts/smoke-guest-agent-base.sh          PASS
```

Der vollständige Image-Build benötigt `virt-customize` auf einem ARM64-Linux-
Builder. Das aktuelle macOS-Setup stellt dieses Tool nicht bereit; deshalb ist
der dokumentierte frische Clone-/systemd-/Listener-Proof nach dem Image-Build
auszuführen. Ein Host-Helper-vsock-Client oder Production-E2E wurde bewusst
nicht vorgezogen.

## Grenzen / Follow-ups

- #15 implementiert Helper-Handshake, `exec`, `report_ip`, Timeouts und den
  echten Host↔Guest-Test.
- #16 ergänzt `time_hint`/Clock-Handling.
- #22 übernimmt die Seal-Checks, Immutable-Markierung und Clone-Mechanik; der
  Agent muss dabei installiert bleiben.
