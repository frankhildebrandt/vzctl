# Abschlussziel: Merge, Release und lokale Installation

Stand:20.10.2026. Der Nutzer hat diese Schritte ausdrücklich beauftragt.
**Erledigt: PR #85 gemergt, v0.3.0-alpha4 veröffentlicht und lokal abgenommen.**
Canvas:65/66 abgeschlossen,0 aktiv in Arbeit. Ein externer Installer-Nachweis offen.

## Auslieferung

- [PR #85](https://github.com/frankhildebrandt/vzctl/pull/85) auf main gemergt,
  Commit56634842fd202a6fbc8339feb5a5ff6a01dd1f22.
- Release-Commit d050883f0d3636593718797a75f39071acd9f4bc enthält anschließend
  ausschließlich die aktuelle README. Vollständige CI auch auf genau diesem
  Commit [grün](https://github.com/frankhildebrandt/vzctl/actions/runs/39488109999):
  Rust496 bestanden/2 bestehende Ignores, Swift213, UI172, Paket7.
- [v0.3.0-alpha4](https://github.com/frankhildebrandt/vzctl/releases/tag/v0.3.0-alpha4)
  mit TAR/PKG/DMG und drei Prüfsummendateien veröffentlicht. Alle6 GitHub-Digests,
  Dateigrößen und Tag-Ziel verifiziert. Paketmetadaten:dirtyfalse, exakt d050883.
- Alle8 eigenen Versionsquellen/Programme und das App-Bundle konsistent.
  [Paket- und Installationsbericht](release-0.3.0-alpha4.md).

## Lokale Abnahme

- Vorhandene Nutzerinstallation unter ~/.local/bin und ~/Applications aktualisiert.
  Programme, Root-DNS-Helfer und sämtliche App-Dateien entsprechen dem Paketbuild.
- SQLite/App/Binaries/Configs gesichert. Neti/Lab wieder gestartet;6 VMs laufen,
  IQ-Store3 bleibt bewusst gestoppt. Net/Edge/CP gestartet, vz-net nur graceful.
- Neti/Lab healthok, warnings0; IQ-Store stopped/checksskipped/warnings0.
  Neun Ready-Claims plus bewusstes foreign Retained, zwei K3s-Nodes Ready.
- VM/Docker/K3s bestehen PostgreSQL/S3/OIDC; HTTPS200 mit striktem TLS.
  Vorhandene PostgreSQL-Zeile, S3-Objekt, neun Bindings und Seeds unverändert.
- Native UI geöffnet und Neti/Lab gesund sowie IQ-Store neutral geprüft.

Die vorherige technische Abnahme bleibt in [09](09-live-update-acceptance.md)
und den dort verknüpften Receipts erhalten. Keine Full/Repeat/Late/Fault/
Retentionserie wiederholt. Der normale Update-Neustart war beauftragt;
keine künstlichen Calls oder kosmetischen Neustarts wegen früherer P99-Warnungen.

## Konkrete aufgelöste Auslieferungshindernisse

CI-Socket-Testserver blockierten im kooperativen Swift-Workerpool. Mit nur
einem Worker lokal reproduziert und per Prozess-Stack belegt; blockierendes
Socket-I/O der zwei Testfixtures liegt jetzt auf DispatchQueue. CI prüft den
Ein-Worker-Fall. Ein weiterer Test wartete100ms, wurde unter Last jedoch erst
nach16,6s fortgesetzt und zählte den regulären10s-Passivretry mit. Ereignis-
abschluss statt Zeitannahme, spätere Epochen im fokussierten Test deaktiviert;
alle Assertions erhalten. Kein Produktcode für diese Testfehler geändert.

Beim Update wartete der alte Edge auf ein beendetes Caddy-Kind. Der hängende
Neti-Down-Aufruf wurde beendet; der vorhandene Installer stoppte verbleibende
VM-Helper sauber, ersetzte die Dienste und startete sie neu. Der unterbrochene
Journal-Eintrag wurde regulär per apply--abort bereinigt, danach Up erfolgreich.
Ein anfänglicher Docker-HTTPS502 blieb als Fehlbeleg erhalten. Nach gezielter
Listener-/Backendprüfung bestanden Docker und K3s mit200. Keine Behauptung eines
zusätzlichen Produktfixes aus diesen Betriebsbefunden.

## Einzig offene externe Bedingung

F17 / [#76](https://github.com/frankhildebrandt/vzctl/issues/76): saubere PKG-
Erstinstallation und anschließendes PKG-Update/Wiederanlauf auf separatem Mac
mit macOS26. Laut Nutzer ist derzeit kein solcher Mac verfügbar. Die lokale
Entwicklungsinstallation und isolierte Hooktests ersetzen diesen Nachweis nicht.
#73 bleibt als übergreifender Tracker für genau diese Bedingung offen.

Neue Nutzeranforderungen weiter im bestehenden Canvas aufnehmen. Höchstens
zwei neue Arbeitspakete gleichzeitig; Runtime-Mutationen nacheinander. Vorhandene
Nachweise übernehmen; nur konkrete neue Änderungen/Fehler/Lücken erneut prüfen.
Folgeauftrag: [10](10-next-slice.md).
