# Release 0.3.0-alpha4

Stand: 20.10.2026. Nutzer hat Branch/PR, Merge, Release und lokale Neuinstallation beauftragt. Dieses Dokument hält ausschließlich die Build- und Paketprüfung fest.

## Version und Builds

- Alle acht eigenen Komponenten tragen `0.3.0-alpha4`: CLI, MCP, OIDC-Simple, Tauri-Rust, Tauri-Konfiguration, npm-UI, Swift und tatsächliches App-Bundle. Eigene Lockfile-Stanzas sind synchronisiert; Abhängigkeiten und historische Testfixtures bleiben unverändert.
- Tauri erzeugt tatsächlich `CFBundleShortVersionString` und `CFBundleVersion` mit `0.3.0-alpha4`; keine Lockerung des Versionsguards nötig.
- `cargo build --workspace --release --locked --offline`: bestanden, 11,03 s.
- `swift build --package-path daemon --configuration release`: bestanden, 26,65 s.
- `npm run tauri:build -- --bundles app`: bestanden; Rust 11,15 s, Frontend-Typprüfung und Vite-Bundle erfolgreich.
- Keine Rust-/Swift-/UI-Buildwarnungen in den erfolgreichen Logs. Swift benötigte Zugriff auf seine regulären Systemcaches außerhalb der Sandbox.
- Daemons mit bestehenden Entitlements ad-hoc signiert; App ad-hoc signiert und `codesign --verify --deep --strict` bestanden.
- Vorhandene sieben Packagingtests: 7/7 bestanden in 21,312 s. Der erste Sandboxlauf scheiterte allein am Unix-Socket des isolierten Launchctl-Teststubs; unveränderte Wiederholung außerhalb der Sandbox erfolgreich.
- Bestehender Metadatavalidator prüft alle acht Quell-/Bundle-Versionen, alle acht Binärversionen und qemu-img-Abhängigkeiten erfolgreich. Vendorpins unverändert: Caddy 2.11.4, Dex 2.45.1, qemu-img 8.2.1.

Logs liegen lokal unter `/tmp/vzctl-alpha4-{rust,swift,ui,package-tests}.log`.

## Finale Artefakte

Finaler Paketbau mit `RELEASE_TAG=v0.3.0-alpha4 ./scripts/package-macos.sh` bestanden. Basis ist der saubere Release-Commit `d050883f0d3636593718797a75f39071acd9f4bc`; `VERSION.json` belegt exakt diesen Commit, `dirty: false`, Tag `v0.3.0-alpha4` und Version `0.3.0-alpha4`. Paketversion des macOS-Installers ist nach bestehendem Vertrag numerisch `0.3.0`; vollständige Alpha-Version steht in `VERSION.json` und den Komponenten. Gegenüber dem PR-Merge `56634842fd202a6fbc8339feb5a5ff6a01dd1f22` enthält dieser Commit ausschließlich eine README-Korrektur.

Die vorhandene Inhaltsprüfung wurde mit exakt diesen Versions-/Commit-Erwartungen wiederverwendet: alle acht Binärdateien sowie App-Dateien, Vendorpakete und vier Installerhooks bytegleich zum Build; Signaturen gültig; TAR-/PKG-Metadaten identisch; DMG-Integrität und Inhalte readonly geprüft und anschließend ausgehängt. Keine Installation oder Runtimeaktion durch die Paketprüfung.

| Artefakt | Bytes | SHA256 |
|---|---:|---|
| `vzctl-v0.3.0-alpha4-darwin-arm64.tar.gz` | 41852457 | `c8c7b737833579fdea4194e0c79c4a7450070b9f03494bef411b3bc7aeaf11fa` |
| `vzctl-v0.3.0-alpha4-darwin-arm64.pkg` | 45373445 | `2b48d870322338ffc40835fcf44eca867eccc0d37340b21fb267edd1723bffef` |
| `vzctl-v0.3.0-alpha4-darwin-arm64.dmg` | 48775807 | `8068508797cb6027ed683ce77fb5938f52a8d883286f0477edfb637ffc7166f9` |

Alle drei `.sha256`-Begleitdateien stimmen. Detailbericht `/tmp/vzctl-alpha4-package-report.json`; Protokolle `/tmp/vzctl-alpha4-package.log` und `/tmp/vzctl-alpha4-package-inspect.log`. Der vorbereitende schmutzige Metadatalauf wurde nicht ausgeliefert.

