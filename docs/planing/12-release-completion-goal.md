# Arbeitsziel: vzctl bis zur belegten Release-Übergabe abschließen

Stand: 20.10.2026. Fortsetzung vom Nutzer ausdrücklich freigegeben.
Dieser Abschlussplan ist maßgeblich; 09–11 und die Übergaben bewahren den Verlauf.
Neue Nutzeraufträge werden hier und im bestehenden Canvas aufgenommen.

## Ziel und Umfang

Den vereinbarten Funktionsumfang fertigstellen und am installierten System
abnehmen: Review-Fixes, CLI/UI/MCP und Agent-Bedienung, fachliche Aufteilung
der vier großen Dateien, K3s mit stackbezogenen Manifesten aus
hypernetwork.config.yaml sowie HTTPS-, OIDC-, PostgreSQL- und S3-Controller
für VM, Docker und K3s. Danach Kriterien/Issues und Dev-Pakete abgleichen
und einen konkreten Übergabestand liefern.

Keine vereinbarte Anforderung entfällt. Erstes veröffentlichtes Release und
vollständige externe Installer-Abnahme werden nicht als erledigt ausgegeben.

## Abschlussdashboard

**61 von 63 Einträgen fachlich abgeschlossen.** Die zwei offenen Einträge
F16/F17 bündeln Repository-Auslieferung und Release; es fehlen keine sieben
Controller-Implementierungen oder weitere Lab-Testläufe.

| Block | Ergebnis | Noch offen |
|---|---|---|
| A · Laufzeit und Controller | Abgeschlossen: Full/Repeat/Late, vier Besitz-/Ausfallfälle, Retention, Supervisor-Neustart, Lab-Down und UI-Wiederanlauf; finale Verbraucher erfolgreich | Kein weiterer interner Prüflauf vorgesehen |
| B · Bedienung und Verträge | Abgeschlossen: native UI, CLI/MCP, privater Kubeconfig-Export, Diagnose und Events;37 s UI-Apply mit bedienbarer Navigation, Fortschritt/Log und Aktionssperre | Kein weiterer interner Prüflauf vorgesehen |
| C · Übergabe und Release | Dev-TAR/PKG/DMG geprüft; Nachweise und Issue-Kriterien abgeglichen; Canvas aktualisiert | Commit/Push bzw. main/merged PR, Release-Version/Veröffentlichung, sauberer macOS-26-PKG-Test |

## Belegter Endzustand

- UI-Apply genau einmal 20:39:54–20:40:31; invocation und Journal
  ebbea683-ed47-4c7a-9cec-3c21050f1917, Generation14 done.
- Danach VM/Docker/K3s jeweils PostgreSQL/S3/OIDC erfolgreich, HTTPS200.
  Daten, Bindings und Credentials erhalten; neun Ready-Claims plus bewusstes
  foreign Retained. Kein weiteres Apply/Reconcile für die Abnahme.
- Sechs VMs laufen: Lab 3 und Neti 3. Neti-Identitäten unverändert.
  IQ-Store 3 bleibt absichtlich gestoppt; Guest-Trust dort erst beim regulären
  späteren Up/Apply. Keine Behauptung global vollständig aktualisierten Trusts.
- Worker meldet einen Performancehinweis: Exec-P99=8511ms in den letzten
  höchstens 32 Ausführungen, Queue0, Agent ready, Token/Service erfolgreich.
  Der Wert altert nicht zeitlich aus. Kein belegter aktueller Backlog oder
  Agent-Ausfall; keine künstlichen Calls/Neustarts zum Grünmachen.
- Installierte CLI und Paket-CLI stammen aus demselben Produktstand, sind nach
  erneutem Workspace-Build/Signierung byteweise verschieden. Keine Gleichsetzung
  ihrer Hashes oder Behauptung eines bereits veröffentlichten Pakets.

| Nachweis | Referenz |
|---|---|
| Funktion, Faults, Retention, Lifecycle und Einzelhashes | [Live-Abnahme09](09-live-update-acceptance.md) |
| Native UI-Bedienung und Screenshots/AX | [UI-Apply-Abnahme](ui-native-apply-2026-10-20.md) |
| Finaler lesender Runtime-Observer | SHA256 90ca73dd28fbf96b1f7a3c7779e2a10a3e8cda6236ff4c120f4890dc7cf91deb |
| Finale Consumer nach UI-Wiederanlauf | SHA256 e805dfb334c549a279049b3079a9b4f00872fc0921e7c70ab6a028a2c8b4e845 |
| Aktuelle Pakete, Signaturen und Grenzen | [Paketprüfbericht](package-verification-2026-10-20.md) |
| GitHub-Kriterien und offenes Vollinventar | [Tracking06](06-github-tracking.md) |
| Fortsetzungsauftrag | [Nächster Schritt10](10-next-slice.md) |

