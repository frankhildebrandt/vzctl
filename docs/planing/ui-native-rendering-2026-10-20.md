# Native UI-Abnahme F3 / F28 – 20.10.2026

Status: **abgeschlossen**. Die native Abnahme erfolgte durch den koordinierenden Agenten; die Browserprüfungen nutzten ausschließlich isolierte Fixtures.

## F3: Topologie und Hintergrunddarstellung

Die zeitweise leere native Topologie entstand bei Navigation über Accessibility-Aktionen im Hintergrund. Die Screenshots zeigten dabei inaktive macOS-Fensterknöpfe. X6 plant die Zellendarstellung über `requestAnimationFrame`; auch das initiale Einpassen nutzt diesen Mechanismus. Ein im Hintergrund angehaltener WKWebView kann deshalb bereits die geänderte Route und das Raster anzeigen, während die Graphzellen noch nicht gezeichnet sind.

Der native Gegenbeleg bestätigte die Wiederaufnahme: Im Vordergrund erschienen beide Netze und alle drei Neti-VMs bei unveränderter Route und derselben App-PID `23074`, ohne Neustart, Reload oder Graphänderung. Nach Installation von F28 waren außerdem Debug → Topologie und Fit View erfolgreich. Der anschließende Wechsel zur IQ-Store-Topologie zeigte `lan` mit drei VMs.

Die Regressionen decken wiederholte erfolgreiche SQL-Abfragen mit anschließendem Topologie-Remount ab:

- `apps/vzctl-ui/e2e/topology.spec.ts`: vier Wechsel bei 1280 und 1100 Pixel Breite; sichtbare VM-Geometrie statt bloßer DOM-Existenz. Chromium und WebKit bestanden, auch mit Produktions-React.
- `apps/vzctl-ui/e2e/topology-api.spec.ts`: gebautes Webbundle mit Tauri-API-Mock, YAML-Parsing, fehlender Diagrammdatei (404), Hot-Pink-Theme und 1848 × 1147 Pixeln. Vier SQL → Topologie-Wechsel bestanden in beiden Browsern.
- Derselbe Bundle-Test hält Animationsframes gezielt zurück: Der Canvas enthält zunächst keine VM-Darstellung. Nach Freigabe erscheinen alle drei VMs ohne Reload. Dies sichert die Wiederaufnahme ab.

**Prüfgrenze:** Eine leere Graphdarstellung nach Hintergrund-AX-Navigation allein belegt keinen Datenverlust oder Remount-Fehler. Vor einer solchen Bewertung muss die native App tatsächlich im Vordergrund sein und ihre Animationsframes wieder verarbeiten. Ein reines AX-Raise genügte in dieser Prüfung nicht. Die Hintergrund-Throttling-Konfiguration bleibt unverändert.

## F28: Rückmeldung der Debug-Prüfung

Zuvor zeigte die Debug-Seite ausschließlich negative Doctor-Ergebnisse an; erfolgreiche Prüfungen blieben unsichtbar. Die UI zeigt jetzt laufende Prüfung, Erfolg mit Werkzeug und VM sowie Fehler. Während einer erneuten Prüfung oder nach einem Fehler erscheint kein veralteter Erfolg.

Der gezielte Regressionstest schlug vor der Änderung fehl und bestand danach; vier Tests der Debug-Seite und der TypeScript-Check waren erfolgreich. Der anschließend gebaute und installierte native Stand wurde geprüft:

- „Debug-Voraussetzungen erfüllt“ sichtbar; `psql` und `nats` als verfügbar aufgeführt.
- Erneuter Klick auf „Voraussetzungen prüfen“ erfolgreich.
- `SELECT 1`: Exitcode 0, eine Ergebniszeile.
- Debug → Topologie: zwei Netze und drei VMs sichtbar; Fit View funktioniert.

Zusätzliche native Abnahme: Der Docker-journald-Stream lud und folgte neuen Einträgen; der sichtbare Zähler stieg von 200 auf 229.

## Nachweise

- [Native Debug-Abnahme](/Users/frank/.codex/visualizations/2026/10/20/01a15839-4351-79d3-a4fd-8ab23bcb8bd7/vzctl-debug-live-verified.png)
- [Native Topologie im Vordergrund](/Users/frank/.codex/visualizations/2026/10/20/01a15839-4351-79d3-a4fd-8ab23bcb8bd7/vzctl-topology-foreground-verified.png)
- Lokale Testlogs: `/tmp/vzctl-debug-feedback-red.log`, `/tmp/vzctl-debug-feedback-green.log`, `/tmp/vzctl-debug-topology-remount.log`, `/tmp/vzctl-debug-topology-production.log`, `/tmp/vzctl-topology-api-bundle.log`.

