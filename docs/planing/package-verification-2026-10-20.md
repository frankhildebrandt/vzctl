# Paketprüfung 20.10.2026

**Ergebnis: bestanden — lokaler F48-Dev-Paketbau.** Dieser Bericht ersetzt den
früheren F26-Paketstand. TAR, PKG und DMG wurden vor der angeordneten Pause
gebaut und vollständig geprüft. Die Dokumentation wurde danach ausschließlich
anhand der vorhandenen Belege vervollständigt; keine erneute Prüfung oder Installation.

Ein separater sauberer Mac mit macOS 26 ist derzeit nicht verfügbar.
PKG-Erstinstallation und PKG-Update auf diesem System bleiben in #76 extern offen.
Die lokalen Inhaltsprüfungen ersetzen diese Abnahme nicht.

## Build und Identität

- Eingefrorener Produktstand einschließlich F48; keine Produktquellen durch diesen Paket-Slice geändert.
- `RUSTFLAGS='-D warnings' cargo build --workspace --release`: Exit 0, 9,67 Sekunden.
- `swift build -Xswiftc -warnings-as-errors --package-path daemon --configuration release`: Exit 0, 1,71 Sekunden. Compiler-Caches benötigten Sandboxfreigabe; der erfolgreiche Lauf war warningfrei.
- Vorhandener nativer F46-UI-Build übernommen, kein unnötiger UI-Neubau oder `npm install`. App-Binary SHA256: `c0728d18154013d9c8cfc36238703162609aa8a3fe174830afe91d88dbfcde0a`. Die vollständige App wurde gegen die Paketinhalte verglichen und ihre tiefe/strikte Signatur geprüft.
- Swift gemäß Makefile ad hoc signiert. Helper, Supervisor und Net besitzen `com.apple.security.virtualization`, kein `com.apple.vm.networking`. CLI nach dem Workspace-Build explizit ad hoc signiert.
- Geprüfte vorhandene Vendor-Bundles übernommen: Caddy 2.11.4, Dex 2.45.1, qemu-img 8.2.1 samt dylibs, Lizenzen und Versionsdateien. Keine Vendor-Downloads.
- Regulärer Paketpfad: `RELEASE_TAG=dev ./scripts/package-macos.sh`, Exit 0. Für DMG-Erstellung war Sandboxfreigabe erforderlich.
- Ziel ARM64, eigene Komponenten und App Version 0.0.1, Mindest-macOS 26. Basis-Commit `76f833d7b8c94a5e02ff219e58433aee8fe5dcea`, `VERSION.json` ausdrücklich `dirty: true`.

**Paket-CLI und installierte Live-CLI unterscheiden sich byteweise.** Der erneute
Workspace-Releasebuild mit Signierung erzeugte Paket-CLI
`105cb475ddf7865d0abe3be2488f79a1f5e38d2ba2124f73b9cfbfdf75d0c823`.
Die zuvor installierte, für die F48-Live-Abnahme gebundene CLI bleibt
`f7cdececf68f6a2239cb0563d0f8287842ef0faa0612c1228fb3a395b7d384ab`.
Dieser Slice hat sie nicht ersetzt. Die Live-Receipts behalten ihren ursprünglichen
Hash; ihre Ergebnisse werden nicht als Byteidentitätsnachweis für die Paket-CLI ausgegeben.

## Artefakte

Alle mitgelieferten `.sha256`-Dateien stimmen mit den tatsächlichen Artefakten
überein. Dateien liegen unter `dist/`; die Werte gelten ausschließlich für diesen Prüfbau.

| Datei | Bytes | SHA256 |
|---|---:|---|
| `vzctl-dev-darwin-arm64.tar.gz` | 41,808,286 | `48936400220e303654dbb9b1b5414f3c06f6efcfd705cad6d50f96e0c05dd8d9` |
| `vzctl-dev-darwin-arm64.pkg` | 45,331,531 | `6ed10352087810e6a8e2a95f2750c3d252850ed56575b22639ff77314cbcd292` |
| `vzctl-dev-darwin-arm64.dmg` | 48,734,523 | `85acc3d9408371cabc9d0d6cb58f47a500fa89d757e6c333434162f6584b582a` |

## Tatsächlich geprüfte Inhalte

