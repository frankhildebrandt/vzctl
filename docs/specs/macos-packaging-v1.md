# macOS-Pakete v1

`make package` baut Release-Binaries, Tauri-App und Vendor-Abhängigkeiten.
`scripts/package-macos.sh` erzeugt TAR, PKG und DMG samt SHA-256-Dateien unter
`dist/`. Der Paketbau führt keine Installation aus.

Pflichtumfang: `vzctl`, `vzctl-mcp`, `vzctl-oidc-simple`, `vz-net`, `vz-edge`,
`vz-supervisor`, `vz-helper`, `vz-dns-bind`, Caddy, Dex und ein relocatable
qemu-img-Bundle. Vendor-Bundles enthalten Lizenz- und Versionsinformationen.
Die Projektlizenz und `VERSION.json` sind ebenfalls enthalten. Fehlende oder
nicht ausführbare Pflichtdateien brechen den Build ab.

`package-metadata.py` vergleicht Cargo-, Swift-, UI-/Tauri-, App-Bundle- und
ausführbare Binary-Versionen. Ein Release-Tag muss dieser Version entsprechen.
Für lokale Pakete ist `RELEASE_TAG=dev` möglich. `pkgbuild` erhält ausschließlich
die numerische Basisversion; ein Alpha-Suffix bleibt in den Metadaten erhalten.
Die Versionsnummer wird nicht automatisch geändert. Git-Commit und Dirty-Status
des Paket-Checkouts stehen in `VERSION.json`; dies ersetzt keine reproduzierbare
Build-Attestierung.

Die App weist macOS 26 als Mindestversion aus. `make ui-build` signiert das
lokale App-Bundle ad hoc, wenn keine `APPLE_SIGNING_IDENTITY` gesetzt ist, und
prüft anschließend die Bundle-Signatur. Das ist keine Developer-ID-Signierung
oder Notarisierung; lokale Dev-Pakete sind keine veröffentlichten Release-Pakete.

Der PKG-Payload installiert die App nach `/Applications`, Programme nach
`/usr/local/bin`, Vendor-Bundles nach `/usr/local/libexec/vzctl` und gemeinsame
Metadaten nach `/usr/local/share/vzctl`. Die App-Komponente ist nicht relocatable;
eine vorhandene Build- oder Runtime-Kopie darf das Installationsziel nicht ändern.

## Installer-Reihenfolge

1. `preinstall` beendet VM-Helper des Console-Users graceful, danach Supervisor,
   Edge und Net. Für Net gelten maximal 60 Sekunden. Kein SIGKILL und kein
   `kickstart -k`. Ein Timeout oder ein verbleibender Prozess einer anderen
   Sitzung verweigert den Austausch.
2. Erst nach erfolgreichem `preinstall` ersetzt Installer den Payload.
3. `postinstall` schreibt Runtime-Dateien und LaunchAgent-Plists mit der UID des
   Console-Users. Die Programme liegen bereits vollständig vor; Startreihenfolge
   ist Net → Edge → Supervisor. Jeder Dienst muss seinen Socket bereitstellen.
   Ein Startfehler wird als Fehler zurückgegeben. Der Job-PATH enthält Homebrew.

Installiert wird ausschließlich auf dem aktuellen Startvolume. Ohne Console-User
werden nur die globalen Dateien installiert. Die Hooks stoppen
keine fremden Nutzersitzungen; vorhandene Prozesse müssen vor dem Update beendet
werden. Eine fehlgeschlagene Nachinstallation kann bereits gestartete Dienste
zurücklassen und benötigt Diagnose; sie wird nicht als Erfolg ausgegeben.

`make test-packaging` prüft Payload, Versionskonflikte, qemu-Relokation,
Prüfsummen, Stop-/Startreihenfolge und Fehlerpfade in temporären Verzeichnissen
mit Fake-Binaries und Fake-Diensten. Native Signatur-, Installer- und
Live-Update-Abnahme erfolgt separat; diese Tests installieren nichts.