Die Screenshots und Testlogs sind lokale Sitzungsartefakte; dieser Bericht hält die Ergebnisse dauerhaft im Repository fest.

Nutzerbestätigung am 20.10.2026 nach App-Wechsel in den Vordergrund: „Ja, jetzt sichtbar“.

## Neues Controller-Lab

Das bestehende Lab-Verzeichnis über den nativen Öffnen-Dialog registriert, ohne
Config zu speichern oder Apply auszulösen. Topologie zeigt `lan` mit
`10.123.0.0/24`, Docker-Netz `containers` mit `10.124.0.0/24`, Router `ops`
an beiden Netzen sowie `server` und `worker` mit ihren Lab-Adressen. Keine
Validierungsprobleme. Die logische Bezeichnung `lan` wird korrekt vom bestehenden
IQ-Store-LAN getrennt dargestellt.

Screenshot:
`/Users/frank/.codex/visualizations/2026/10/20/01a15839-4351-79d3-a4fd-8ab23bcb8bd7/vzctl-controller-lab-topology.png`.
Die responsive Ansicht bei kleinerer Fensterbreite lässt sich bis zu allen
Knoten und zum Inspector scrollen.

### Leseabnahme nach dem fehlgeschlagenen Full-Apply

Am 20.10.2026 gegen 15:11 Uhr blieb die vorhandene native App nach
Topologie → Betrieb → Debug → VM → Fehler → Doctor → Topologie funktionsfähig.
Beide Netze und alle drei Lab-VMs waren sichtbar; Betrieb meldete 3/3 running,
die Ops-Detailseite PID 66348 und Agent ready. Die Debugseite erklärte die
fehlenden `observability.dataSources`; es wurde keine SQL-Abfrage gestartet.
Die Fehleransicht zeigte Sessionfehler mit Route und QueryKey.

Doctor meldete 12 ok, 2 Warnungen, 0 Fehler: `dns.host_port` war bereits belegt,
ein Docker-Context unerreichbar. Gleichzeitig waren DNS-Probe, Bind-Helper und
CA-Trust erfolgreich. Diese Warnungen sind Beobachtungen, noch keine bestätigten
Produktfehler; eine gesonderte Ursachendiagnose steht aus.

F41 wurde dabei bestätigt: Der gespeicherte fehlgeschlagene CLI-Apply erschien
auf Betrieb nicht mit Journal, Schritt oder Ursache; sichtbar waren nur
15 offene Diff-Actions neben dem gesunden VM-Inventar. Consumer-Erfolg ist aus
dieser Leseprüfung nicht ableitbar. Es gab keine Apply-/Resume-, VM- oder
YAML-Mutation.

### F41: Persistierter Apply-Fehler nativ abgenommen

Am 20.10.2026 gegen 15:30 Uhr wurde die aktualisierte App nach dem koordinierten
CP-/CLI-/UI-Austausch regulär geöffnet. Betrieb zeigte sofort „Letzter Vorgang
fehlgeschlagen“ mit der gespeicherten Docker-/SSH-Ursache „Connection refused“,
Schritt `ensure_containers`, Journal
`f0e0c5dd-ad44-46bf-901f-5daf2f325736`, Inhaber `localhost:6489`, PID 6489
und Lease-Ende `2026-10-20T13:21:38Z`. Die drei Lab-VMs wurden getrennt davon
als laufend angezeigt. Der Fehler blieb nach Betrieb → Topologie → Betrieb
sichtbar; die Topologie zeigte beide Netze und alle drei VMs.

Die Lease war zum Prüfzeitpunkt abgelaufen. Daher waren „Fortsetzen (--resume)“
und „Nur abbrechen“ mit ausdrücklichem Auswahlhinweis verfügbar. Beide blieben
unbetätigt. Die Sperre bei aktiver oder ungültiger Lease sowie Fehler beim
Resume sind durch gezielte Regressionen abgedeckt; eine aktive Lease wurde für
die native Prüfung nicht künstlich erzeugt.