| Programm | SHA256 im finalen Paket |
|---|---|
| `vzctl` | `28c8b43e6e119ac6d9dd8369fd55eaa8f5b4f5e475279c212f0cd4b7ea94933f` |
| `vzctl-mcp` | `0321c5daed00c043f0951e403fdb84eab659b369100ae46c9feeef87907ea401` |
| `vzctl-oidc-simple` | `ae5a70ea9b64e584043fc2f9159bb7a750e3a9266dacd14b809acdbf2c2f22f5` |
| `vz-net` | `48afeb3051ad0a5f04cfd595764e816021fe5843826b1f19679c2439589eb203` |
| `vz-edge` | `40b730a1a1e8159d7cda188076b17fd5f82393161eb07bdfeceead8ca75d7dd6` |
| `vz-supervisor` | `bf6ccf0e778faa23b7cc546a91bf30773671ba39da1564df2a75ae782f7c84d2` |
| `vz-helper` | `52ea3a5190187fd3bf831c578652557a1230135781022cc98022793fbaa6f59d` |
| `vz-dns-bind` | `51cdc5d6231ea6df32090b493ebaa23b23b430d331ef4e56e6ff087f3b8fff50` |


Keine Developer-ID-Signierung oder Notarisierung wird behauptet. Der separate Erstinstallations-/Update-Test auf einem weiteren macOS-26-Mac bleibt extern offen.

## Veröffentlichung und lokale Installation

[Release](https://github.com/frankhildebrandt/vzctl/releases/tag/v0.3.0-alpha4)
veröffentlicht; Tag zeigt auf exakt d050883. Alle6 GitHub-Asset-Digests und
Dateigrößen entsprechen den lokalen Dateien. [CI auf exakt d050883](https://github.com/frankhildebrandt/vzctl/actions/runs/39488109999)
mit allen14 Schritten grün: Rust496/2 bestehende Ignores, Swift213, UI172, Paket7.

Lokale vorhandene Nutzerinstallation mit den geprüften Binaries/App aktualisiert
(`make -o release -o ui-build install`, ohne Neubau). Root-DNS-Helfer über die
reguläre REST-/Admin-Route aktualisiert. Alle8 installierten Programme, Root-
Helfer und sämtliche App-Dateien bytegleich zu den geprüften Paketbuilds.
Kein PKG-Installerhook auf diesem Mac ausgeführt; keine Migration der vorhandenen
Nutzerinstallation auf systemweite PKG-Pfade.

SQLite/App/Binaries/Configs vorher gesichert. Lab sauber gestoppt; Neti-Down
wartete im alten Edge bei Process.waitUntilExit auf ein beendetes Caddy-Kind.
Der wartende CLI-Aufruf wurde beendet; vorhandener Installer stoppte alle
verbliebenen VM-Helper sauber, danach CP/Edge/Net. vz-net stoppte graceful.
Unterbrochenes Journal regulär per apply--abort bereinigt. Neti und Lab mit
unverändertem YAML wieder gestartet, beide up completed. Dies ist eine
Betriebswiederherstellung und kein behaupteter zusätzlicher Produktfix.

Final6 laufende VMs,3 IQ-Store-VMs weiter gestoppt; Neti/Lab healthok und
warnings0. IQ-Store checksskipped/warnings0. Neun Ready-Claims plus bekanntes
foreign Retained, zwei K3s-Nodes Ready. VM/Docker/K3s PostgreSQL/S3/OIDC
erfolgreich; striktes HTTPS200. Vorhandene PostgreSQL-Zeile, S3-Objekt,
neun Binding-Hashes und S3-Seeds unverändert. Initialer Docker-HTTPS502
als Fehlbeleg erhalten; nach gezielter Listener-/Backendprüfung Docker und
K3s erfolgreich. Kein erneuter Full/Repeat/Late/Fault-/Retentionlauf.

Native App geöffnet: Lab/Neti3/3 und Gesundheitok, IQ-Store0/3 mit
Gestoppt/nichtgeprüft. Dashboard9VMs/6aktiv. Screenshots privat unter
/private/tmp/vzctl-release-20261020/ui/.

Dauerhafter redigierter Abschluss: [Verifikationsdaten](release-0.3.0-alpha4-verification.json).
Private Einzelreceipts unter /private/tmp/vzctl-release-20261020/; deren
SHA256 stehen in den Verifikationsdaten. Die externe PKG-Abnahme bleibt
in #76 und Tracker #73 offen.
