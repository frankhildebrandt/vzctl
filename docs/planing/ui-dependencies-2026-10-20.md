# D6: UI-Sicherheitsupdates

Stand: 20.10.2026. Nur lokale Quellen, Abhängigkeiten und Tests; keine
Installation oder Neustarts der nativen App bzw. laufender Dienste.

## Ergebnis

Der UI-Audit sinkt von **8 auf 0 Befunde**. Vorher: 1 kritisch, 3 hoch,
3 moderat, 1 niedrig. Der aktualisierte Lockfile wurde durch `npm ci`
neu installiert und anschließend getestet.

| Paket | Vorher installiert | Jetzt installiert |
| --- | --- | --- |
| Vitest, Coverage, Mocker | 4.1.10 | 4.1.12 |
| Vite | 8.2.0 | 8.3.4 |
| esbuild | 0.27.7 | 0.28.2 |
| seroval, seroval-plugins | 1.6.0 | 1.6.11 |
| undici | 7.29.0 | 7.30.1 |
| nanoid | 3.3.16 | 3.3.20 |
| source-map-js | 1.2.1 | 1.2.2 |

Vitest/Coverage, Vite und esbuild sind im Paketmanifest auf die gemeinsam
geprüften Versionen gesetzt. Vite 8.3.4 erlaubt esbuild 0.28 offiziell als
Peer. Transitive Pakete wurden innerhalb der bereits erlaubten Bereiche
aktualisiert. Kein `audit fix --force`, keine Overrides.

Zusätzlich ist `tslib 2.8.1` ausdrücklich als Laufzeitabhängigkeit deklariert:
AntV X6 importiert sie in ausgeliefertem JavaScript, führte sie aber nur als
eigene Entwicklungsabhängigkeit. Der bereinigte Paketbaum machte dieses
bisher zufällig durch andere Pakete erfüllte Erfordernis sichtbar.

## Offizielle Advisory-Prüfung

- [Vitest/Mocker](https://github.com/advisories/GHSA-82fw-gwwq-j7x9):
  Dateizugriff über Redirect-Mocks außerhalb der erlaubten Serverpfade,
  behoben ab 4.1.11. Relevant für entsprechende Entwicklungsserverpfade.
- Seroval:
  [Promise-/Plugin-Aufruf](https://github.com/advisories/GHSA-p6vx-979v-rg4c)
  und [ungeprüfte TypedArray-Länge](https://github.com/advisories/GHSA-jp82-f5mq-hwhp)
  betreffen Deserialisierung. Seroval gelangt über TanStack Router in den
  Abhängigkeitsbaum; die gepatchte Version wurde auch im Produktionsbundle
  geprüft.
- [esbuild](https://github.com/advisories/GHSA-g7r4-m6w7-qqqr):
  Dateizugriff im Windows-Entwicklungsserver, behoben ab 0.28.1.
- [undici BalancedPool](https://github.com/advisories/GHSA-w293-vg96-wgc3):
  verlorene TLS-Verbindungsoptionen; der Audit enthält zusätzlich weitere
  HTTP-/WebSocket-/Cache-Advisories, gemeinsam behoben ab 7.29.1.
  Im UI-Baum kommt undici über die Testumgebung jsdom.
- [nanoid](https://github.com/advisories/GHSA-2v37-7h3g-55p8):
  Endlosschleife bestimmter benutzerdefinierter Generatoren; Fix ab 3.3.18.
- [source-map-js](https://github.com/advisories/GHSA-68fv-2mgg-jv7q):
  Event-Loop-Blockade durch manipulierte Source-Map-Offsets; Fix ab 1.2.2.

Die Severity des Paket-Audits ist keine Behauptung, dass jeder gemeldete
Angriffspfad in dieser macOS-UI erreichbar war. Die betroffenen Versionen
wurden trotzdem vollständig entfernt.

## Validierung

- Sauberes `npm ci`: erfolgreich, Audit 0.
- Abschließendes `npm audit --json`: alle Schweregrade 0.
- `npm ls --depth=0`: keine fehlenden oder ungültigen direkten Abhängigkeiten.
- `npm test`: **149 Tests in 26 Dateien bestanden**.
- `npm run build`: TypeScript und Vite erfolgreich, ohne Buildwarnungen.
- Chromium und WebKit: **12 Dev-E2E** bestanden, einschließlich
  Topologie-Rendern, Resize/Remount, Palette/Drag, Save/Reload und expliziter
  Recovery. Enter auf neu geöffnetem Purge-Dialog löst keine Mutation aus.
- Chromium und WebKit: **2 Produktionsbundle-E2E** bestanden, einschließlich
  dynamisch geladener Topologie und Terminal.
- esbuild 0.28.2 zusätzlich direkt mit einer TypeScript-Transformation geprüft.

## Verbleibende Hinweise

Keine offenen npm-Advisories zum Prüfzeitpunkt. npm meldet nicht pauschal
freigegebene Installationshooks für esbuild und die optionalen fsevents-Pakete.
Die benötigten Plattform-Binaries, Tests und Builds funktionieren ohne
zusätzliche Freigabe dieser Hooks.

Vite 8.3 spiegelt Browser-Konsolenmeldungen in den Terminal-Output. Die dabei
entdeckten AntV-Uplink-Fallbacks wurden separat als F18 behandelt.

## F18: Kurze Internet-Uplinks

Die isolierte Reproduktion der bestehenden E2E-Topologie ergab 16 Fallbacks
an Internet-Uplinks (Initialladen, Palette, Undo). Der Abstand zwischen
Gateway-Unterkante und Netzoberkante beträgt nur 24px; die bisherigen
12px-Hindernisabstände plus Portgeometrie lassen keinen freien
Manhattan-Korridor. AntV zeichnete die Kanten anschließend orthogonal.

Nur diese kurzen Uplinks verwenden jetzt ausdrücklich den orthogonalen Router
mit 6px Abstand. Knotenpositionen bleiben erhalten. NIC-Anbindungen und
Netz-Routen behalten Manhattan samt Hindernisumgehung und Diagnosemeldungen.
Orth vermeidet beliebige vom Nutzer darüber gelegte Knoten nicht; der Einsatz
ist deshalb auf die dekorativen Internet-Uplinks begrenzt und im Code erklärt.

Abnahme: 10/10 Chromium-/WebKit-Topologietests ohne Fallbackwarnung; sichtbare
Uplink-Pfade und beide tatsächlichen Portanschlüsse nach Gateway-/Netzbewegung
sowie Save/Reload geprüft. Default- und verschobene Darstellung zusätzlich
visuell kontrolliert. 19/19 vorhandene Projektions-/Domain-Tests und
`tsc --noEmit` erfolgreich. Kein globales Layout-Redesign oder Logfilter.
