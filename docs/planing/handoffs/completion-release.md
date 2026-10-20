# Release-Abschlussübergabe · 20.10.2026

Dieser Slice ist beendet. Auf Roots bestätigten finalen Runtime-/Consumer-PASS
wurden #54/#73/#74/#75 kriterienspezifisch aktualisiert und #77 als completed
geschlossen. #84 war bereits geschlossen. Keine neuen Kommentare, Builds,
Tests, Runtime-/vzctl-CLI-Abfragen, Commits/Pushes, PRs/Merges oder Releases.
Nur vorhandene Quellen/Belege sowie GitHub-Issue-Bodies und Status gepflegt.

Fachlicher Abschluss **A/B erfüllt**, Canvas **61/63**, F16/F17 offen.
Versions-/Dokumentationshäkchen bleibt bis zur Versionsentscheidung offen.
Root pflegt Canvas/12/10/README; Runtime-Agent 09 und seine Übergabe.

## Maßgebliche Abschlussbelege

Private `evidence/`-Pfade liegen unter `/Users/frank/vzctl-labs/controller-lab/`.
Die einzelnen Full-/Repeat-/Late-/MCP-/Fault-/Retention-Belege bleiben in
[09-live-update-acceptance.md](../09-live-update-acceptance.md) erhalten.
Keine erfolgreiche Prüfung wurde für alte offene Häkchen wiederholt.

| Abschluss | Beleg | SHA256 |
|---|---|---|
| CP-Neustart | `evidence/lifecycle-f48-cp-restart-20261020T182747.606692Z/result.json` | `3eae1d8a0d6d137ac28c6e9c60d029b5b04f400ecf300e6563b07572ed33148e` |
| Graceful Lab-down | `evidence/lifecycle-f48-down-20261020T182901.716132Z/result.json` | `129d0a760ae380095feb68763e0833f216cb899f07969dd242715911e27c7a97` |
| Lesender UI-Observer | `evidence/lifecycle-f48-observe-ui-apply-20261020T184206.679011Z/result.json` | `90ca73dd28fbf96b1f7a3c7779e2a10a3e8cda6236ff4c120f4890dc7cf91deb` |
| Finale Runtime-/Consumer-Nachabnahme | `evidence/post-ui-consumers-f48-20261020T184404.823467Z/result.json` | `e805dfb334c549a279049b3079a9b4f00872fc0921e7c70ab6a028a2c8b4e845` |

Root hat diese Belege gelesen und ihre Hashes geprüft. Final alle neun
PostgreSQL-/S3-/OIDC-Probes für VM/Docker/K3s erfolgreich, HTTPS erfolgreich;
Daten, Bindings/Credentials und geschützte Identitäten erhalten. Neun Ready
plus bekannter foreign Retained, Journal Generation 14 done; Neti-PIDs
unverändert, drei IQ-Store-VMs weiterhin gestoppt.

Nativer UI-Apply genau einmal **20:39:54–20:40:31 MESZ (37s)**, Invocation
`ebbea683-ed47-4c7a-9cec-3c21050f1917`. Debug ↔ Betrieb während aktivem Job
bedienbar; Log 50% → 95% → fertig; Aktionen währenddessen gesperrt und danach
freigegeben. UI zeigt Apply fertig, 3/3 Lab-VMs und Drift0. Eigener nativer
Nachweis: [ui-native-apply-2026-10-20.md](../ui-native-apply-2026-10-20.md).

**Erhaltene Hinweisgrenzen:** Worker-Agent bereit, Token/Service erreichbar,
queue_depth=0; historische Exec-P99 **8511ms** führt weiterhin zu degraded bzw.
Stack-Warnung/Exit1. Kein belegter aktueller Funktionsausfall, kein Neustart oder
Upgrade zur Kosmetik. Keine Behauptung eines vollständig gesunden Gesamtzustands.
`global_trust_complete=false` bleibt für die drei bewusst gestoppten IQ-Store-
Gäste sichtbar; Aktualisierung beim späteren regulären Stack-Up/Apply.

## Noch offene Kriterien — genau diese