1. PKG ausschließlich mit `pkgutil --expand-full` entpackt. Kennung `dev.vzctl.installer`, Version 0.0.1, Installationswurzel `/`, App fest unter `/Applications/vzctl.app`, keine Bundle-Relocation.
2. Alle acht eigenen Programme bytegenau gegen die aktuellen Release-Builds verglichen; jeweils `codesign --verify --strict` erfolgreich.
3. Alle Vendor-Dateien einschließlich dylibs, Lizenzen und Versionsdateien bytegenau gegen `daemon/Vendor/` verglichen.
4. `preinstall`, `postinstall`, `installer-common.sh` und `stop-vz-helpers.sh` bytegenau gegen die Quellen geprüft. Keine echten Installerhooks ausgeführt.
5. Jede reguläre Datei der App gegen den vorhandenen Tauri-Build verglichen. App-Metadaten und `codesign --verify --deep --strict` erfolgreich.
6. TAR enthält dieselben acht Programme, Vendor-Dateien, Lizenz und `VERSION.json` wie PKG/Build. AppleDouble-Dateien separat als Metadaten erkannt.
7. DMG mit `hdiutil verify` geprüft und ausschließlich readonly/nobrowse eingehängt. Enthält byteidentisches PKG, dieselbe signierte App, `/Applications`-Link und README.
8. Test-DMG anschließend erfolgreich ausgehängt. Alle eigenen Build-/Prüfprozesse beendet; keine Installation, VM- oder Dienstaktion.

## Binärnachweis

| Programm | SHA256 im Build und Paket |
|---|---|
| `vzctl` | `105cb475ddf7865d0abe3be2488f79a1f5e38d2ba2124f73b9cfbfdf75d0c823` |
| `vzctl-mcp` | `4ee4081e9c6f2e7b96d4c92ead7e6cf6501f5bd03f7523f8758c249cd9ee5e8f` |
| `vzctl-oidc-simple` | `25098ac9f819a94016a289c1a08f15290ca064019fdb5fa14cc227068f25c530` |
| `vz-net` | `884683aaa90c9eb5b4603908d0b4a609b106050d0fd03c188b10764394ae821b` |
| `vz-edge` | `8fd8455c19aaed9137819b11097530e6968cc061648235ba5e8010bfc5aa5a10` |
| `vz-supervisor` | `096a293bee58112839cf36e5e9163eea1628d3318a2881d99dacd9f684353a4c` |
| `vz-helper` | `a56baa7658adecbeb26e3927a1d4a781f62a5aba0fbefa5c80fc1c6ada716756` |
| `vz-dns-bind` | `36f59d2a4cd375d873bf3ce89e365fdf40d7d0f9358fbb445c4123a58c3279b4` |

## Tests und Grenzen

- Sieben vorhandene isolierte Packagingtests bestanden. Dienste und Installerwerkzeuge waren Fixtures; der zunächst durch die Sandbox blockierte Test-Unix-Socket wurde mit erforderlicher Freigabe erneut geprüft.
- Rust und Swift warningfrei gebaut. Keine neue vollständige Produkt-Testserie in diesem Paket-Slice; bestehende fachliche Abnahmen stehen in [09-live-update-acceptance.md](09-live-update-acceptance.md) und [ui-native-rendering-2026-10-20.md](ui-native-rendering-2026-10-20.md).
- Dev-Artefakte, nicht veröffentlicht. PKG erwartbar unsigniert (`no signature`), App/Programme ad hoc signiert; keine Developer-ID-Signierung oder Notarisierung.
- Inhalts-, Metadaten-, Signatur- und Integritätsnachweis; keine saubere Erstinstallation, kein reales PKG-Update und keine Gatekeeper-Abnahme.
- `git_commit` plus `dirty` ist keine reproduzierbare Build-Attestierung. Maßgeblich sind die konkreten obigen Hashes.

## Vorhandene Detailnachweise

- Rust: `/tmp/vzctl-f48-package-rust-release.log`
- Swift: `/tmp/vzctl-f48-package-swift-release.log`
- Wiederverwendeter UI-Build: `/tmp/vzctl-f46-ui-release.log`
- Entitlements: `/tmp/vzctl-f48-package-entitlements.log`
- Packagingtests: `/tmp/vzctl-f48-packaging-tests.log`
- Paketbau: `/tmp/vzctl-f48-package.log`
- Prüfskript: `/tmp/vzctl-final-package-inspect.py`
- Prüfprotokoll: `/tmp/vzctl-f48-package-inspect.log`
- Maschinenlesbarer Abschlussbericht: `/tmp/vzctl-final-package-report.json`

Die temporären Nachweise können später gelöscht werden; dieser Bericht bewahrt die
wesentlichen Ergebnisse. Das vorbereitende Signaturlog entstand vor der letzten
CLI-Signierung; für die finale CLI gilt der Abschlussbericht und die obige Tabelle.