13 UI-Tests, TypeScript-Prüfung und fünf Swift-Tests einschließlich echter
REST-Abfrage einer wieder geöffneten SQLite-Journaldatei bestanden. Ein
unabhängiger Review fand keine Blocker. Supervisor und UI wurden warningfrei
gebaut und signiert; native Artefakte:

- Supervisor SHA256: `096a293bee58112839cf36e5e9163eea1628d3318a2881d99dacd9f684353a4c`
- UI-Executable SHA256: `0c689c0fbf6de33e4670418cff14c6255170e0a47ab78bf50904044f167e2e59`
- Testlogs: `/tmp/vzctl-f41-ui-green.log`, `/tmp/vzctl-f41-types.log`,
  `/tmp/vzctl-f41-swift-green.log`.

F41 ist damit lokal und nativ abgenommen. Die geforderte Repository-Auslieferung
von #54 bleibt separat offen. Diese Prüfung belegt keine erfolgreiche
Consumer-Provisionierung und löste keine Recovery- oder VM-Mutation aus.

### #54: Deklarative edge-dmz-Topologie

Am 20.10.2026 gegen 15:34 Uhr wurde das vorhandene Verzeichnis
`examples/edge-dmz` über „Stacks → Öffnen“ in derselben installierten App
geöffnet und ausschließlich die Topologie gelesen. Ohne Debug-JSON waren
visuell erkennbar:

- `dmz` (10.80.1.0/24), `lan` (10.90.0.0/24) und das isolierte Docker-Netz
  `containers` (10.95.0.0/24).
- `web` und `host` in ihren Netzen sowie die beiden Router `router` und `docker`
  mit ihren jeweiligen zwei Netzanschlüssen und Adressen.
- Die Routen `dmz → lan` mit „via router“ und `lan → containers` mit
  „via docker“ sowie die Internet-Uplinks.

Nach Scrollen waren auch die Router-Knoten, Minimap und der Inspector mit
„Keine Probleme“ sichtbar. Die Angaben wurden mit der vorhandenen YAML
abgeglichen. Kein Speichern, Auto-Layout, Apply, Up oder VM-Start erfolgte;
das Öffnen ergänzt lediglich den lokalen Stack-Verlauf.

Damit ist das ausdrücklich auf edge-dmz bezogene Topologie-Kriterium erfüllt.
Der separate optionale Liveablauf `edge-dmz open → up → status tiles green`
bleibt ungeprüft. Ebenfalls offen bleiben der Nachweis einer reaktionsfähigen
UI während eines langen Up/Apply und die Repository-Auslieferung.

### #54: `make ui-dev` Start-Smoke

Am 20.10.2026 wurde der tatsächliche Makefile-Einstieg einmal kontrolliert
ausgeführt. `ui-dev` ruft `npm install` und danach `npm run tauri:dev` auf;
dieser Pfad installiert oder startet keine vzctl-Dienste. Die installierte
App wurde vorher regulär beendet.

`npm install` meldete unveränderte Abhängigkeiten und 0 Vulnerabilities.
Vite 8.3.4 startete auf Port 1420; der native Rust-Debug-Build endete nach
27,06 Sekunden ohne Compilerwarnungen und startete `target/debug/vzctl-ui`.
Npm wies auf nicht durch `allowScripts` abgedeckte Install-Skripte von
esbuild/fsevents hin. Es wurden dafür keine zusätzlichen Freigaben erteilt.
`package.json`, `package-lock.json` und der Tauri-`Cargo.lock` blieben
SHA256-identisch.

Die native Dev-Navigation, Topologie und Fehlerseite sind in diesem Lauf
**nicht abgenommen**: CUA inventarisierte den laufenden ungebündelten
Debug-Prozess nicht und lehnte sowohl den Namen als auch den vollständigen
Executable-Pfad mit `Invalid app` ab. Die bereits dokumentierte Abnahme der
installierten App ersetzt diesen fehlenden Dev-Nachweis nicht.

Ctrl-C beendete den eigenen Dev-Prozessbaum (Exit 130); anschließend liefen
weder Debug-App noch Tauri/Vite weiter und Port 1420 hatte keinen Listener.
Die installierte App wurde danach wieder regulär geöffnet; ihr Dashboard
war sichtbar. Keine Stack-, VM-, Save- oder Apply-Aktion erfolgte.

Lokale Nachweise: `/tmp/vzctl-ui-dev-smoke.log` und
`/tmp/vzctl-ui-dev-input-sha256.txt`. Start und kontrolliertes Beenden sind
belegt; native Dev-Navigation bleibt offen.