| Issue | Noch offen |
|---|---|
| [#54](https://github.com/frankhildebrandt/vzctl/issues/54) | Geprüften UI-Arbeitsstand committen/ausliefern, auf main oder per gemergtem PR bereitstellen. ui-dev-Start-Smoke und langer nativer UI-Apply bereits belegt. |
| [#73](https://github.com/frankhildebrandt/vzctl/issues/73) | Übergreifende Repository-/Releaseübergabe, Versionsentscheidung mit finaler Dokumentationssynchronisierung sowie externer PKG-Nachweis. Fachliche A/B-Kriterien erfüllt; 61/63, nur F16/F17 offen. |
| [#74](https://github.com/frankhildebrandt/vzctl/issues/74) | Änderungen im Repository ausliefern. K3s-/Manifest-/Kubeconfig-/Lifecycle-Kriterien erfüllt. |
| [#75](https://github.com/frankhildebrandt/vzctl/issues/75) | Änderungen im Repository ausliefern. Controller-/Binding-/MCP-/Event-/Fault-/Retention-/Lifecycle-/UI-Kriterien erfüllt. |
| [#76](https://github.com/frankhildebrandt/vzctl/issues/76) | Saubere PKG-Erstinstallation und reales PKG-Update mit Wiederanlauf auf separatem macOS-26-Mac; derzeit nicht vorhanden. Zusätzlich Repository-Auslieferung. |

#54-Nice-Punkte Deep-Link, Fensterpersistenz und konkreter edge-dmz-Live-Up
bleiben optional und getrennt; sie erzeugen kein neues Abschlussarbeitspaket.
Commit/Push, PR/Merge, Release-Version und Veröffentlichung sind nicht beauftragt
und nicht ausgeführt. Kein Repo-/Installerkriterium durch Issue-Schließen verdeckt.

## Offenes Inventar / Dependency-PRs

Die repositoryweite Connectorabfrage (Issues/PRs jeweils Grenze 100) lieferte vor
#77-Abschluss sechs Issues #54/#73/#74/#75/#76/#77 und neun offene PRs.
Alle Issues sind in `06-github-issue-map.json` enthalten; kein Issue fehlte.
Nach gezieltem Abschluss von #77 bleiben **fünf offene Issues: #54/#73/#74/#75/#76**.
Die Map enthält stabile IDs und benötigt keine Änderung.

Die neun PRs sind nur mit Status/Titel erfasst; keine neue Check-/Review-/
Mergeprüfung und kein Merge. Lokale D5/D6-Updates sind bereits geprüft.
Mögliche Überschneidungen gehören erst zu einer konkret beauftragten
Repository-Auslieferung; **keine neun neuen Releaseblocker oder Arbeitspakete**.

| PR | Status | Titel |
|---|---|---|
| [#58](https://github.com/frankhildebrandt/vzctl/pull/58) | offen | chore(deps-dev): Bump esbuild from 0.27.7 to 0.28.1 in /apps/vzctl-ui |
| [#62](https://github.com/frankhildebrandt/vzctl/pull/62) | offen | chore(deps): Bump esbuild, @astrojs/starlight and astro in /apps/docs |
| [#64](https://github.com/frankhildebrandt/vzctl/pull/64) | offen | chore(deps): Bump svgo from 4.0.2 to 4.1.0 in /apps/docs |
| [#66](https://github.com/frankhildebrandt/vzctl/pull/66) | offen | chore(deps): Bump sharp, @astrojs/starlight and astro in /apps/docs |
| [#67](https://github.com/frankhildebrandt/vzctl/pull/67) | offen | chore(deps-dev): Bump vitest from 4.1.10 to 4.1.11 in /apps/vzctl-ui |
| [#68](https://github.com/frankhildebrandt/vzctl/pull/68) | offen | chore(deps): Bump js-yaml from 4.3.1 to 4.3.2 in /apps/docs |
| [#70](https://github.com/frankhildebrandt/vzctl/pull/70) | offen | chore(deps-dev): Bump undici from 7.29.0 to 7.30.0 in /apps/vzctl-ui |
| [#71](https://github.com/frankhildebrandt/vzctl/pull/71) | offen | chore(deps): Bump devalue from 5.9.0 to 5.9.4 in /apps/docs |
| [#72](https://github.com/frankhildebrandt/vzctl/pull/72) | offen | chore(deps): Bump source-map-js from 1.2.1 to 1.2.2 in /apps/docs |

## Vorhandene Belege wiederverwenden

Full/MCP, Repeat Gen10, Late mit neun Ready-Claims, vier Faultfälle,
Retention Gen12 und Kubeconfig-Export sind abgeschlossen. Detailpfade und Hashes
in [09](../09-live-update-acceptance.md), [Runtime-Übergabe](completion-runtime.md)
und den jeweiligen GitHub-Bodies. Gemeinsame Abnahmen wurden genau einmal
gezählt, historische Verifierfehler behalten und nicht als unbelegte
Produktfehler behauptet. Der kurze Quellencheck zum Stop-Wait ergab keinen
`stopping`-basierten Produktfehler: der Helper-/RPC-Vertrag erlaubt diesen
Zustand nicht; keine Produktänderung daraus abgeleitet.

Aktuelle Dev-Pakete und ihre Grenzen stehen im
[Paketprüfbericht](../package-verification-2026-10-20.md). Lokale Inhaltsprüfung
und Entwicklungsinstallation ersetzen den externen PKG-Test nicht.
Nächste Entscheidung liegt beim Nutzer: konkrete Repository-Auslieferung bzw.
Release-Version; keine technische Runtime-Restkette offen.