## Arbeitsregeln und Zuständigkeiten

1. Höchstens zwei neue Arbeitspakete gleichzeitig; Runtime-Mutationen nur
   nacheinander unter einer Zuständigkeit.
2. Fertig-Kriterium vor Start benennen, danach abschließen. Erfolgreiche
   Nachweise übernehmen; Wiederholung nur bei konkreter Änderung/Fehler/Lücke.
3. Keine neuen Runnerketten/Testframeworks und keine beiläufigen Optimierungen.
   Prüfskriptfehler von Produktfehlern trennen; fehlende Checks möglichst lesend.
4. Nach Abschluss: Kriterien/Issues, Canvas und Folgeauftrag aktualisieren.
   Sammelaufgaben nicht als neue Produktfehler zählen.
5. Alte Subagents haben Angefangenes übergeben. Neue Agents erben das aktive
   Parent-Modell ohne stillen Override.
6. completion_runtime hat die finale Live-Abnahme geliefert; completion_release
   den Kriterienabgleich. Parent integriert Dashboard, Ziel und Übergabe.
   Nach Abschluss keine weiteren Runtimeaktionen ohne neuen konkreten Auftrag.

## Konkrete externe Abschlussbedingungen

- Commit/Push und Draft-PR sind jetzt ausdrücklich beauftragt; Branch
  codex/release-readiness-2026-10-20 auf Basis 76f833d. Merge bleibt offen.
- Release-Version und Veröffentlichung sind eine gesonderte Entscheidung.
  Dev-Pakete 0.0.1/dev sind keine veröffentlichte Release-Version.
- Nutzer hat keinen separaten Mac mit macOS 26. Saubere PKG-Erstinstallation
  und echtes PKG-Update/Wiederanlauf bleiben in #76 extern offen.
  Lokale Inhaltsprüfung und Entwicklungsinstallation ersetzen sie nicht.

Die technische Restabnahme ist beendet. Release-Bereitschaft inklusive dieser
externen Bedingungen bleibt offen, bis sie tatsächlich erfüllt sind.


## Formaler Zielabgleich nach Abschluss

Der vorherige Zielturn war Fortschritt: tatsächlicher UI-Wiederanlauf und
Consumer-Abnahme, GitHub-Kriterienänderungen und #77-Abschluss sowie Canvaspflege.
Der abschließende Abgleich hat die Quellen und vorhandenen Belege gelesen;
er hat keinen erfolgreichen Test oder Live-Eingriff wiederholt.

| Vereinbarte Anforderung | Aktueller Beleg / Ergebnis |
|---|---|
| F1–F10 beheben und angemessen verifizieren | Jeweilige Quellen-/Test-/Live-Nachweise im Canvas und in09; alle zehn completed. Vorhandener finaler Rustlog tatsächlich gelesen:20 Suites,496 passed,0 failed,2 ignored. UI-Gesamtsuite149 passed sowie spätere gezielte Regressionen; Swiftlog alle ausgeführten Tests erfolgreich. Keine Behauptung vollständiger Live-Abdeckung aller Einzel-CLI-Verben |
| Vier große Dateien fachlich teilen, Verträge absichern | Hauptdateien aktuell531/443/769/427 Zeilen; Fachmodule vorhanden. Rust-/Swift-Verträge und Regressionen belegt. Zeilenzahl ist nur ein Identitätsmerkmal |
| Installierte CLI und native UI prüfen | Installierte CLI-/App-Hashes und unveränderte Lab-Konfiguration erneut abgeglichen. Native Screenshots/AX und37s Apply einschließlich Navigation/Log/Ende; finale Runtime-/Consumer-Receipts mit unveränderten Hashes geprüft |
| K3s, stackbezogene Manifeste, vier Controller für VM/Docker/K3s | Schema/Beispiel/Implementierung vorhanden; Full/Repeat/Late, Ownership/Faults, Retention und Lifecycle belegt. Finale neun PostgreSQL/S3/OIDC-Probes erfolgreich, HTTPS erfolgreich |
| Agent-Bedienung, Status, Diagnose, Events, Kubeconfig | Reale MCP-Aufrufe, semantische Events, redigierte Diagnose und privater TLS-Kubeconfig-Export in09 belegt; historische Worker-Latenzwarnung transparent erhalten |
| Bestehende und gestoppte Stacks respektieren | Finale Host-Receipts erhalten Neti-PIDs und drei gestoppte IQ-Store-VMs; fehlender Trust dort sichtbar bis regulärem späteren Up/Apply |
| Canvas/Todos, neue Aufgaben, Subagents, GitHub und Folgearbeit pflegen |63 eindeutige Einträge,61 completed, ausschließlich F16/F17 offen, jeder Eintrag mit Nachweis. Canvas real gerendert/typegeprüft; neue Agents mit Übergaben fertig. GitHub-Schreibvorgänge zurückgelesen, fünf offene Issues mit einzeln benannten Restkriterien; konkreter Folgeauftrag10 |
| Aktuelle Dev-Pakete und Releaseübergabe | TAR/PKG/DMG-Inhalts-/Signatur-/Hashprüfung und sieben Packagingtests bestanden. Version0.0.1/dev, nicht veröffentlicht; Paket-/Live-Binary-Grenze und fehlender sauberer Installer-Test ausdrücklich dokumentiert |
| Noch nicht ausführbare Kriterien offen ausweisen | F16/F17 bleiben offen. Repository-Verfahren/Versionsentscheidung beim Nutzer angefragt; separater macOS-26-Mac fehlt laut Nutzer. Keine Freigabe aus ausbleibender Antwort abgeleitet |

