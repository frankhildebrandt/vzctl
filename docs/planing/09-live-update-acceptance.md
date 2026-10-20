# V1: vorbereitetes Betriebsupdate und Abnahme

Stand 20.10.2026. Betriebsfreigabe vom Nutzer ausdrücklich erteilt: „Ja, Betriebsupdate jetzt durchführen“.
Dieser Plan führt keine Befehle aus.

## Bereits geprüft

- Lokale TAR/PKG/DMG unter `dist/`, Version `0.0.1`, Kennzeichnung `dev`.
  Paketinhalt, Prüfsummen, DMG-Integrität, festes Installerziel und
  ad-hoc App-/Daemon-Signaturen geprüft. Keine Notarisierung.
- Neuer nativer UI-Build direkt aus dem Buildverzeichnis geöffnet:
  Neti und IQ-Store zeigen alle Netze/VMs; Fit View und Stackwechsel funktionieren.
  Dabei keine Stack-YAML gespeichert.
- Nativer Purge-Dialog geöffnet und abgebrochen; keine Löschoperation.
  Der dabei gefundene initiale Fokus auf der destruktiven Schaltfläche ist
  auf „Abbrechen“ korrigiert; Enter-Abbruch ohne Mutation in Chromium/WebKit geprüft.
- Rust 395 Tests grün, Swift-Gesamtsuite ohne Compilerwarnungen,
  UI 149 Tests grün. Auch der CLI-Vertragsslice aus der [#17-Matrix](08-cli-release-contract-matrix.md)
  ist mit 395 CLI-Tests grün. UI-Abhängigkeiten: Audit 8→0, 14 Browserprüfungen grün.
- Der installierte Supervisor ist weiterhin der alte Stand. Debug-Routing,
  vollständige Health und Recovery sind damit noch nicht live abgenommen.

## Vor dem Eingriff

1. CLI-Vertragsslice und UI-Updates sind fertig und getestet. Ausführbare
   Artefakte werden neu gebaut, Paketprüfungen wiederholt; den bisherigen
   Prüfbau nicht als Nachweis für spätere Codeänderungen verwenden.
2. Freigabe für Unterbrechung und Wiederanlauf der sechs VMs liegt vor.
   Neti verbietet eigenständige Neustarts in seiner `AGENTS.md`.
3. Unmittelbar vor dem Update: aktive VM-IDs, LaunchAgent-Pfade, Binary-Hashes,
   Netz-/DNS-/Ingress-Status und Erreichbarkeit erfassen.
4. Private Sicherung der installierten Binaries, Plists und Konfigurationsdaten;
   SQLite konsistent über Backup-Schnittstelle sichern. Keine Secrets in
   Diagnoseausgaben. Vorhandene VM-Datenträger nicht löschen oder überschreiben.

## Update der bestehenden Entwicklungsinstallation

Der aktuelle Host verwendet `~/.local/bin` und `~/Applications/vzctl.app`.
Für diesen Host den vorhandenen `make install`-Pfad beibehalten. Ein paralleler
PKG-Wechsel nach `/usr/local/bin` würde einen zweiten CLI-Pfad hinterlassen und
ist keine saubere Abnahme derselben Installation.

- Installer stoppt alle VM-Helper geordnet, dann Supervisor → Edge → Net.
  Stop-Fehler abbrechen; Net niemals mit SIGKILL beenden.
- Installation mit `ACTIVATE=0`: erst sämtliche Binaries einschließlich Vendor/App
  austauschen, dann Net → Edge → Supervisor starten.
- Vorher aktive VMs gezielt wieder starten. Bei Neti zuerst `neti/main-node`,
  danach `neti/neti-home` und `neti/neti-external`; bei IQ-Store zuerst
  `iq-store/docker`, danach `iq-store/client-noble` und
  `iq-store/client-resolute`.
- Kein pauschaler Neti-Bootstrap: Er würde zusätzlich Anwendungen aktualisieren.
  Keine Purge-Operation und kein ungeprüftes vollständiges Apply auf bestehende
  Stacks.
- DNS-Bind-Helper und vorhandene Listener prüfen. Falls der privilegierte
  Helper ebenfalls aktualisiert werden muss, den dafür vorgesehenen
  `vzctl dns install-bind-helper`-Weg verwenden.

## Neti-DNS-Migration

Separat vorbereiteter Worktree: `/tmp/neti-dns-fix-20261020`.
Zehn Offline-Tests grün. `--dns-only --check` liest nur;
`--stage` erzeugt Backups/Kandidaten mit `PendingActivation`.

Vor Aktivierung muss Host-Bridge-DNS auf `.0` aus dem jeweiligen Gast erreichbar
sein. Kandidaten gegen den dann aktuellen Gastzustand prüfen, fremde
Hosts-Overrides nicht überschreiben. Erst im freigegebenen Fenster die geprüften
Dateien installieren und resolved aktivieren. Keine generierten `/run`-Dateien
überschreiben. Alle drei Gäste danach auf DNS und HTTPS-Ingress prüfen.

## Abnahme und Rückweg

- Sechs vorher aktive VMs wieder erreichbar; Guest-Agent- und
  projektspezifischer Health-Status vollständig und Ursachen nachvollziehbar.
- Native UI: beide Topologien, Debug-Quellen/Doctor, Health, Fehlerhistorie,
  vorhandene Logstreams und Bestätigungsdialoge prüfen.
- Wiederholter gleicher Edge-Sollzustand behält Caddy-PID. DNS-Bind-Reconnect
  im freigegebenen Fenster prüfen und danach alle bisherigen HTTPS-Einstiege.
- Bei Start-/Datenfehler stoppen, private Sicherungen bereithalten und nicht
  blind wiederholt installieren. Binary-Rückbau benötigt erneut geordneten
  Stop; SQLite-Rückbau nur bei nachgewiesener Schema-Inkompatibilität und
  gesichertem aktuellen Stand.
- Die echte PKG-Installation auf einem sauberen macOS-26-Testhost und der
  [Controller-Lab-Lauf](07-controller-lab-acceptance.md) bleiben eigenständige
  Nachweise. Ein erfolgreicher Entwicklungsinstaller ersetzt diese nicht.

Erst nach diesen Nachweisen V1/F17 und die betreffenden Issue-Kriterien
abschließen. Release-Version, Commit/Push und Veröffentlichung sind separate
Entscheidungen; derzeit wurde nichts veröffentlicht.

## Tatsächlich ausgeführt (20.10.2026)

Die freigegebene Unterbrechung ist abgeschlossen. Sechs VMs wurden geordnet
angehalten und wieder gestartet. Installation über bestehenden Entwicklerpfad
mit `ACTIVATE=0`; anschließend Net → Edge → Supervisor. Private Sicherung unter
`~/Library/Application Support/vzctl-update-backups/20261020T094019Z`, SQLite
über Backup-API konsistent gesichert (`integrity_check=ok`). Keine Datenträger
verändert. Der Root-DNS-Helper wurde über den vorgesehenen Admin-Dialog
aktualisiert; Edge hat seine Listener automatisch neu verbunden.

- Beide Bestandsstacks melden über CLI/REST/native UI `health=ok`, je 3/3 VMs.
- Neti-DNS auf allen drei Gästen aktiviert; interne/externe Auflösung erfolgreich.
- Nach Boot fehlende Neti-Routerregeln gezielt wiederhergestellt. F19 ergänzt
  einen eigenen aktivierten Restore-Service; dessen Start liefert Erfolg und
  erhält alle fünf Regeln. Echter erneuter Bootnachweis im isolierten Lab offen.
- F20 korrigiert Dienst-DNS für alternative Backendports: DNS bleibt auf Bridge
  `.0:53`, HTTPS geht auf den vorgesehenen Host-Service-Alias `.1`. Fünf Gäste
  liefern HTTP 200 mit erfolgreicher Zertifikatsprüfung. CLI-Fix installiert.
- Caddy-PID 96688 bleibt bei gleicher Konfiguration und DNS-Reconnect unverändert;
  zuvor vorhandener verwaister Caddy-Prozess ist beseitigt.
- F1/F25: echte REST→CLI-Argumentreihenfolge und Via-VM-Loopback-Vertrag
  korrigiert. Neti-YAML nutzt 127.0.0.1 für PostgreSQL via main-node. SELECT 1
  über REST und installierte native UI erfolgreich (exit0, status ok, eine Zeile).
  Doctor liefert im installierten Supervisor Erfolg; native Darstellung wird geprüft.
- Frischer nativer Start und Debug-Remount zeigen beide Netze/3 VMs. Ein zuvor
  leerer Graph bleibt als beobachteter Fall dokumentiert; Layoutfehler bei
  1100×740 unter F23 korrigiert, in Chromium/WebKit getestet und nativ installiert.

F21–F25 sind installiert. Neue Nacharbeiten F26–F27 stehen im Canvas. PKG-Abnahme auf sauberem Testhost,
K3s-/Controller-Liveabnahme sowie Auslieferungsentscheidungen bleiben offen.

## F26-Projektidentität: gezieltes Folgeupdate

Vorherige Binaries und konsistentes SQLite zusätzlich unter `pre-f26` im privaten
Backup gesichert. CLI/MCP, native UI, Edge und Supervisor aktualisiert; nur Edge
und Supervisor angehalten. Net, Root-DNS-Helper und VM-Helper unverändert weitergelaufen.

- CLI SHA256 `bfd0f9347504e15b4cff67f4eb943bae6601b686b56089d9d722feeef4eaf8ce`.
- Supervisor SHA256 `6233d0e7c7fd01a0a9bd3b1157642910ecc1df51dfb284fd32eabb85c9deb5f0`.
- Edge SHA256 `a7653ae0cd0138f7c03fe86b3a5a0298783181e8ead4b9952de8714253e3391f`.
- Fünf Legacy-Netzschlüssel und sieben Attachments unverändert erhalten;
  logische Netznamen korrekt migriert. Net-PID95742 und alle sechs VM-Helper-PIDs
  unverändert. Caddy beim Softwareupdate einmal neu gestartet (PID25030).
- Neti und IQ-Store health ok, keine Warnungen; fünf normale Gast-HTTPS-Aufrufe
  liefern HTTP200/TLSverify0.
- Lab-Resume passiert ensure_nets erfolgreich, DNS-Admin-Dialog noch offen.

422 Rust-Tests, 195 Swift-Einzelprüfungen (ein optionaler Skip) und 161 UI-Tests
bestanden; Builds ohne Compilerwarnungen. Finale Dev-Pakete separat dokumentiert
in [package-verification-2026-10-20.md](package-verification-2026-10-20.md).
F27 ist darin noch nicht enthalten. Nach Anlage neuer projektbezogener IDs kein
blinder Rückbau auf alte Binaries oder alte SQLite-Kopie.

## Lab-Vorprüfung und CLI-Folgeupdate F27/F29

DNS-Adminfreigabe bestätigt. Lab passierte Netz/DNS/CA, begann anschließend Bake,
weil Tag v2 trotz existierender Raw-Datei nicht registriert war. Die Vorprüfung
hatte diese Registrierung nicht geprüft. Nur der Lab-Apply/Bake/Builder-Prozessbaum
wurde mit SIGTERM gestoppt; sechs Bestands-VMs, Net-PID und Attachments unverändert.
Keine Shared-Image-/Aliasdatei wurde ersetzt. Journal kontrolliert abgebrochen.

Lab und Generator pinnen jetzt registriertes v1 mit vorhandenem Raw und gültigem
Sealmarker. Zusätzlich verhindert F29 den automatischen Neubau eines gültigen
Pins allein wegen älterer oder fehlender Gastwerkzeug-Version; Updates erfolgen
über Guest-Utils. F27 berücksichtigt optionale iwatch-Bytes im Bundle-Cache.

Unabhängig geprüft, 16 Image-/Reuse- und 12 Guest-Utils-Tests erfolgreich. CLI/MCP
am 20.10.2026 atomisch ersetzt; alte Binaries privat unter `pre-f29-cli` gesichert.
Installierter CLI-SHA256: `3ad7e2ca6a66edbee19c2016e9ee85ac27786249f28247b8d6e6fbdae3b19dcf`.
Signaturen geprüft. Kein Daemon-/VM-Neustart. Neuer Bootstrap folgt mit frischem
Ressourcengate, ohne Force und ohne Resume über geänderte Konfiguration.

## Native Restabnahme und K3s-Shellfix F30

Native UI-F28 installiert (Binary-SHA256
`d2b43e27f93d54dd7754cf0adc862d1eb482cc580165f147f4199b0bb935f61b`).
Debug zeigt erfolgreiche Voraussetzungen und SELECT 1; beide Topologien samt
Fit/Stackwechsel und journald-Follow erfolgreich. F3 war eine Hintergrund-
Renderpause und ist im Vordergrund ohne Graphfix aufgelöst;
[ausführlicher Nachweis](ui-native-rendering-2026-10-20.md). V1 abgeschlossen.

Korrigiertes Lab-Apply hat das Image in 3 ms ohne Bake übersprungen; drei Gäste laufen.
F27 live: iwatch 0.3.0 ausführbar, Bundlemanifest persistent. K3s stoppte kontrolliert
mit unvollständigem AWK-Zitat vor der eigentlichen Installation. F30 behebt dies;
neun Tests mit realem sh/Syntaxprüfung plus unabhängiger Review erfolgreich.
CLI atomisch installiert, alte Binaries unter `pre-f30-cli` gesichert; SHA256
`e70fcf77e08bcadb13c14b2de42917d6ba3e602742a7341a7e27e2e8666b8e10`.
Unverändertes Bootstrap-YAML wird per --resume fortgesetzt. Provider noch nicht gestartet.

## K3s-API und CRD-Versionskonflikt F31

Nach F30 sind Server und Worker mit `v1.37.1+k3s1` bereit; die erste CRD
ist etabliert. Der Bootstrap stoppte anschließend an einem Versionskonflikt
zwischen CRD-Erstellung und Server-side Apply. F31 erlaubt höchstens drei
Apply-Versuche ausschließlich bei diesem Konflikt. Jeder erneute Versuch prüft
Besitzer, unveränderte UID und aktuelle ResourceVersion; UID/RV stehen auch im
Apply-Payload. Kein Force, kein Retry bei anderen Fehlern.

Zwölf gezielte Tests und unabhängiger Review erfolgreich, Release warningfrei.
CLI atomisch installiert, Vorgänger unter `pre-f31-cli` gesichert; SHA256
`d52df017f32c142b6c781a7d2fe115d8c22b67afe79b0cb0f0e3c74257c26e22`.
MCP unverändert. Kein Daemon- oder VM-Neustart. Bootstrap-Resume mit unveränderter
Konfiguration läuft; Provider- und vollständige Controller-Abnahme stehen aus.

Der erste Resume hielt am UID-Schutz an: Der echte `kubectl get` lieferte
ein einzelnes vorhandenes Objekt als `kind: List`; dessen UID liegt im Item.
Die strikte Normalisierung von leerer/einzelner Liste ist ergänzt, mehrdeutige
oder unpassende Objekte werden weiterhin abgewiesen. RED-Reproduktion und
13 gezielte Tests plus unabhängiger Review erfolgreich. CLI-Folgeinstallation
`4965bc97aed24998443d1ca9a1ee078f17bcdcd2e1b098dc8de073034a23b191`,
MCP `ac5fdc294ad7242d3a9ebddf8ad367018a5e8451b81cabf496a93fb85c742850`;
Vorgänger privat unter `pre-f31-list-cli`. Erneuter unveränderter Resume läuft.

F31 Live-Abnahme erfolgreich: Resume Exit 0 in 9 s, beide Nodes Ready mit exakter
gepinnter Version, vier CRDs Established und Namespace app Active. Private Belege
unter `controller-lab/evidence/bootstrap-ready.json` und zugehörigen API-Antworten.
Die separate CLI-Statusprojektion wird unter F32 nachgebessert; API-Abnahme ersetzt
nicht die noch offenen Consumer-/Provider- und Wiederholungsprüfungen.

## F32/F33: Agent-Verträge und Router-Projektgrenze

Unabhängig reviewed und integriert: 438 Rust-Tests bestanden, ein Test ignoriert;
199 Swift-Einzeltests bestanden, ein optionaler DNS-Labtest übersprungen. Alle
Tests und Releasebuilds ohne Compilerwarnungen.

Installiert:

- CLI: `cb7d6acef71754d7ecad875414346b22d85848781fb69abc2f4a334107d629e8`
- MCP: `91dd9f7cbfa25252f5a775a2fb8db85d300505ee101bda9b5f88c4e12732408c`
- Supervisor: `f7080a1d1650c836e17e94518a35a51e300ac4dad732db18df7da3123b8f2a19`

Private Sicherung `pre-f32-f33` enthält die Vorgänger und eine konsistente
SQLite-Kopie. Ausschließlich Supervisor gestoppt/gestartet; neue PID 48933 und
erfolgreiche REST-Health mit DB/Net/Edge geprüft. Alle neun VM-Helper, Net,
Edge und Caddy behalten ihre PIDs. Beide Bestandsstacks anschließend Exit 0,
Health `ok`, null Warnungen.

Echtes MCP-stdio erfolgreich mit read-only `controller_status`,
`kubernetes_status` und `diagnose(stack_path)`: Projekt, aktivierte Registry,
Generation, zwei Ready-Nodes mit exakter Version und vorhandene Controller-/
K3s-Diagnoseabschnitte geprüft. Das Bootstrap enthält noch keine Claims;
öffentliche Bindings realer Provider werden separat unter K7 abgenommen.

F33 wurde zuvor im read-only Routeplan gefunden: globale Routerauswahl hätte
Neti-Policies entfernt. Nach gemeinsamer CLI-/CP-Installation enthält der
configbasierte Plan nur `controller-lab/ops`. Lab-Routen/Policies wurden in
den drei privaten Configs konsistent ergänzt; Apply Exit 0. Neti-Regelhash,
Net-PID und acht geschützte VM-PIDs unverändert. Die native Lab-Topologie zeigt
korrekt eigene Netze und IPs.

F19 hält derzeit **vor** dem geplanten ops-Neustart: Die Docker-DNS-Vorprobe
meldet einen Auflösungsfehler. Regeln sind gespeichert und Restore-Service
aktiviert, aber Bootnachweis und Provider bleiben offen. Private Evidenz:
`controller-lab/evidence/f19-live-20261020T113247Z/`.

## F34: Bootstrap-Firewall blockiert erlaubten Verkehr

Die Vorprobe ist inzwischen eingegrenzt: Container-DNS zeigt korrekt auf
`10.123.0.0`. DNS und TCP werden durch eine zusätzliche leere
`ip/filter/FORWARD`-Chain mit Policy Drop blockiert. Sie stammt aus der eigenen
alten Router-CloudInit-Zeile `iptables -P FORWARD DROP`; die korrekten Freigaben
in `inet/vzctl` können eine zweite Drop-Chain nicht aufheben.

Der Fix nutzt die eigene Tabelle schon beim Bootstrap. Die Migration verlangt
den exakten Herkunftsnachweis im geschützten CloudInit-Skript und eine leere
Standard-Chain; Entfernung und neue Regeln erfolgen gemeinsam in einer
nft-Transaktion. Fremde Regeln dürfen dabei nicht geändert werden.

Der unabhängige Review hat zusätzlich eine
Bootlücke bestehender VMs gefunden: Neue CloudInit-Guards erreichen diese nicht,
während Forwarding bisher vor dem späten Restore aktiviert wurde. Dieser
Bestandspfad ist mit persistentem Forwarding-Default `0` und privilegierten
Runtime-Start-Guards abgesichert. Zwölf Go-, ein CloudInit- und zwölf
GuestUtils-Tests grün; unabhängiger Review abgeschlossen, Release warningfrei.
CLI atomisch installiert, SHA256
`ecb79a4da783270bff9609d3d8569dcb30646bc490ef1d98779b9558638fdf8f`;
MCP unverändert, Sicherung `pre-f34-cli`. Kein Daemonneustart.

F34 live: Apply ausschließlich im Lab erfolgreich. Docker-DNS/HTTPS mit
TLS-Prüfung, erlaubtes TCP 6443 und gesperrtes TCP 22 zur gleichen Server-IP
erfolgreich geprüft; der SSH-Listener war separat nachgewiesen.
Neti-Regeln und die acht geschützten VM-PIDs unverändert.

## F35: Helper-Lock-Rennen beim isolierten Neustart

Der anschließende `vm restart controller-lab/ops` meldete Erfolg mit `starting`,
aber der neue Helper scheiterte am noch gehaltenen Lock des alten Helpers.
Der alte Prozess meldet `stopped` vor seinem endgültigen Cleanup; die bisherige
CLI wartet nur auf den Status. Der F19-Runner beendete sich nach seinem
begrenzten Bereitschaftsfenster mit Exit 1. Evidenz:
`controller-lab/evidence/f19-live-20261020T115440Z/`.

Nach bestätigtem Ende des alten Helpers wurde ausschließlich ops einmal
gezielt gestartet. Wiederanlauf erfolgreich, PID 59137; neue Boot-ID und
Router-Restore `active/exited`, `Result=success`, `ExecMainStatus=0`.
Die anderen acht VMs blieben mit unveränderten PIDs aktiv. Dies ist eine
Wiederherstellung, noch keine bestandene F19-Neustartabnahme.
Nach dem Wiederanlauf zusätzlich geprüft: aktive eigene Regeln und gespeicherte
Dateihashes identisch, Neti-Regeln unverändert; Docker-DNS/HTTPS sowie erlaubtes
TCP 6443 und gesperrtes TCP 22 weiterhin erfolgreich. Privater Nachweis unter
`controller-lab/evidence/f35-recover-ops/`.
Der gemeinsame Supervisor-Lifecycle wird unter F35 korrigiert und unabhängig
geprüft. Provider bleiben aus.

## F36: Plan klassifiziert neue Container zu streng

Die Vorbereitung des vollständigen Lab-Apply hat einen weiteren Vertragsfehler
erkannt: Das erste Hinzufügen des Containers und seiner Bindings markiert eine
ansonsten unveränderte VM als destruktives Update. FullApply wurde nicht
ausgeführt; kein `--force` verwendet. F36 korrigiert die Klassifikation mit
gezielten Regressionen. Tatsächliche Recreates und Änderungen an bestehenden
Consumer-Bindings bleiben geschützt.

## Externe Installer-Abnahme

Der Nutzer bestätigt, dass kein separater macOS-26-Test-Mac verfügbar ist.
Saubere PKG-Erstinstallation und anschließendes PKG-Update in #76 bleiben
als externe Abnahme offen. Lokale Paketprüfungen und Entwicklungsinstallation
ersetzen dieses Kriterium nicht; die übrige Arbeit wird fortgesetzt.

## F35/F36 Installation und F35-Zeitlimit

F35/F36 installiert nach 445 Rust- und 205 Swift-Tests (je ein ausgewiesener
Skip), unabhängigen Reviews und warningfreien Releases. CLI SHA256
`634b3f97c85d3aa5a6f2bcc4e0ec22d3d84dbe3d076ee50b94a721e0f427080e`,
Supervisor `2f9c5b6b768bc185415dc79b5a149a39c5402ff5b34ae7602de5f58c077f6d02`.
Nur CP neu gestartet (PID63903), neun VM-PIDs sowie Net/Edge/Caddy erhalten.
Sicherung `pre-f35-f36`.

F19-Lauf `f19-live-20261020T121425Z` passiert Apply und alle Netzwerkproben,
scheitert aber beim Restart mit CLI-Exit10 / `os error35`. Der neue Helper
startet tatsächlich erfolgreich (ops64171); die anderen acht PIDs unverändert.
Ursache: CLI-Socketfrist5s ist kürzer als Supervisor-Freigabe+Claim-Budget10s.
F35 bleibt offen, beide Rust-RPC-Clients werden auf20s abgestimmt; verzögerte
UDS-Antwort nach5,5s reproduziert den Fehler im Rot/Grün-Test. Kein zusätzlicher
VM-Start nötig. FullClaims und Provider weiterhin nicht aktiviert.

## F19/F35 bestanden; Provider-Erstversuch angehalten

CLI-Zeitlimitfix e54457bf installiert mit Sicherung `pre-f35-timeout-cli`,
kein Daemonneustart. Reguläre Neustartabnahme erfolgreich:
`f19-live-20261020T122012Z/report.json`, SHA256
`658d9fba5460f1b583545ce5b6ecdd5cf014a1ca9560be5d9577b45de2952db0`.
Neue Boot-ID, aktive/gespeicherte Regeln identisch, Restore aktiv/enabled/success.
DNS/HTTPS und Allow6443/Deny22 vor/nach Boot bestanden; acht geschützte VM-PIDs,
Net-PID95742 und Neti-Regeln unverändert. F19 und F35 abgeschlossen.

Provider-Vorprüfung bestand, Setup startete ausschließlich in ops, endete aber
sofort mit Exit127. Keine Go/MinIO/mc/venv-Meilensteine erreicht. Runner stoppte
kontrolliert und entfernte Adminstaging. Evidenz
`provider-live-20261020T122138.112480Z/failed.json`. Kein automatischer Retry;
Teilzustand und fehlendes Programm werden read-only geprüft. FullClaims aus.

Read-only-Diagnose: alle benötigten Programme und PATH vorhanden, aber
Guest-Agent `PrivateTmp=yes`; Setup-Dateien liegen in dessen privatem `/tmp`.
Der transiente systemd-Dienst im PID1-Namespace sieht sie nicht. Kein Setupschritt
erreicht, `/opt/vzctl-lab`, `/var/lib/vzctl-lab`, `/etc/vzctl-lab` fehlen.
Korrektur verwendet privates gemeinsames Staging außerhalb von `/tmp`;
Agent-Isolation bleibt erhalten. Wiederaufnahme muss diesen konkreten fehlgeschlagenen
Bericht und den unveränderten Teilzustand prüfen.

## F37: gemeinsames privates Staging bestätigt

Korrektur und eng gebundene Recovery unabhängig geprüft; 32 Offlineprüfungen
und 7 Shellsyntaxprüfungen grün (`/tmp/vzctl-provider-recovery-offline.log`).
Setupasset SHA256 `b70759cad866523a6fa9dd523987672b736923eb743d1e6026a4ef6e50d14e52`
atomisch ins Lab übernommen; altes Asset unter `evidence/f37-provider-asset-before.sh`
gesichert. Aktive YAML unverändert.

Recovery mit Runner SHA256
`f411525070a7a22917adc4f0974177ea85615b48bb394c3e1d44728eff1fb466`
besteht alle Vorprüfungen. Gemeinsames `/run/vzctl-lab-provider-stage` mit
0700/Admin0600 sowie Sichtbarkeit/Assethashes aus Agent und PID1 bestätigt.
Die neue Unit `vzctl-lab-setup-recovery.service` läuft. Evidenz
`provider-live-20261020T123107.555864Z/`. F37 abgeschlossen; Providerbereitschaft
ist noch separat abzunehmen, FullClaims weiterhin aus.

Die Recovery-Unit endete nach20s mit Exit1, alle vier Build-Meilensteine
noch pending. Runner hat Unit beendet und Adminstaging entfernt (beides bestätigt).
F37-Sichtbarkeit ist bestanden; die spätere Installationsursache und der Teilzustand
werden separat read-only geprüft. Kein automatischer erneuter Lauf.

## F38: Go-Build unter systemd ohne Login-Umgebung

Read-only-Nachweis zeigt PostgreSQL installiert/aktiv, Go1.24.2 korrekt entpackt
und MinIO-Source vorhanden. Das Go-Archiv hat exakt den offiziellen SHA256
([Go-Downloads](https://go.dev/dl/)). Die unprivilegierte Fortschrittsprobe meldete
Go irrtümlich pending, weil dessen Elternverzeichnis root0700 ist.

Der erste Go-Build scheitert ohne HOME und ohne GOCACHE; lokal mit identischem
Setup-Export reproduziert. Die Live-Unit hat keine Login-Umgebung. Fix verwendet
explizite private GOCACHE/GOPATH/GOMODCACHE unter `/var/lib/vzctl-lab`, ohne HOME
zu überschreiben. Progress liest gezielt privilegiert. Geprüfte Fortsetzung nur
ab dem Build-Teilstand; apt und PostgreSQL-Initialisierung nicht blind wiederholen.

F38 gezielte Fortsetzung gestartet nach unabhängigem Review und17 Offline-/
5 Syntaxprüfungen. Runner SHA256
`8bce4a0c88107c39a1c421635c279fc152346738402b8e104770075a7593832a`,
Recoveryasset `d99885215e50d8b8aa8ee517028a04bd4ef39d31344a8a67015d286d9821cf0a`.
Vorher vollständiger MinIO-Quellbaum, Go-Binary/Archiv, initialer DB-Zustand und
beide Fehlversuche exakt geprüft. Historisches Labasset b707 bleibt erhalten;
der Generator für neue Labs enthält den Cachefix. Binaries werden explizit0755
und vor Secret-/Servicekonfiguration als `vzctl-agent` geprüft.
Neue begrenzte Unit `vzctl-lab-setup-build-recovery.service`, Evidenz
`provider-live-20261020T124243.973565Z/`. Go korrekt present, Build läuft.
Providerbereitschaft weiterhin offen.

Nachfolgende Runner unabhängig geprüft, noch nicht ausgeführt:
FullClaims d495f0e61e4eed65d984667734fdb48d3779e82287530f36a8f81a7d38f38e8a
(30 Offline-/6 Syntaxchecks); MCP03909b027a95c1a6687e663ca5a096f0b4ea822aa8e22c8026f864842e3a2c7e
(43 Checks); Repeat/Late65257c065a6c15aea879b87db659eaf9249059baa8f4c45028b475f9ea83bd65
(33 Checks). Reihenfolge Full → MCP mit5Claims → Repeat → Late mit9Claims.

F38 bestanden: Build-Recovery Exit0, PostgreSQL16.15 und MinIO bereit, Endpunkte
geprüft, alle9 VM-PIDs/CP unverändert. Adminstaging aus beiden Namespaces entfernt.
Erfolgsbericht `provider-live-20261020T124243.973565Z/result.json`, SHA256
`6183236628cde595e6d09b3a36b39ac9488cf8ba4df003ad452c6a2ed50e124d`.
FullClaims-Abnahme mit geprüftem Runner d495f0e gestartet.

FullClaims-Lauf `full-live-20261020T124444.616124Z` bestand exakt15 nonbreaking
Planaktionen vor Aktivierung. FullYAML aktiviert und genau ein Apply gestartet;
danach Runner angehalten mit `command_failed`. Kein automatisches Resume/Rollback.
Read-only Status: db/docker-web/kube-web/login Ready, files Degraded mit
ProviderUnavailable (CLI24). K7-Registry aktiviert, Generation
`7f6d2d99-b2c4-43ba-914e-2446bcf7e8a7`. S3-Controllerzugriff wird untersucht,
Consumer- und MCP-Abnahme noch offen.

## F39: MinIO-Serviceaccount-Secret zu lang

Read-only echte Adminoperationen unter Agent-UID999 funktionieren. Lab-Bucket
existiert mit exaktem Owner `vzctl:controller-lab/files`; Serviceaccountliste leer.
Flagreihenfolge, HOME und Aliasformat als Ursache ausgeschlossen. Exakter
[MinIO-Pin](https://github.com/minio/minio/blob/0d7408fc9969caf07de6a8c3a84f9fbb10a6739e/internal/auth/credentials.go)
begrenzt `CreateNewCredentialsWithMetadata` auf40 Zeichen, während der
SecretStore64Hex liefert. Fix soll vorhandene Seeds stabil ableiten und gültige
Werte erhalten, keine Accounts/Secrets rotieren. Gezielte Regression und Review
vor CLI-Installation; anschließend eng geprüfte Fortsetzung des bestehenden
Apply-Journals. Historische F19-/Providerbelege behalten ihre ursprünglichen Hashes.

F39 abgeschlossen: fünf gezielte S3-Tests (inklusive echter 40-Zeichen-Grenze als
Rot/Grün-Regression, unverändertem Seed und stabiler Wiederholung), unabhängiger
Review ohne Blocker. Warningfreier CLI-Release in14.09s, Signaturprüfung grün.
CLI SHA256 `f25c6228c098e03d29acd02b267deb0d358259aca02fee3507ae855d655401b7`
atomar installiert; alter Stand unter `pre-f39-cli` gesichert. MCP/CP unverändert,
kein Daemon-/VM-Neustart. Periodischer K7-Loop meldet danach selbstständig alle
fünf Claims Ready, warnings0, einschließlich erfolgreicher S3-Anlage. Registry
bleibt `7f6d2d99-b2c4-43ba-914e-2446bcf7e8a7`, lastSuccess1792501181.
Kein manueller Reconcile; FullApply-Journal und Consumer-Proben noch offen.

## F40: mc-Ausgaben erst nach vollständigem Puffern begrenzt

Read-only bestätigt: `subprocess.run(stdout=PIPE, stderr=PIPE)` sammelt beide
Streams vollständig. Erst danach wird stdout gegen1MiB geprüft; stderr hat
keine Größenprüfung. Zeitlimits begrenzen nicht den benötigten Gast-RAM.
Gemeinsame Grenze beim Lesen und Bereinigung der Prozessgruppe implementieren,
mit echten lokalen stdout-/stderr-Flut-, Timeout- und Secretmarker-Proben.
Fehlerdiagnose nur um feste, in Rust validierte Operationstokens ergänzen.
Bestehende Fehlercodes, Owner- und Credential-Regeln erhalten. Noch keine
Installation; aktuelle Full-Abnahme bleibt auf F39-CLI f25c6228.

F39-Fortsetzung zunächst vor Evidence-Anlage und Resume angehalten:
`active_stack_lease`. Read-only SQLite zeigt ausschließlich Lab-Lock
`localhost:72273`, abgelaufen um12:50UTC (Prüfzeit13:05UTC). Die alte
Vorprüfung zählt alle Zeilen statt aktiver Leases. Eng gebundene Korrektur
mit Prüfung des beendeten Holders; keine manuelle Locklöschung.

F40 Code/Tests fertig: sechs S3-Tests mit warnings-as-errors grün, darunter
echte stdout-/stderr-Flut, gemeinsamer600KiB+600KiB-Grenzfall, Timeout nach EOF
und SIGTERM-ignorierender Enkelprozess. Unabhängiger Review ohne Blocker.
Release und Installation folgen nach der laufenden Full-Abnahme.

Lease-Gatekorrektur unabhängig geprüft,38 Offlinechecks grün; Runner
`5facd5b05e885dcbb989e0062895bc98305dfa598a09f7e3c9695dbb4d84e5a5`.
Erneuter Aufruf vor Evidence-Anlage/Resume mit `host_memory_pressure` angehalten.
Read-only: macOS Pressure2, memory_pressure-Q49%, etwa26GiB komprimierter
Speicher. Nach Buildabschluss erneut Pressure2. Keine Apps/VMs beendet; Nutzer
um Schließen entbehrlicher Anwendungen gebeten. FullApply bleibt unverändert.
F40-Release4555d1c4934bcfad8d7adde8c30c1e57f14932ccecf28b99e60cf07d57ea7335
warningfrei gebaut und signiert, noch nicht installiert.

Nach Nutzer-Rückfrage Speicherbewertung korrigiert: Sysctl liefert Dispatch-Stufe2
(Warnung), nicht kritisch4; memory_pressure-Q zeigt45–47% verfügbare/pagebare
Reserve, nicht ausschließlich unbenutzten RAM. Compressor rund27–28GiB, komplett
freie Seiten etwa1GiB; aktuell Swapusage0, kein Swap-I/O. Normal-only war eine
Vorsichtsgrenze des ursprünglichen VM-Aufbaus. Für Resume/Repeat/Late in exakt
denselben laufenden VMs wird1/2 mit tatsächlichem Messwert dokumentiert zugelassen;
4/Unbekannt bleibt gesperrt. Keine RAM-/VM-Erhöhung und kein automatisches
Schließen fremder Anwendungen. Quellen: Apples kern_memorystatus_notify.c
(Sysctl wandelt intern auf Dispatch-Flags um) und doc/vm/memorystatus_notify.md.

## F41: fehlgeschlagenes CLI-Apply in der nativen Stack-UI nicht sichtbar

Lesende native Lab-Abnahme: zwei Netze, drei VMs und Seitenwechsel korrekt.
Betrieb zeigt3/3 running und15 offene Änderungen, aber kein gespeichertes
fehlgeschlagenes Apply-Journal und keine Ursache. Fehleransicht zeigt nur
Sessionfehler. API-/UI-Abgleich und gezielte Regression in Arbeit; kein
automatisches Resume. Doctor12ok/2warn/0fail, Warnungen zunächst dokumentiert
(DNS-Hostport bereits belegt, ein unerreichbarer Dockercontext).

Geprüfte Memory-Anpassung: Continuation9d30e0a029b8d2d2f6bb73691bc6c97d346f379a89611e32af71e930c56bc50d,
Followup4250d2aef3e9c580b845eaa2ef0cc6a6681643a52121444350239d374759ef71.
26 Memory-,38 Continuation- und33 Followup-Offlinetests grün, unabhängiger Review.
Historischer Providerhelper unverändert.

Resume tatsächlich ausgeführt: `full-live-f39-20261020T131635.139206Z`.
Generation7 passiert `ensure_service_claims`, scheitert um13:16:38UTC in
`ensure_containers`: Docker inspect proof → SSH Exit255/connection refused.
Neue Lease localhost:6489 bis13:21:38UTC. Kein automatischer Retry/Rollback;
Docker-SSH-Kontext/Forward werden lesend diagnostiziert.

## F42: Docker-SSH kollidiert mit HTTPS-Ingress-DNS

Gastseitig ssh.service/ssh.socket aktiv, Port22 lauscht und Docker läuft;
F19-Boot-ID unverändert. Hostauflösung von docker.svc.controller-lab.vz.test
zeigt127.0.0.1. Der Name ist zugleich festes Docker-SSH-Ziel und frei
deklarierter HTTPS-Ingresshost. DNS-Horizon priorisiert den Ingress korrekt.
Keine SSH-Portweiterleitung vorhanden; SSH landet deshalb am Hostport22.
Unabhängiger Code-/Livenachweis durch Apply/Edge. Produktkorrektur soll
Management exakt an die eigene Docker-VM binden und mehrdeutige Auswahl
ablehnen. Kein Ändern des Lab-YAML/Resumehashes als Workaround.

F41 lokal geprüft:13 UI-/Recoverytests, Typecheck und fünf Swift-Tests mit
Warnungen als Fehler grün. REST-Integration nutzt persistiertes Journal nach
DB-Reopen und prüft kanonische Stack-ID sowie unveränderte Datenbank. Nur
begrenzte Statusfelder, keine Payloads/Resources; unlesbare Erhebung wird
sichtbar. Aktive/unlesbare Lease blockiert Recovery; Abort bleibt bestätigt.
Unabhängiger Review ohne Blocker, keine DB-Migration. Release/Installation
und native Nachprüfung offen.


F40/F41/F42 gezielt installiert: CLI c64c2eea9a8740dc7919b77f2502edde67ba64f64956fa46c6433b69bf76d883,
Supervisor096a293bee58112839cf36e5e9163eea1628d3318a2881d99dacd9f684353a4c.
F42: 24 Docker-Tests mit Warnungen als Fehler, echte SSH-Konfigurations- und
UDS-Auswahlregressionen; unabhängiger Review ohne Blocker. Builds warningfrei,
Signaturen strict geprüft. Konsistente SQLite-/Binarysicherung in pre-f40-f41-f42;
keine DB-Migration. Nur CP neu gestartet, jetzt PID10852. Alle neun VMs vollständig
mit denselben IDs/PIDs/running registriert; Net/Edge/Caddy unverändert (12 Prozesse).
MCP und Full-YAML unverändert. F41-UI-Installation und native Nachprüfung folgen.


F41 UI installiert: Executable0c689c0fbf6de33e4670418cff14c6255170e0a47ab78bf50904044f167e2e59,
alte App gesichert, Signatur geprüft. Nativ bestätigt: „Letzter Vorgang fehlgeschlagen“,
Docker/SSH-Connection-refused-Ursache, Schritt ensure_containers, Journalf0e0c5dd…,
Lease localhost:6489/PID6489/Expiry13:21:38Z. Abgelaufene Lease erlaubt bewusst
angebotene Resume-/Abort-Aktionen; nichts ausgelöst. Drei Lab-VMs running.
Aktive-Lease-Sperre und Abort-Modal durch gezielte Regressionen geprüft; kein
künstlicher nativer aktiver Lock erzeugt. F40/F41 abgeschlossen.


## F43: falscher Kubernetes-Drift durch API-Defaults in Arrays

F42-Fortsetzung hat Apply und Journal done passiert. Fünf Claims Ready,
warnings0, Pod proof Running/ready. Verbraucherabnahme wartet dennoch auf
Kubernetes-Status: Deployment proof und Service proof melden ResourceDrift/Exit24.
Lesender rekursiver Vergleich aller deklarierten Werte zeigt0Abweichungen.
API ergänzt imagePullPolicy/terminationMessage*, Portprotokolle, Probe-Schwellen
und defaultMode innerhalb von Arrayelementen. matches_desired rekursiert bisher
nur Objekte, Arrays werden vollständig verglichen. F43/#79: enger rekursiver
Arrayvergleich mit Tests für echte Änderungen und Länge/Reihenfolge/Typen.
Kein weiteres Apply für das bereits fertige Journal. Historische Belege bleiben
unverändert; Consumer-Nachlauf wird an fertiges Journal und CLI-Wechsel gebunden.


Ownership-/Ausfallrunner vorbereitet, noch nicht ausgeführt:
`/tmp/vzctl-controller-ownership-fault-live.py`,
SHAf73953e2cc9d1fc33783befd4d953f0505f93b32f7374a1c0c600c9754044ab4.
31 Offlinechecks, 11 ausgeführte Restore-Skriptfälle mit gefaktem kubectl und
6 Vorgänger-/Reihenfolgeprüfungen grün; unabhängiger Review ohne Blocker.
Timer/finally prüfen dieselbe Secret-UID, Datenhash, Owner und ResourceVersion.
CR-Cleanup bindet die ursprüngliche CREATE-UID im privaten root0600-Gastbeleg;
fehlender Beleg erlaubt keine Löschung nach Name. Foreign-DB-Fall ausschließlich
nach drei gepinnten erfolgreichen Fällen, danach keine weiteren Ausfallfälle.
Unklare Erstellung ohne UIDbeleg bleibt für kontrollierte Recovery erhalten.


F43 Code und Installation: sechs Kubernetes-CLI-Tests mit Warnungen als Fehler,
echte Rot/Grün-Reproduktion für Deployment/Service und acht negative Driftfälle
bestanden. Unabhängiger Review ohne Blocker. CLI10e1d03f502c5e740608d2b18d30a0832616a4304dbf9f0c8ea99964169d82df
warningfrei gebaut und strict signaturgeprüft. Neues Binary vor Installation
read-only: Kubernetes exit0/ready/issues[]. F42-Runner regulär am13:42:28UTC
beendet; failed.json SHA977b73827e7505bc6c5f584be3b93791ddca37159564374eea54431f2ac72d95,
Journal Generation7/done/done, keine Locks, noch keine Consumer-Probes.
CLI anschließend gezielt installiert, Sicherung pre-f43-cli; MCP/CP unverändert,
13 Laufzeitprozesse einschließlich neun VMs unverändert. Kein Daemonrestart.
Consumer-Nachlauf ohne Apply/Resume wird vor Ausführung unabhängig geprüft.


Consumer-Nachlauf36fb91873a1197e7d83ec79ab5c0ea81c3b0406b7ca7e4fd5d1a8c47c32dbb24
(15 Offlinechecks und unabhängiger Review) einmal ausgeführt, früh mit
command_failed angehalten: full-live-f43-20261020T134357.172025Z/failed.json,
SHAae6487653c63df6f3f71881a48c677a2000af35c24e52a2257c91945626c03f6.
Keine Apply-/Resume-Aktion; nachfolgende Sicherheits-/Consumer-Prüfung wird
lesend eingegrenzt. Kein erfolgreicher Full-Consumer-Beleg; MCP/Followups warten.

Consumer-Folgefehler eingegrenzt: Docker proof restarting/Exit1, RestartCount19;
Code/CA0644, /opt/vzctl-lab0755, einzelne Bindingdirs0755 und Dateien0444 korrekt.
Privates Elternverzeichnis0700 bleibt unverändert. Logs stdout/stderr leer, da
Pip-Ausgabe in /tmp/pip.log liegt; vorhandenes Log wird begrenzt und ausschließlich
mit festen Fehlerklassen ausgewertet. Noch keine bestätigte neue Produktursache.


## F44: DNS-SD-Owner von CNAME-Antworten verloren

Pip-Fehlerklasse Name-or-service-not-known; OOMKilled=false. Code, requirements
und CA identisch zum bereiten K3s-Consumer. Docker-Sicherheitscheck scheitert
bereits an container_restarting, keine UID-/Readonly-Assertion erreicht.
Edge liefert CNAME files.pythonhosted.org→dualstack.python.map.fastly.net,
aber A-Records tragen fälschlich den Alias-Owner. Öffentlicher Resolver liefert
dieselben IPs unter dem korrekten kanonischen Owner. Gast getaddrinfo EAI_NONAME.
Unabhängige Quellprüfung bestätigt: SystemDNSResolver verwirft fullname,
SystemDNSRecord enthält keinen Owner, Serializer schreibt jeden RR mit0xC00C.
Numerischer Upstream unverändert, betroffen upstream=system. F44/#80 mit engem
Owner-Erhalt und Wire-Regressionssuite; kein Resolver-/Rechte-Workaround.
Redigierter DiagnosebelegSHA5eeb78add8e0c6746c9417ea219ff565b2b9b85c471c7ad40a77197d3aed7c82.
Geplanter Edge-only-Installer sichert Altbinary und belegt Edge/Caddy-PID-Wechsel;
neun VM-PIDs, Net, CP sowie CLI/MCP/CP-Binaries müssen unverändert bleiben.
Nächster Consumer-Nachlauf ohne Apply bindet diesen Wechsel gesondert.


F44 Release/Installation: 65 VzEdge-Tests mit Warnungen als Fehler grün,
echter RED belegt vier falsche Owner. Unabhängiger Review einschließlich
DNS-SD-Literalescapes/Encodergrenzen ohne Blocker. Edge8fd8455c19aaed9137819b11097530e6968cc061648235ba5e8010bfc5aa5a10
warningfrei gebaut und strict signaturgeprüft. Erste Installervorprüfung stoppte
vor Backup/Mutation wegen veraltetem Caddy25030-Pin. Historischer Prozessbeleg
pre-f40-f41-f42/processes-after.json (SHA762f19c338477b2c3e175b9d6cd36a51ddd76c3e8e7c07449706746642ccd0df)
belegt bereits Caddy72413; Pin korrigiert und Belegbindung ergänzt, erneut reviewed.
Finaler InstallerSHA9727e032e28cab88cbf572a75f65846d9b0c8dd68b255fb8c4972cdbb577f299.
Gezieltes Update erfolgreich: Edge30115/Caddy30132, Net95742/CP10852 und neun
VM-PIDs unverändert. CLI/MCP/CP und Full-YAML unverändert; private Sicherung
pre-f44-edge. ErfolgsreceiptSHA4f7eff27ee54f5adafd059f4b31ed1337a86b1c95817857164756400dbafb3f1.
Bestandsstacks Neti/IQ-Store jeweils healthok, warning_count0, stderr0.
OPS-Gast löst files.pythonhosted.org erfolgreich auf und erreicht TCP443, stderr0.
Consumer-Nachlauf60dbc8912e19acc8dc3cdb4c929a7ed62e9bace24b8af7c6a8a8f174b78ca757
nach 35 Offlinechecks und unabhängigem finalem Receiptreview einmal gestartet;
kein Apply/Resume. Erfolgreiche Consumer-Abnahme noch abzuwarten.


F44-Consumer-Nachlauf beendet: `full-live-f44-20261020T140104.237572Z/failed.json`,
SHA256 `155e1ae614b6915d835026c72e95e501ad830643080ca5e664a7862f15d88deb`.
Keine Apply-/Resume-Aktion. Docker UID10001 und vier Readonly-Mounts sowie
Kubernetes UID10001/Secret-Mounts/Deployment-Owner erfolgreich geprüft.
Anschließend `command_failed`; bisheriger Runner verwirft sichere Ergebnis-JSON
bei Guest-Nonzero. Lesend eingegrenzt: benötigte VM-Bibliotheken vorhanden,
PostgreSQL-Verbindung/CREATE-/USAGE-Rechte und bestehende proof-Zeile korrekt,
Rolle ohne Superuser/CreateDB/CreateRole. Vorhandenes S3-Objekt korrekt und
fremder Bucket AccessDenied. OIDC-Discovery per TLS/CA erfolgreich, Issuer korrekt.
Login-/Tokenfluss oder folgende HTTPS-Probe noch offen. Keine weitere schreibende
Probe wiederholt; neuer gezielter Nachlauf muss feste Phasen und ausschließlich
allowlisted Ergebnisse auch bei Nonzero erhalten. MCP/Repeat/Late warten weiter.


Gezielte OIDC-Diagnose vorbereitet und unabhängig geprüft: Runner
`/tmp/vzctl-controller-oidc-diagnostic.py`, SHA256
`7e2f4b34bcfe1116b51b71eadd6bfb1302c826cb7b9248e29f027a93cd70ca99`.
28 Offlinechecks einschließlich echter synthetischer Guest-Programmläufe für
HTTP400 und Redirect307 bestanden. Nicht-Redirect-Loginfehler behalten Status
und feste Fehlercodes; Codes, Tokens, Location und Rohbody gelangen nicht in
Nachweise. Keine PostgreSQL-/S3-Probes und keine Apply-/Configänderung.
Dies ist zunächst nur der Vorbereitungsnachweis, noch kein Login-Erfolg.

OIDC-Diagnose danach einmal ausgeführt und erfolgreich: Discovery, Authorize,
Login307, Token, JWKS/RS256-Prüfung, Nonce/Claims und Userinfo vollständig
bestanden. Beleg `oidc-diagnostic-f44-20261020T141218.113841Z/result.json`,
SHA256 `b64916158aba11b3ad07ae4e3742ebf0f1a3efd9b6d9d2f4f5b3ac8eb57f431e`.
Keine DB-/S3-Schreibprobe und keine Apply-/Configänderung. Kein OIDC-Produktfix
belegt; gemeinsame Consumer-Abnahme mit erhaltenen Phasen/Resultaten folgt.


Neuer Full-Consumer-Nachlauf a55f3fff7268d8a2087586d8c603d3a60b67b385a64ffbf86ac563b35000990d
nach 49 Offlinechecks und unabhängiger Review einmal ausgeführt. Feste Phasen und
allowlisted Ergebnisse erhalten auch Guest-Exit1 und HTTP503.
`full-live-verified-20261020T141559.157843Z/failed.json`, SHA256
`71752d85c7ed443a0cc1e97c9e428ab1424fbbd3b6836edd6792a1d28d2d674e`.
VM-Probe vollständig erfolgreich: PostgreSQL/S3/OIDC true, Guest-/Transportexit0.
Docker-HTTPS liefert503: PostgreSQL/S3 true, OIDC URLError; Curl22.
Kubernetes-HTTPS-Probe noch nicht ausgeführt. Keine Apply-/Resume-Aktion.
Damit gezielte lesende Eingrenzung Docker-OIDC-DNS/TLS/Discovery; kein
pauschaler OIDC-Fix und keine weitere DB-/S3-Schreibprobe.


## F45: unvollständige lokale Zertifikatskette unter strikter Prüfung

Docker Python3.13.16/OpenSSL3.0.22 meldet bei OIDC-TLS
SSLCertVerificationError85: Missing Authority Key Identifier. CA lesbar,
keine Proxyumgebung, DNSauth10.123.0.1 erreichbar. VM Python3.12.3/OpenSSL3.0.13
hat strict=false und akzeptiert dieselbe Kette. Öffentliche CA-Extensions
enthalten SKI und BasicConstraints, jedoch keine KeyUsage. Quellcode bestätigt
fehlende Leaf-AKI (rcgen-Defaultfalse) und fehlende CA-key_usages.
[Python3.13](https://docs.python.org/3.13/library/ssl.html#ssl.create_default_context)
aktiviert X509_STRICT standardmäßig.

F45/#81 umfasst neue korrekte Ausstellung, echte strikte Ketten-/Negativtests
und einen expliziten Bestandsupgradepfad. Die CA ist global: Root-Schlüssel
erhalten, keine stille --force-Rotation. Zertifikatsfingerprint und Trustrollout
müssen im geplanten Upgrade ausdrücklich behandelt werden. Keine Zertifikate,
Trustdateien oder TLS-Prüfoptionen live geändert; Consumerwiederholung wartet.

F45-Vorbereitung weiter: echte OpenSSL-Strict-Regression vor Fix scheitert
mit error85 am Leaf und error92 an der Root-CA. CI stellt OpenSSL@3 ausdrücklich
bereit und exportiert VZCTL_TEST_OPENSSL; Workflow-YAML lokal erfolgreich geparst.
Kontrollierter Rollout-/Recoveryentwurf und verwaltetes Trustinventar stehen in
[11-certificate-upgrade-2026-10-20.md](11-certificate-upgrade-2026-10-20.md).
Produktcode/Review noch in Arbeit, keine Live-Truständerung.

Lifecycle-Nachlauf zusätzlich vorbereitet: Runner
`/tmp/vzctl-controller-lifecycle-live.py`, SHA256
`6f99ad5251003fb55741bba17cbdcbefef56ce12242e4facaa52af492d4ba0ee`.
111 Offlinechecks auch durch Root bestanden. Tatsächliches Swift-Zeitformat
mit Z unter Python3.9 und queued-Job vor Journalanlage abgedeckt. Getrennte Phasen
CP-Neustart, normales Lab-Down und lesender Observer eines expliziten UI-Apply.
Noch keine Livefreigabe: erfolgreiche neue Full-/Retention-Kette und F45-Transition
sowie abschließender Review fehlen.

### F45 Gesamtprüfung und F46 Betriebszustand

F45: gesamte Rustworkspace-Suite mit `RUSTFLAGS=-D warnings` und OpenSSL erfolgreich; 466 Tests bestanden, ein ausgewiesener Skip. Log: `/tmp/vzctl-f45-workspace-tests-unsandboxed.log`. Der erste Sandboxlauf konnte Mock-UDS nicht anlegen (EPERM); nach erlaubter lokaler Socketnutzung vollständig grün. Produktreview und zehn Zertifikats-/zwei CLI-Tests ebenfalls grün. Keine Live-Truständerung.

F46/#82: Nutzer bestätigt stopped als normalen Zustand. Echte Supervisorabfrage am 20.10.2026 um14:40UTC bestätigt Neti/Lab sechs running, IQ-Store drei stopped seit14:34:55UTC. Gespeicherte PIDs im stopped-Record sind kein Laufnachweis. Benutzerzustände erhalten; keine Starts für Prüfungen. Neue Rolloutbaseline nötig, alte Neun-VM-Receipts bleiben historische Nachweise.

### F46 echte Read-only-Abnahme

Neue Debug-CLI SHA256 `376885d59401fb54adbb276befb811caf328c5ac4136ec1d553b4cbf68fc136e`: IQ-Store `stack status` Exit0, `checks=skipped`, `stopped_vms=3`, null Warnungen, stderr leer. Beleg `/tmp/vzctl-f46-iqstore-readonly.json`. Keine Installation, keine VM-/Zertifikatsänderung. Summary-Text wird ergänzend neutral als stopped formuliert.

17 gezielte Rust-Tests,172 gesamte UI-Tests, Typecheck und unabhängiger Review bestanden. Der gemeinsame Cargo-Lauf fand eine Testverzeichnis-Kollision in identity_cli (PID+gleicher Zeitstempel); deterministischer Red-Test bestätigt parallele gemeinsame Verzeichnisse. Enger Testisolierungsfix in Arbeit; gemeinsamer finaler Lauf bleibt offen.

### F45/F46 finaler gemeinsamer Build

Testisolierung korrigiert und deterministisch geprüft (gleicher Clocktick); gemeinsame Rustsuite474 bestanden/1 Skip, UI172 bestanden, keine Warnungen. Logs `/tmp/vzctl-f45-f46-workspace-final.log`, `/tmp/vzctl-f46-ui-all.log`, `/tmp/vzctl-f45-f46-release-final.log`, `/tmp/vzctl-f46-ui-release.log`. CLI SHA256 `ccb10e53bcbb72370e91fde7c38c09e000f737c38beec7211cae88e903d2ac1b`; UI-Appbinär SHA256 `c0728d18154013d9c8cfc36238703162609aa8a3fe174830afe91d88dbfcde0a`. Codesign-Prüfung erfolgreich, noch nicht installiert.

Release-CLI direkt lesend am14:58UTC: IQ-Store neutral stopped/3/Checks skipped/Warnings0/Exit0; Neti running/Checks evaluated/Warnings0/Exit0. Beleg `/tmp/vzctl-f46-release-readonly.json`.

Privater F45-Kandidat `/private/tmp/vzctl-f45-candidate-20261020T145701Z`, Manifest SHA256 `9eaaa01293e983154d3bb29aaa9e390d7a5f471c07ec94b32c55e37c3ff86ae6`. Aktives CA-Inventar stimmt exakt mit Quellbindung überein, Schlüssel unverändert. CA und neun Leaves bestehen Strictprüfung (10 Ketten); Beleg `/private/tmp/vzctl-f45-candidate-strict.json`, SHA256 `785ac9313c12d5e207e0325fd2b4f00341d0b9d046d5470599192b696f9fe6eb`. Keine aktive CA-/Truständerung. Aktiver Caddy nach nutzerseitigem IQ-Store-Down PID43773 mit fünf aktiven Hosts; frische Rolloutbaseline bindet diesen Zustand.

### F45 Vorbereitung sicher angehalten: Legacy-Zertifikatsschreiber

Der geprüfte prepare-Lauf stoppte mit `candidate_file_hash` vor Anlage des Arbeits-/Backupverzeichnisses und vor jeder aktiven Änderung. Alter K7-Reconcile schreibt inzwischen die drei Lab-Paare auth/docker/kube (Zertifikat, **auch Schlüssel**, Metadaten) neu. Der zuvor gültige private Kandidat ist deshalb nur historischer Nachweis und darf nicht aktiviert werden. Beleg `/tmp/vzctl-f45-prepare-source-change.json`. Endgültige Quelle und Kandidat müssen unter CP-Quiesce gebunden werden; Root/Nicht-Lab-Inventar bleiben vorab geschützt. Kein Versuch, den30-Sekunden-Writer durch schnelles Wiederholen zu überholen. Finaler Keyerhalt bezieht sich auf den geprüften, angehaltenen Quellzustand.

### F45: quieszierter Rollout aktiviert

Producer `7d92a477…` und Consumer `20b6afa7…` gemeinsam unabhängig geprüft (90/133 Offlinechecks). Plan `2270e73ea05c68f038ea04347118a73b7c99177d0c7383db9a3b1f38a9c8b4a7`; private Belege unter `vzctl-update-backups/20261020T094019Z/f45-profile-upgrade`. Promotion bestanden, Receipt `784fe97d76e19bd2df0b04c3c0f40bfb36adfe62257cb079cee2f69b29388428`. Finale Zertifikate erst bei angehaltenem CP erzeugt und strikt geprüft. CLI `ccb10e53…` aktiv, CP 64686, fünf Claims frisch Ready. Alle sechs laufenden VM-PIDs, Net95742, Edge30115 und Caddy43773 unverändert; IQ-Store weiterhin dreimal stopped. Guest-Trust für sechs laufende VMs bestanden; IQ-Store-Trust bleibt bis regulärem Stack-Apply ausstehend. Schlüsselbund/Lab-Apply und Full-Consumer-Abnahme noch offen.

Finaler effektiver Plan `1cd7e07a388aacc19ef80c6684bd3259ec4087599f65c4eb03820d62349cc0b6`, finales Manifest `27862d0ba65d8d649db378d98289766192e45ecb6d736585d4c2f2e2bbdbd44d`, Strictnachweis `7484f9b3dd290c3a3da9d93fd655f0ab4de5a0b0344cf33e871216ab62ccf1a4`. Guest-Trust-Receipt `92dd0e342b216e9bfdcee352aaef2784c3446c674ddb569921d02c62e22c26a2` bestätigt sechs Ziele und drei ausstehende gestoppte Ziele.

Keychainphase sicher bei `keychain_after_add_inventory` angehalten: `add-trusted-cert` und neue Root-Verifikation lieferten Erfolg, `find-certificate` zeigt jedoch weiterhin ausschließlich die alte CA. Alte CA wurde nicht gelöscht. Fehlerbeleg `7968daa284c63b82349802a62228c6a4adb3bd0aff72ccde3e4aa65948bb4d93`; gesicherter Recoveryablauf wird separat geprüft. Kein pauschaler Wiederholungsaufruf des alten Runners.

### F46 native Abnahme abgeschlossen

App `c0728d18154013d9c8cfc36238703162609aa8a3fe174830afe91d88dbfcde0a` installiert, alte App unter `pre-f46-ui` privat gesichert. Installer unabhängig mit 37 Offlinechecks geprüft. Native `/vms`-Sichtprüfung: IQ-Store „Gestoppt · nicht geprüft“, drei gestoppte VMs und abhängige Prüfungen übersprungen; Neti und Lab jeweils 3/3 und Gesundheit ok. Vor/nach Installation alle sechs laufenden VM-PIDs, drei stopped-Zustände und Dienst-PIDs unverändert. Keine VM-Aktion. Screenshot `f46-stopped-neutral.png` im lokalen Visualisierungsordner.

### F45 Schlüsselbund erfolgreich, neuer Befund F47/#83

Recoveryrunner `81ea3ce99f328835a7f9fd3097037ea417781479dff00f0adabf0579593c7344` mit53 Offlinechecks unabhängig geprüft. Exakt alte Zertifikatsinstanz entfernt/neue importiert; TrustSettings und vollständiges fremdes Zertifikatsinventar unverändert. Keychain-Receipt `9cb0c9f94ae10442b18499a59e34efe7d1d9f857df0c982d8c8dfd0785e8b099`. Ursache: identische Issuer/Serial-Kennung wird beim Add als Duplikat behandelt.

Anschließender deklarativer CA-onlyApply: lokale Code-ConfigMapdatei, Lab-CA-Asset und OPS-CA aktualisiert, Plan enthielt genau die erforderliche Manifeständerung. Generation8 `c21fade8-a4e8-4e71-9852-aff12f748fd9` scheitert jedoch `ensure_kubernetes`: zwei SSA-Konflikte mit Manager `vzctl-kubernetes` auf `.data.ca.crt` und `.metadata.annotations.vzctl.dev/manifest-sha256`. Keine Force-/Resumeaktion, keine SQLiteänderung. F47/#83 aufgenommen; Kubernetes-Projektion und Full-Consumer bleiben offen.

F47 read-only Nachprüfung: sechs running/drei stopped, alle VM-/Net-/Edge-/Caddy-/CP-PIDs erhalten; Neti DNS und drei HTTPS-Prüfungen HTTP200/TLSverify0. Fünf Claims Ready und RegistryUUID unverändert. Root/OPS/Docker neue CA, K8sConfigMap/Pod noch alte CA mit unveränderten UIDs. ManagedFields-Beleg zeigt denselben Manager in zwei Operationen Apply/Update; beide besitzen die beiden Konfliktfelder. Imperatives Create ist der Ursprung. Evidence `/private/tmp/vzctl-f47-agent-managed-fields.json`, `/tmp/vzctl-f47-readonly-result.json`, `/tmp/vzctl-f47-protected-result.json`.

F45 isolierte positive Docker-Nachprüfung: unverändertes Python im bestehenden proof-Container, ssl.create_default_context mit `/opt/lab/ca.crt`, VERIFY_X509_STRICT explizit als aktiv geprüft. Öffentliche OIDC-Discovery HTTP200, Issuer passend, Gast-/CLI-Exit0. Keine Login-/DB-/S3-Mutation. Vollständiger Beleg `/private/tmp/vzctl-f45-root-docker-strict-discovery-confirmed.json`; dies ist nur die TLS/Discovery-Teilabnahme, keine Full-Consumer-Gesamtabnahme.

F47 Produktstand: sechs neue Regressionen und19 Kubernetes-Modultests warningfrei bestanden; zuvor vier regressionsrelevante Fälle rot. Neue managed_fields-Projektion erhält eigenen Apply-Feldumfang, ergänzt bei Nur-Update eine identische SSA-Brücke und entfernt nur eigene Update-Einträge mit UID/RV/gesamtem managedFields-Test. Fremde Manager/Subresources erhalten; keine Force-SSA. Unabhängiger Review und echter isolierter K3s-Vertragstest vor Installation noch offen. Logs `/tmp/vzctl-f47-agent-{red,green,kubernetes}.log`.

F47 echte API-Abnahme: 23 strikte Modultests bestanden; isolierter Live-Harness scheitert in `legacy_update` mit `contract_legacy_apply_failed`, noch keine bestätigte Migration. Namespace und Testressourcen erfolgreich bereinigt. Privater Beleg `controller-lab/evidence/f47-k3s-contract-20261020T155608.590236Z/contract-result.json`, Plan SHA256 `9f01b779aa8bdd242de94138a8b68a3675f7d322ef7e21abc1c932a8a18dcf1b`. Noch keine CLI-Installation/Resume. Gezielte private API-Diagnostik wird ergänzt.

F47 Diagnose 16:02UTC: `synthetic-diagnostics.json` unter `f47-k3s-contract-20261020T160128.065441Z` bestätigt erfolgreiche identische SSA-Brücke. Apply besitzt die konkreten Felder ohne Map-Strukturmarker `.`, Update besitzt zusätzlich diese Marker unter data/annotations/labels. Produkt meldet `field ownership bridge did not establish complete Apply ownership`; noch kein Ownership-Patch. Auch dieser Namespace vollständig bereinigt. Exakte Feldmengen dienen jetzt als Regression.

### F47 installiert und Generation8 abgeschlossen; K3s-OIDC noch offen

489 Workspace-Tests bestanden/2 ignored (darunter der separat mit zehn Fällen bestandene echte K3s-Vertrag), keine Warnungen. Finaler Suitebeleg `/tmp/vzctl-f47-workspace-final-openssl.log`; voriger Lauf scheiterte ausschließlich am irrtümlich gesetzten fehlenden OpenSSL-Pfad, korrigiert auf vorhandenes `/opt/homebrew/bin/openssl` (4.0.3). Releasebuild und Codesign grün. CLI SHA256 `fc921a24463d9baa7f57b09d6aaa6f1223b4bf51a5a2260c717482fd576a30d4`, installiert ohne Dienstneustart; Installerreceipt `5bcf3712339050091c2b8c1cfd75d5df8a5ed912ec26c3ad03e35a2768fa6404`.

Genau einmaliger regulärer Resume: gleiche Journal-ID/Generation8/Payload, status done, keine Lease, Plan leer. Neue Registry `1636a8e1-2584-4ab5-aeb7-1db5bd445265`, fünf Ready/keine Warnung. ConfigMap und bestehender Pod erhalten neueCA; Deployment/ConfigMap/Pod/Dockeridentitäten unverändert. Recoveryreceipt `56bdba86a394760175659feffe1e77b6e2a4cb5382874a599396bb6b7b4546a3`.

Full-Nachlauf ohne Apply/Resume: VM und Docker PostgreSQL/S3/OIDC vollständig bestanden; Docker HTTPS200. Kubernetes PostgreSQL/S3 erfolgreich, OIDC URLError, HTTPS503. Fehlerbeleg `full-live-f47-verified-20261020T161753.239620Z/failed.json`, SHA256 `f26383f0fbfc7e5dd0a3b74c957dbd5598551444025bb8a02ff94a52ca7fe794`. Keine automatische Wiederholung; gezielte lesende Pod-DNS/TLS/Discoverydiagnose folgt.

## Aktuell: F48 / #84 – K3s-Resolver

Der normale CoreDNS-Weg liefert für `auth.svc.controller-lab.vz.test` und
`server.lan.controller-lab.vz.test` NXDOMAIN. Direkte Bridge-DNS-Abfragen aus
demselben Pod liefern Adressen; striktes TLS mit korrektem SNI erreicht die
öffentliche OIDC-Discovery mit HTTP200. K3s verwirft die automatisch gefundenen
Resolverdateien wegen Loopback bzw. scoped Linklocal und verwendet Google-DNS.
Dies ist ein DNS-Integrationsfehler, kein verbleibender TLS-Fehler.

Produktfix in Arbeit: eigene Resolverdatei auf Server und Agents, `resolv-conf`
aus dem eindeutig zugeordneten Cluster-Netz/Registry, nur bei aktiviertem Stack-DNS.
Danach gezielter idempotenter CoreDNS-Rollout mit geprüfter K3s-Addonidentität
und UID-/RV-Schutz. Fremde DNS-Konfiguration wird nicht übernommen.
Bestehende ClusterFirst-Anwendungspods bleiben erhalten. Bestehende Pods mit
`dnsPolicy: Default` übernehmen den Resolver erst bei regulärer Neuerstellung.
Noch keine F48-Liveänderung. Vorhandene F47-Receipts bleiben historisch unverändert.

Read-only Belege: `/tmp/vzctl-k3s-oidc-readonly-result.json`,
`/tmp/vzctl-k3s-dns-readonly-result.json`,
`/tmp/vzctl-k3s-forward-readonly-result.json`.
Issue: https://github.com/frankhildebrandt/vzctl/issues/84.

### F48 Vorprüfung: Reihenfolge der Docker-Mounts

Die erste F48-Vorprüfung hielt vor Installation/Mutation an. Alle Betriebswerte
waren unverändert; nur der Hash von `docker inspect .Mounts` variierte.
Lesender Beweis: beide beobachteten Hashwerte entsprechen Permutationen derselben
vier vollständigen Mountobjekte (`/private/tmp/vzctl-f48-mount-order-diagnostic.json`).
Der Abnahmevergleich wird daher nach eindeutigem Destination-Pfad sortiert;
alle Felder, Readonly-/Bind-Vertrag, Container-ID und User bleiben geprüft.
Dies ist eine Korrektur des Testvergleichs, kein Produktfehler. Der alte Plan
`f48-apply-20261020T164326Z` bleibt als nicht ausgeführter Plan erhalten.

## F48-Apply, vollständige Consumer- und MCP-Abnahme erfolgreich

CLI `f7cdececf68f6a2239cb0563d0f8287842ef0faa0612c1228fb3a395b7d384ab`
installiert; 496 Workspace-Tests bestanden, 2 ausgewiesene Ignores, keine Warnungen.
Regulärer Lab-Apply mit unveränderten YAML-/Manifestinputs erfolgreich; eigene
Resolverdateien auf beiden Nodes und gezielter CoreDNS-Rollout bestätigt.
Alle Anwendungs-, Daten-, Binding-, Seed- und geschützten Prozessidentitäten erhalten.

- Apply: `f48-apply-20261020T164823Z/result.json`, SHA
  `ecc6813884475b7bf5fb9519f2534fcdee3330491bcc18d6c6d394dbf8f143a2`.
- Full: `full-live-f48-verified-20261020T165021.477100Z/result.json`, SHA
  `534fac9e13afb1e01858d867fb65e4e99c82af7ec10b6d656fc44d352f245311`.
  VM/Docker/K3s jeweils PostgreSQL/S3/OIDC erfolgreich, beide HTTPS-Proben200.
  Normales Pod-DNS für lokale Namen, cluster.local und öffentlichen CNAME geprüft;
  strikte TLS-Discovery und echte Anmeldung erfolgreich.
- MCP: `mcp-live-20261020T165132.868386Z/result.json`, SHA
  `1303378af03124475813ae98bf1558611bfb9c0b416d6305404e037d063a00e8`.
  Fünf Ready-Claims mit öffentlichen Bindingreferenzen, zwei Ready-Nodes,
  Diagnose ok ohne sensible Datensammlung.
- Unabhängig: `/private/tmp/vzctl-f48-edge-after-apply-result.json`, SHA
  `97ee6d83b590943bc4c0b0db8430c3fa16ee0963c454cc2fd013db4059b94e05`.
  Neti-DNS/3×HTTPS200/TLSverify0, alle geschützten PIDs/6running/3stopped erhalten.

F42/F43/F44/F45/F47 sind damit fertig (Issues #78/#79/#80/#81/#83).
F48/#84 bleibt bis zum echten Wiederholungsnachweis ohne Neustart offen.
Anschluss: vorhandene Folge-Runner minimal an echten F48-Beleg binden, danach
Repeat → LateClaims → Ownership/Ausfälle → Retention → Lifecycle/UI-Apply.
Keine neuen Runnerketten; historische Receipts unverändert. Docker-Mounthashes
über vollständige, nach eindeutigem Destination-Pfad sortierte Objekte bilden.

## Abschlussfortsetzung: Repeat und Late belegt · 20.10.2026, 20:11 MESZ

Nach ausdrücklichem Nutzerauftrag „fortsetzen“ wurde ausschließlich die vorhandene
Late-Abnahme einmal ausgeführt. Erfolgreicher Repeat Generation 10 wird übernommen:
`evidence/followup-repeat-f48-20261020T170352.422564Z/result.json`, SHA256
`6740dace3e86e1d9f2897a9433e299b340bf35b1afe348259da6b1ed25234998`.
Kein neues Repeat-Apply oder zusätzliches Reconcile.

Vier zuvor fehlende Kubernetes-Claims wurden vom regulären Controller automatisch
Ready; konservative Obergrenze aus dem ursprünglichen Ablauf **8,663142 s**.
Neun Ready-Claims insgesamt, gleiche Registry und unverändertes abgeschlossenes
Journal Generation 10. Alte Bindings, Eingaben, Seeds, Daten, CoreDNS sowie
VM-/Dienst-/Anwendungsidentitäten erhalten; sechs VMs laufen, IQ-Store drei gestoppt.
Late-HTTPS **200**, PostgreSQL/S3/OIDC erfolgreich.

- Late: `evidence/followup-late-f48-20261020T180633.949929Z/result.json`, SHA256
  `80a3838ad504490e3b9b676379a02dc3c7f30d27a8c358cbedc2120308302f0b`.
- Event: `evidence/late-events-f48-20261020.ndjson`, SHA256
  `288b13bf28c3a72b689f0e86f1ed75ed2719c37e06e0bf110642d0dc6a8cbbe8`.
  Eine echte `controller.status.changed` mit neun Ready-Claims, privat 0600;
  CLI-Subscription sauber beendet, stderr leer.
- Bestehender Adapter nach enger Testkorrektur: `/private/tmp/vzctl-controller-f45-baseline.py`,
  SHA256 `6689bc49e926aaa2729467712975e591d008092259a86d3260cabc30caa12e05`.

Der ursprüngliche Late-Runner stoppte nach erfolgreicher Ready-Prüfung an einer
falschen Testannahme: eigener Ingress-Claim erzeugt eigenen Service/NodePort
(`31605` statt `31040`). Beide neuen Late-Blöcke sind exakt an den zugehörigen
UID-/Owner-geprüften Service und dessen Ready-Endpoint gebunden; alle bestehenden
Caddy-Blöcke sind bytegleich. Fünf negative Offlinefälle sichern diese enge
Prüfung ab. Originales `failed.json` und Preflight bleiben unverändert;
direkte Nachabnahme ohne erneute Claim-Anlage, Apply oder Reconcile beendet.
Die noch fehlende ursprüngliche Late-HTTPS-Consumerprobe wurde einmal ausgeführt.
Dies ist kein belegter Produktfehler.

Zusätzlich von Root geprüft: echter Kubeconfig-Export für #74 unter
`evidence/kubeconfig-export.json`, SHA256
`21ec20ac565ae5f82146f07cec9c4bb2e0650042aa32c0102df583e5474c160a`.
Neue `kubeconfig.yaml` 0600 mit Nutzerbesitz, Endpoint `10.123.0.11:6443`,
TLS aktiv, Credentials ausschließlich in Datei; keine Eingabe-/Registryänderung.

Alle privaten `evidence/`-Pfade oben liegen unter
`/Users/frank/vzctl-labs/controller-lab/`. Details:
[Runtime-Übergabe](handoffs/completion-runtime.md).
Nächster einzelner Schritt: `secret-owner` mit neuem Adapterhash;
danach MinIO → K3s → foreign-db zuletzt → Retention → Lifecycle/UI-Apply.
Diese Folgeabnahmen bleiben offen; kein Faultfall wurde hier begonnen.

### Ergänzung: MCP mit neun Ready-Claims

Root hat anschließend den vorhandenen generischen Stdio-Smoke ausschließlich
lesend auf dem neuen Late-Zustand ausgeführt. `controller_status`,
`kubernetes_status` und `diagnose` bestanden: neun Ready-Claims mit öffentlichen
Bindingreferenzen, zwei Ready-Nodes, keine Credentials in der Diagnose.
Beleg: `evidence/mcp-late-20261020T181307.392723Z/result.json`, SHA256
`77c7426ce1c959b99efc93000e5ade1c93dd0de526f3d7f2155f3360bc642723`.
Kein Apply/Reconcile und keine Dienst-/VM-Mutation; ursprüngliche MCP-Belege erhalten.

### Fault 1: Secret-Ownership bestanden

Vorhandener Fault-Runner genau einmal mit `--case secret-owner` ausgeführt,
gebunden an Full `534fac…`, Late `80a383…`, Followup `e8894a…` und Adapter
`6689bc…`. Vor Mutation Restore-Timer aktiviert; Wiederherstellung in `finally`.
`db` meldete `OwnershipConflict`/Exit 24, unabhängige Claims blieben Ready;
Secret-UID und Daten erhalten, ursprünglicher Owner wiederhergestellt.
Recovery-Reconcile erfolgreich; VM/Docker/K3s jeweils PostgreSQL/S3/OIDC wahr,
beide HTTPS-Proben 200. Bindings, Daten, geschützte VM-/Dienstidentitäten und
Journal Generation 10 unverändert; alle drei IQ-Store-VMs weiter gestoppt.

Beleg: `evidence/fault-secret-owner-20261020T181333.397822Z/result.json`, SHA256
`a4d23a1933ed900680414b9518c23cac7ad0f9e6e7a5899d4ae5d099ee77cc2c`.
Kein Apply/Resume; nächster einzelner Fall MinIO.

### Fault 2: MinIO-Ausfall bestanden

Nach Fault 1 genau ein vorhandener `--case minio`-Lauf, gleiche Beleg-/Hashbindung.
Restore-Timer vor dem Lab-Dienststopp, Dienststart in `finally` geprüft.
`files` meldete `ProviderUnavailable`/Exit 24; DB und OIDC blieben Ready.
Nach Wiederherstellung Recovery-Reconcile sowie VM/Docker/K3s mit jeweils
PostgreSQL/S3/OIDC erfolgreich; beide HTTPS-Proben 200. Bestehende Bindings,
Daten, Identitäten und Journal Generation 10 erhalten; IQ-Store weiter gestoppt.

Beleg: `evidence/fault-minio-20261020T181516.967081Z/result.json`, SHA256
`aa5b0f54d8199b4e59fd22b564caad9d90f31c10544799a9594031bb0c7a1086`.
Kein Apply/Resume; nächster einzelner Fall K3s.

### Fault 3: K3s-Ausfall bestanden

Nach zwei erfolgreichen Vorgängern genau ein vorhandener `--case k3s`-Lauf.
Restore-Timer vor dem Lab-Dienststopp, erfolgreicher Dienststart in `finally`.
Discovery und Kubernetes-Status meldeten `KubernetesUnavailable`/Exit 24;
bestehende Ingress-Routen blieben während des Ausfalls erhalten.
Nach Wiederherstellung Recovery-Reconcile sowie alle VM/Docker/K3s-Consumerprobes
erfolgreich, beide HTTPS-Proben 200. Nur der ausdrücklich erlaubte K3s-Server-
Dienststartzeitpunkt änderte sich; VM-/Anwendungsidentitäten, Bindings und Daten
erhalten. Journal weiterhin Generation 10, IQ-Store weiterhin dreimal gestoppt.

Beleg: `evidence/fault-k3s-20261020T181553.662987Z/result.json`, SHA256
`cfc1caa51dfaf92e57c466984c4ee132b54d8bc822424b7896e502410e3555ac`.
Kein Apply/Resume; nächster Fall foreign-db mit allen drei Vorgängerbelegen.

### Fault 4: Fremde PostgreSQL-Datenbank geschützt; vier Faults abgeschlossen

`foreign-db` wurde genau einmal und zuletzt ausgeführt, mit Pfad-/SHA-Bindung
an alle drei erfolgreichen Vorgänger. Restore-Timer vor Anlage aktiviert.
Der bestehende per-Claim-Datensatz belegt `Conflict`/`OwnershipConflict` für die
exakte Test-CR-UID `1b16a066-8ad3-4a6c-97f0-33f554084b42`.
Die fremde Datenbank `controller_foreign` behält Owner `postgres`; kein fremdes
Binding erzeugt. UID-geschützter CR-Cleanup erfolgreich, Restore-Timer inaktiv.
Jetzt neun Ready-Claims plus bekannter foreign-Datensatz `Retained`/`ClaimRemoved`.

Der erste Runner stoppte erst nach Cleanup und Recovery-Reconcile an
`cp_registry_not_ready`: Nur sein Foreign-Zweig prüft irrtümlich eine
`controller reconcile`-Antwort wie eine `controller status`-Antwort.
Registry-Felder werden ausschließlich bei `status` ergänzt
(`crates/vzctl/src/controllers/runtime.rs`, Status-/Reconcile-Verzweigung).
Lesender tatsächlicher Status bestätigt enabled, gleiche Registry-Generation
und frischen `lastAttemptAt`; kein belegter Produkt- oder CP-Loopfehler.
Originaler Fehlerbeleg bleibt erhalten:
`evidence/fault-foreign-db-20261020T181636.383891Z/failed.json`, SHA256
`86cf4f7885917e4faa5a682596eb584f757943c850c705131d05a411b3c06bb9`.

Direkte Nachabnahme ausschließlich lesend: vorhandener Conflict-Datensatz und
UID-Receipt geprüft, CR/Secret-Abwesenheit, DB-Owner, inaktiven Restore-Timer,
Registry, neun Ready plus foreign Retained, bestehende DB-/S3-Daten und alle
geschützten Identitäten geprüft. K3s-Recovery-Consumerprobes werden per SHA
`cfc1caa…` übernommen; keine erneute Consumerprobe, Anlage, Apply oder Reconcile.
Journal Generation 10, Bindings, Eingaben und Seeds unverändert.

Ergebnis: `evidence/fault-foreign-db-20261020T181636.383891Z/result.json`, SHA256
`fd3bcb09eb95fdbc208522149c81d848ba53135d56e71bfc30e4bf3bc14b2430`.
Alle vier Faults sind damit abgeschlossen. Kein Testprozess aktiv.
Nächster begrenzter Schritt nach Root-Abgleich: vorhandene Retention-Abnahme mit
neuem Adapter `6689bc…` und allen vier echten Faultbelegen; noch nicht begonnen.

## Retention bestanden, Originalzustand wiederhergestellt · 20.10.2026

Nach Root-Abgleich der vier Faultbelege genau einmal vorhandenen Retention-Runner
`1058d12514aaaa2d44fb41612bd24e56b71029fa0296a0cbf8f980f1cab52d41`
mit Adapter `6689bc…` und vollständiger Full-/Late-/Fault-Belegkette ausgeführt.
Private Originalkopie und beide Kandidaten vor Aktivierung angelegt und validiert.
Vor jedem Apply exakten Plan geprüft: fünf Service-Claim-Aktionen und ausschließlich
eigenes `ops/proof`-Container-Binding-Recreate; insgesamt genau zwei Applys.

- Entfernt: vier Late-Claims Ready, fünf YAML-Claims plus foreign-Datensatz
  Retained. Docker-/Kube-HTTPS entzogen; Late-Host erhalten. OIDC-Discovery und
  Late-Client verfügbar, entfernter YAML-Client unbekannt. Daten, Bindings und
  Credentials in dieser Phase lesend unverändert geprüft.
- Restore: Originalbytes von `hypernetwork.config.yaml` wiederhergestellt,
  SHA256 `4746f3a0301db79b80200f445fdb240fea82f35aea71a453dd1fe5f1c587a8fd`.
  Originaler Caddy-Konfigurationshash wiederhergestellt; nur die erlaubten
  eigenen Routendifferenzen und `ops/proof`-Recreates. Neun Ready plus ein
  bekanntes foreign Retained. Journal Generation **12**, abgeschlossen.
- Danach VM/Docker/K3s jeweils PostgreSQL/S3/OIDC erfolgreich, beide HTTPS-Proben
  200. Bestehende Daten, Binding-/Credential-Hashes, Late-CRs und alle geschützten
  VM-/Dienstidentitäten erhalten. Sechs VMs laufen, IQ-Store bleibt dreimal gestoppt.
  Keine Host-/VM-/Dienstneustarts, kein Purge.

Beleg: `evidence/retention-live-f48-20261020T182200.125655Z/result.json`, SHA256
`c7ffa96123639338c13fc27513f0ac3c8ea9ec526edb0eae943f3c3f0db18114`.
Zusätzliche reine Leseprüfung `post-restore-readonly.json` im selben Verzeichnis,
SHA256 `67705b86fab0e7df9f9ee27a0ee842e0cdeb34d2fd6f307433409f6f811222bb`:
aktive Konfiguration bytegleich privater Originalkopie, Seeds unverändert,
fremde Datenbank `controller_foreign` weiterhin vorhanden mit Owner `postgres`.
Kein Fehler, kein zusätzlicher Apply oder Wiederholungsversuch.

Retention beendet; kein Prüfprozess aktiv. Nächster begrenzter Schritt nach
Root-Abgleich: vorhandener Lifecycle-Modus `cp-restart`, danach separater
graceful Lab-down und konkret abgestimmtes UI-Apply samt nativer Beobachtung.
Lifecycle/UI-Apply wurden hier nicht begonnen.

## Lifecycle: CP-Neustart bestanden · 20.10.2026

Vor der ersten Betriebsaktion hielt der vorbereitete Lifecycle-Runner rein lokal
an: seine Regex für Belegnamen lehnte den vorgeschriebenen Namen `k3s` ab.
Auf Root-Freigabe nur diese Prüfung auf die exakte bekannte Namensmenge geändert;
Duplikate/Unbekannte bleiben verboten. Neuer Lifecycle-SHA256
`a2959a839d08e6d435ffe6ee7540a619c32f0cd934015005623187bf0820d161`.
Historischer Eingabefehler ohne Runtimeaktion:
`evidence/lifecycle-input-preflight-f48-20261020.json`, SHA256
`88d5dcc5002f8a2add989aea05a7eecd2272303b453df54b490b127689a023a9`.

Danach genau ein regulärer Supervisor-Neustart. Supervisor PID `33942`,
Net/Edge/Caddy und sämtliche VM-/Anwendungsidentitäten unverändert.
Persistierte Registry-Generation übernommen, frischer erfolgreicher regulärer
Controllerlauf mit neun Ready plus foreign Retained. Daten, Late-CRs,
Bindings/Credentials und Eingaben erhalten. VM/Docker/K3s jeweils PostgreSQL,
S3 und OIDC erfolgreich; beide HTTPS-Proben 200. Kein Apply oder VM-Neustart.

Beleg: `evidence/lifecycle-f48-cp-restart-20261020T182747.606692Z/result.json`, SHA256
`3eae1d8a0d6d137ac28c6e9c60d029b5b04f400ecf300e6563b07572ed33148e`.
Nächster freigegebener Modus: einmal graceful Lab-down, anschließend Stopp vor UI-Apply.

### Graceful Lab-down und Quieszenz belegt

Nach erfolgreichem CP-Neustart genau ein reguläres Lab-down ohne Purge.
Alle drei Lab-VMs und weiterhin alle drei IQ-Store-VMs gestoppt; Neti-VMs mit
unveränderten PIDs aktiv. Registry deaktiviert, Generation null, alte Lab-Helper
beendet; Supervisor/Net/Edge unverändert. Acht Lab-Caddy-Header entfernt,
sämtliche fremden Blöcke bytegleich. Journal Generation **13**, abgeschlossen,
keine Lease. Bindings/Credentials erhalten; 35 Sekunden tatsächliche Ruhe
einschließlich Registry-/Dateihash-/Prozessprüfung erfolgreich beobachtet.

Der ursprüngliche Prüfer stoppte unmittelbar nach Down in `down_disabled`.
Sein sanitisiertes Fehlerformat nennt keinen einzelnen Gatecode; der genaue
historische Auslöser bleibt unbekannt. Sämtliche tatsächlichen Stop-/Registry-/
Helper-/Routenguards bestanden in der direkten lesenden Nachabnahme.
Kein Produktfehler wird daraus abgeleitet. Fehlerbeleg unverändert:
`evidence/lifecycle-f48-down-20261020T182901.716132Z/failed.json`, SHA256
`e758e0d4e9c95e546057f108bfab4f5e77dace0840ba6cc544cc78d90d4264cc`.

Nach erfolgreicher 35-Sekunden-Prüfung hielt ausschließlich die Planprojektion
an einer weiteren Prüferannahme an: Sie erwartete kurze VM-Namen, während der
reale Plan korrekt `controller-lab/ops`, `/server`, `/worker` nennt.
Auf Root-Freigabe exakt die vorhandene `LAB_VMS`-Menge als Soll korrigiert;
fremde/kurze Namen, Breaking und Zusatzaktionen werden weiterhin abgewiesen.
Aktueller Lifecycle-SHA256:
`d2dfdf9ff9a113206bf4c84037ebf4e82b00655f6390f56dde6c0aadbb3624d3`.
Keine Betriebsaktion oder erfolgreiche Ruheprüfung wiederholt.

Ergebnis: `evidence/lifecycle-f48-down-20261020T182901.716132Z/result.json`, SHA256
`129d0a760ae380095feb68763e0833f216cb899f07969dd242715911e27c7a97`.
Wiederanlaufplan: exakt drei `start vm` für die qualifizierten Lab-IDs,
alle `breaking=false`. Kein automatisches Apply/Up, kein aktiver Prüfprozess.
Anschluss: konkret durch Root gestarteten UI-Apply-Job mit dessen Job-ID über
den vorhandenen lesenden Observer beobachten und native Bedienbarkeit separat belegen.

## Native UI-Apply und abschließende Runtime-Abnahme bestanden · 20.10.2026

Root startete genau ein natives UI-Apply um 20:39:54 MESZ; Abschluss um
20:40:31, Dauer 37 Sekunden. Sichtbare Invocation-ID
`ebbea683-ed47-4c7a-9cec-3c21050f1917` ist exakt die neue Journal-ID,
Generation **14**. Während des aktiven Apply waren Debug ↔ Betrieb bedienbar,
Fortschritt und Log liefen weiter, Mutationsaktionen waren gesperrt; nachher
wieder freigegeben. Drei Lab-VMs laufen, Drift 0. Nativer Nachweis:
[ui-native-apply-2026-10-20.md](ui-native-apply-2026-10-20.md).

Der bestehende Observer wurde eng an diese tatsächliche Invocation-/Journal-ID
gebunden. Die REST-jobId ist im UI intern und wird weder aufgelistet noch geloggt;
keine Instrumentierung, Proxy oder neuer Runner wurde dafür angelegt.
Aktueller Lifecycle-SHA256 nach dieser engen Korrelationsergänzung:
`aca08d472d2184649ca597c80155dda046b06ee28ce225550011e51c2fa908ec`.
Die native aktive Phase und ihr Ende belegt Root; der Observerloop selbst begann
nach Jobabschluss (`observed_active=false`) und prüfte den echten Folgezustand.
Das neue Journal war zuvor direkt lesend als running/ensure_service_claims bestätigt.

Observer bestanden: genau dieses neue Journal abgeschlossen, neue aktive Registry
mit frischem regulärem Controllerlauf, neun Ready plus foreign Retained;
Daten, Bindings/Credentials, Late-Claims, persistierte Identitäten, ursprüngliche
Caddy-Konfiguration und geschützte Bestandsstacks erhalten. Nur die drei Lab-VMs
wurden durch dieses UI-Apply wieder gestartet; IQ-Store bleibt dreimal gestoppt.
Beleg: `evidence/lifecycle-f48-observe-ui-apply-20261020T184206.679011Z/result.json`,
SHA256 `90ca73dd28fbf96b1f7a3c7779e2a10a3e8cda6236ff4c120f4890dc7cf91deb`.

Nach gesonderter Root-Freigabe genau einmal die vorhandenen engen Consumerprobes
ausgeführt: VM/Docker/K3s jeweils PostgreSQL/S3/OIDC erfolgreich, beide HTTPS200.
Bestehende Daten zuerst lesend geprüft; anschließend Persistenz-, Binding-,
Runtime- und Eingabevergleich unverändert. Kein zusätzliches Apply oder Lifecycle.
Beleg: `evidence/post-ui-consumers-f48-20261020T184404.823467Z/result.json`, SHA256
`e805dfb334c549a279049b3079a9b4f00872fc0921e7c70ab6a028a2c8b4e845`.

Transparente Performancebeobachtung im abschließenden `stack-health.json`:
Worker-Agent ready, Service/Token okay, Queue 0; Exec-P99 8511 ms löst
`AGENT_DEGRADED` und Stack-Health degraded/Exit1 aus. Ops/Server sind okay.
`health.go` nutzt die letzten 32 Exec-Laufzeiten ohne zeitlichen Verfall;
P99 entspricht dort dem Maximum, Schwelle 5000 ms. Dies belegt eine langsame
gespeicherte Exec-Ausführung, keinen aktuellen Backlog oder defekten Agent.
Root führt es als Performancehinweis mit; kein Upgrade, Neustart oder künstliche
Exec-Serie zum Verdrängen des Messwerts. Keine weiteren Status-/Metrikprüfungen.

Runtime-Abnahmen Repeat, Late samt Events/MCP, vier Faults, Retention,
CP-Neustart, graceful Down und reales UI-Apply samt Nachprobes sind damit belegt.
Keine Prüfprozesse mehr aktiv. Anschluss ausschließlich GitHub-/Release-Abgleich;
externer macOS-26-Installationsnachweis und Auslieferungsentscheidungen bleiben
gemäß Abschlussplan getrennt offen. Gestoppte IQ-Store-VMs wurden nicht gestartet;
deren Trust-Abgleich bleibt beim regulären späteren Stack-Apply.
