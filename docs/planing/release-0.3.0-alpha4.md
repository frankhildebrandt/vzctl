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

Noch nicht gebaut: Erst der vom Root bestätigte saubere Release-Commit wird in die endgültigen Pakete eingebettet. Der vorbereitende Metadatalauf war ausdrücklich kein finales Releaseartefakt. Paketversion des macOS-Installers ist nach bestehendem Vertrag numerisch `0.3.0`; vollständige Alpha-Version steht in `VERSION.json` und den Komponenten.

Keine Developer-ID-Signierung oder Notarisierung wird behauptet. Der separate Erstinstallations-/Update-Test auf einem weiteren macOS-26-Mac bleibt extern offen.