Das gespeicherte Ziel erlaubt den Abschluss ausdrücklich, wenn vereinbarte
Aufgaben erledigt **oder konkrete externe Hindernisse transparent ausgewiesen**
sind. Dieser Review-/Abnahmelauf erfüllt diese Bedingung. Das ist kein Nachweis
eines veröffentlichten ersten Releases und schließt die offenen fünf
Auslieferungs-/Installer-Issues nicht. Commit/Push und Draft-PR wurden anschließend ausdrücklich beauftragt;
aktueller Übergabestand siehe Repository-Übergabe unten.

## Repository-Übergabe · 20.10.2026

Der Nutzer hat Commit, Push und einen Draft-PR ausdrücklich freigegeben.
Branch: codex/release-readiness-2026-10-20, Zielbranch: main.
Der zuvor geprüfte Gesamtstand wird übernommen. Runtime und Release-Version
bleiben unverändert. PR-Link und Commit werden nach Erstellung ergänzt;
Merge, Tag und Veröffentlichung sind kein Teil dieses Auftrags.


## Folgeauftrag freigegeben: Merge, Release und lokale Installation

Der Nutzer hat nach Commit/Push/Draft-PR zusätzlich Merge, Release und lokale
Neuinstallation beauftragt. Dafür sind R2–R4 im Canvas ergänzt; aktueller
Stand61/66. Die abgeschlossene technische Abnahme bleibt bestehen.
Zielrelease: v0.3.0-alpha4, Fortsetzung nach v0.2.0-alpha3.
Eigene Komponenten werden konsistent versioniert; Release-Pakete entstehen
neu aus diesem Quellstand. Nach grünem PR/CI erfolgt Merge und Veröffentlichung;
danach lokales Update mit Wiederherstellung der zuvor aktiven Stacks.
IQ-Store bleibt gestoppt. Der separate saubere macOS-26-PKG-Test bleibt offen.

## Aktueller Auslieferungsblocker

PR #85 ist bereit. Zwei CI-Läufe bauten Swift erfolgreich, hingen danach
über31 Minuten ohne Testausgabe; kontrolliert abgebrochen. Nur den Runner
umzustellen reichte nicht als Ursachenbeweis. Mit
`LIBDISPATCH_COOPERATIVE_POOL_STRICT=1` lokal reproduziert: Der einzige
kooperative Worker blockiert in `SupervisorReporter.send`/`read` innerhalb
`runArgumentFailureIsReportedBeforeRunStarts`, während sein eigener
Socket-Testserver als `Task.detached` auf denselben Worker wartet.
Prozess-Sample: `/private/tmp/vzctl-swift-pool-strict.sample`.

Beide blockierenden Socket-Testserver verwenden nun `DispatchQueue.global`.
Alle213 Swift-Tests bestehen warningfrei auch mit nur einem kooperativen
Worker; der CI-Schritt verwendet denselben Stressfall und ein5min-Limit.
Die CI führt weiterhin alle bisherigen Ziele aus, mit getrennten Phasen.
Remote-Nachweis ist vor Merge offen.
