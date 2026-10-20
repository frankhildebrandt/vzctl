# Folgeauftrag: Repository-Auslieferung und Releaseentscheidung

Stand: 20.10.2026. Maßgeblich: [Abschlussziel12](12-release-completion-goal.md).
**Technische Restabnahme abgeschlossen, 61/63.** Keine weiteren Lab-Testläufe
oder Nebenoptimierungen nötig. Neue Nutzeranforderungen weiter im Canvas aufnehmen.

## Bereits fertig — nicht erneut ausführen

Review-Fixes, Aufteilung der großen Dateien, CLI/Swift/UI-Verträge, K3s mit
stackbezogenen Manifesten und vier Controller für VM/Docker/K3s. Full/Repeat/Late,
Besitzschutz/Ausfälle/Reparatur, Datenerhalt, CP-Neustart und Lab-Down bestanden.
Nativer UI-Apply37 s erfolgreich, Generation14 done; danach alle VM-/Docker-/K3s-
Consumer PostgreSQL/S3/OIDC erfolgreich und HTTPS200. MCP/Events/Kubeconfig
belegt. Aktuelle Dev-TAR/PKG/DMG samt Inhalts-/Signaturprüfung fertig.

Belege: [09](09-live-update-acceptance.md),
[native UI](ui-native-apply-2026-10-20.md),
[Pakete](package-verification-2026-10-20.md),
[GitHub](06-github-tracking.md),
[Canvas](canvases/stack-review-2026-10-20.canvas.tsx).

## Freigegebener aktueller Auftrag

1. Nutzer hat Commit/Push/Draft-PR freigegeben. Gesamtstand auf
   codex/release-readiness-2026-10-20 übernehmen, pushen und Draft-PR gegen main
   erstellen; danach Nachweise/Canvas aktualisieren. Kein Merge beauftragt.
   Vorhandene Änderungen bewahren, nichts blind verwerfen. Keine neuen Builds
   oder kompletten Testserien ohne Quelländerung/konkrete Lücke.
2. Release-Version und Kanal festlegen; danach erforderliche Versions-/Doku-
   Synchronisierung und Release-Artefakte für genau diese Version erstellen.
   Vorhandene Dev-Pakete nicht umetikettieren. Tag/Veröffentlichung nur beauftragt.
3. Sobald separater sauberer macOS-26-Mac verfügbar: PKG-Erstinstallation und
   PKG-Update/Wiederanlauf gemäß #76 prüfen. Danach verbleibende echte
   GitHub-Kriterien schließen und Canvas aktualisieren.

Die offenen Dependency-PRs sind inventarisiert, keine neun neuen Releaseblocker.
Bei konkreter Repository-Auslieferung Überschneidungen mit den bereits geprüften
lokalen D5/D6-Updates abgleichen; kein ungeprüftes Sammelmerge.

## Betriebsgrenzen

Lab 3/Neti 3 laufen; IQ-Store 3 bleibt absichtlich gestoppt, Guest-Trust dort beim
normalen späteren Up/Apply. Historischer Worker-Exec-P99-Hinweis 8,5 s / Queue 0 / ready
bleibt transparent; keine künstliche Last zum Grünmachen. Runtime ist abgenommen
und ruht. Keine neuen Applies/Reboots/Provider- oder Eigentumsänderungen nötig.


## Aktueller erweiterter Auftrag

Merge, Release und lokale Neuinstallation sind ausdrücklich freigegeben.
R2–R4 im Canvas führen die Arbeit. Zuerst Versionen auf0.3.0-alpha4 synchronisieren,
Build/PR/CI abschließen und mergen; dann Tag/Release mit tatsächlichen neuen
Paketen veröffentlichen und diese lokal installieren. Zuvor aktive Stacks
anschließend wiederherstellen, IQ-Store gestoppt lassen. Kein weiterer
Repeat-/Late-/Fault-/Retentionlauf; nur angemessene Release-/Update-Smokes.
Die früheren Freigabegrenzen oben beschreiben den Stand vor diesem Folgeauftrag.
