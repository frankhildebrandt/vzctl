# Folgeauftrag: externe PKG-Abnahme

Stand:20.10.2026. PR #85 gemergt, v0.3.0-alpha4 veröffentlicht und lokal
abgenommen. Dashboard65/66; keine aktive technische Restarbeit.

## Sobald ein separater Mac mit macOS26 verfügbar ist

1. Dort das veröffentlichte PKG und seine SHA256-Datei beziehen und prüfen.
2. Saubere Erstinstallation über das PKG ausführen; App/CLI/MCP, Vendor-
   Laufzeiten, LaunchAgents, DNS-Helfer und Startreihenfolge prüfen.
3. Reales PKG-Update mit laufenden VMs ausführen; geordneten Stopp vor
   Payloadtausch, Wiederanlauf, Versionen, Daten und Agent-Bedienung belegen.
4. #76 und den Tracker #73 nach erfüllten Kriterien schließen; CanvasF17
   abschließen. Keine erneute Full/Repeat/Late/Fault-Serie ohne konkreten Anlass.

Der Nutzer hat derzeit keinen solchen Test-Mac. Dieser externe Nachweis ist
keine verdeckte Implementationsaufgabe und wird nicht als erfüllt markiert.

## Bereits erledigt

Review-Fixes, fachliche Dateiaufteilung, CLI/Swift/UI/MCP-Verträge, K3s mit
stackbezogenen YAML-Manifesten, vier Controller für VM/Docker/K3s, Live-
Verbraucher und Datenerhalt. Release-CI auf exakt d050883 vollständig grün;
alle6 Release-Asset-Hashes verifiziert. Lokale Programme/App bytegleich.

Sechs VMs laufen wieder; IQ-Store3 bleibt absichtlich gestoppt. Neti/Lab
healthok/warnings0, neun Ready-Claims plus ein bewusstes Retained, zwei Ready-
Nodes. Native UI und PostgreSQL/S3/OIDC/HTTPS geprüft. Keine weiteren
Neustarts, Applies oder Last zum Grünmachen nötig.

Belege: [Abschlussziel](12-release-completion-goal.md),
[Releasebericht](release-0.3.0-alpha4.md),
[GitHub-Tracking](06-github-tracking.md),
[Canvas](canvases/stack-review-2026-10-20.canvas.tsx).

Neue Nutzeranforderungen in diesem Dashboard aufnehmen und als konkrete
nächste Arbeitspakete bearbeiten; keine selbst erfundenen Nebenoptimierungen.
