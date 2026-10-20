# CLI-Release-Matrix für #17

Stand: 20.10.2026, lokaler Quellstand und Release-Prüfbau `0.0.1`.
Zuerst Read-only-Abgleich; anschließend gezielter Contract-Fix mit
Prozessregressionen. Keine neuen Produktfeatures, Installationen oder Paketänderungen.

**Ergebnis:** Die registrierten Funktionen haben Handler. Drei ausdrücklich
abzugrenzende Hinweis-Commands bleiben ohne eigenständige Funktion:
`certs rollout`, `certs verify`, `oidc token`. Die beim Erst-Audit gefundenen
Korrekturen R1–R6 sind umgesetzt; die nachfolgende Matrix bewahrt Befund und
Abnahmekriterien. Neue CA-/OIDC-Features sind dafür nicht erforderlich.

## Abschluss des Contract-Slices

| Punkt | Ergebnis |
|---|---|
| R1 Fehlerausgabe | `main`, Stack, Route, Status-Alias, Port und Services liefern die Fehlerdaten auf stdout, Pflichtfelder und unveränderte Exitcodes. Stack-Validierungsfehler behalten ihre strukturierte `errors[]`-Liste. Unbekannte Verben verwenden den bekannten Namespace; unbekannte Root-Commands `unknown`. Controller/Kubernetes-Namespace-Namen ebenfalls korrigiert |
| R2 Status-Semantik | Bestehende `stack.status warn/1`, Kubernetes-/Controller-`warn/24` bleiben erhalten; zentrale Spec erklärt Bereitschaft, Ausführungsfehler und Watch-Zykluscode |
| R3 Rebake | Hilfe/Spec ergänzt; alle vier Fehlerausgänge verwenden `image.rebake`. Parsefehler und fehlender lokaler Alias per CLI-Prozess geprüft; Bake-/Seal-Verhalten unverändert |
| R4 Streams/Help | Stack-Watch-Ausnahme und Controller-NDJSON ausdrücklich dokumentiert; Help-Parität um Diagnose/Controller/Kubernetes ergänzt |
| R5 Stubs | Keine neuen Funktionen; Hilfe und CA-/OIDC-Spec nennen die drei Verben ausdrücklich nicht unterstützt. Fail/Exit2-Regressionsfälle grün |
| R6 Regressionen | `tests/cli_error_contract.rs`: acht Prozessintegrationstests, darunter 44 positionsabhängige Parserfälle, 13 Namespace-Familien, Stack-/Route-Datei-/RPC-Fehler, Port-Erfolg/Fehler, Controller Ready/Unavailable und zwei Watch-Fehlerzyklen mit sauberem SIGINT-Ende. Bestehender Stack-Test liest Fehler-JSON jetzt korrekt aus stdout |

Test-first: Vor dem Fix scheiterten fünf der zunächst sechs neuen Tests.
Der später erweiterte Namespace-Test zeigte zusätzlich
`controller.unknown`/`kubernetes.unknown`; auch diese Namen sind korrigiert.
Test-Fixtures verwenden einen atomaren Sequenzzähler zusätzlich zum Zeitstempel,
damit parallele Starts keine temporären Verzeichnisse teilen.

Abschließende Abnahme:
`RUSTFLAGS='-D warnings' cargo test -p vzctl --tests` →
**395 bestanden, 0 Fehler, 1 ignorierter Netzwerktest** in 15 Testsuites.
`cargo fmt --all -- --check` ebenfalls grün. Keine Compiler-Warnings.

## Grundlage und Aussagegrenze

- Registrierung: `crates/vzctl/src/main.rs`, `image_cli.rs`, `vm_create.rs`,
  Namespace-Dispatcher und `help.rs`.
- Vertrag: `docs/specs/cli-contract-v1.md`, `events-v1.md`,
  `certs-v1.md`, `oidc-v1.md`, `service-controllers-v1.md`,
  `docs/images/bake-contract-v1.md`.
- Tests: vorhandene Unit-, Golden- und Prozessintegrationstests unter
  `crates/vzctl/src/` und `crates/vzctl/tests/`. „Handler vorhanden“ bedeutet
  nicht, dass jeder Live-Erfolgsweg automatisiert getestet wurde.
- 92 unterschiedliche Command-Pfade wurden zusätzlich mit
  `--audit-unknown --format json` in einem temporären, leeren `HOME` und
  `VZCTL_STATE_DIR` aufgerufen. Die absichtlich ungültigen Optionen verhindern
  Mutationen und Supervisor-/Guest-Aufrufe. 70 lieferten ein vollständiges
  stdout-Envelope mit passendem Prozess-Exitcode; 22 nicht. Einer dieser 22
  ist `stack watch`, das als NDJSON-Command kein `--format` akzeptiert.
- Zusätzliche sichere Einzelproben bestätigten die drei Hinweis-Commands,
  `image rebake`, `stack watch --format json` sowie unbekannte Image-/VM-Verben.
  Keine Live-CA-/Keychain-, VM-, Docker-, K3s- oder Service-Mutation.
- Die separat gelaufene Rust-Gesamtsuite war grün (395 bestanden,
  ein ignorierter Netzwerktest); sie beweist nicht die hier neu gefundenen,
  bislang ungetesteten Fehlerzweige.

## Registrierter Umfang und Absicherung beim Erst-Audit

`I` = eigener CLI-Prozess mit Fixture/Mock; `U` = Unit-/Golden-Test.
Alle Pfade sind repository-relativ. Die Tabelle nennt Familien vollständig,
fasst Varianten mit demselben Handler zusammen und benennt Testlücken.

| Command-Familie / registrierte Verben | Handler | Vorhandene Vertragsabsicherung | Konkreter Rest |
|---|---|---|---|
| `help`, `help <topic>`, `<namespace> help`, `help exit-codes`; ohne Argumente Root-Hilfe | `help.rs` | I: `help_cli.rs` (Root, Namespace-Gleichheit, Exitcodes, unbekanntes Topic) | Neue Topics `diagnose`, `controller`, `kubernetes` fehlen in der Namespace-Paritätsliste; R6 |
| `version`, `doctor` | `main.rs`, `doctor.rs` | U: `cli_tests.rs`, Golden `version_json_matches_golden_contract`, `doctor_json_matches_golden_contract`, WARN/Hard-Fail-Mapping | Frühe Parserfehler ohne JSON; R1 |
| `validate`, `validate --schema` | `config.rs` | I: `validate_cli.rs` positive/negative/schema Fixtures | Keine neue Lücke im geprüften Umfang; Schemaexport ist bewusst kein CLI-Envelope |
| `plan`, `diff`, `up`, `apply`, `down`, `adopt` | `reconciler.rs` und `reconciler/*` | I: `reconciler_cli.rs`, `edge_dmz_cli.rs`; U: `reconciler/tests.rs` (Resume/Input/Lease/Plan) | Mutierende Erfolgswege zusätzlich Live-Abnahme; keine fehlende CLI-Funktion |
| `stack init`; `stack vm/net/volume/mount add/remove` | `stack.rs` | I: `stack_cli.rs` (Scaffold, vorhandene Datei, VM/IP/RAM, Net-Referenzfehler, Volume/Mount-Roundtrip) | Fehler-JSON auf stderr und ohne `summary`; R1 |
| `stack status`, Alias `status`; `stack watch` | `observability.rs` | U: Health-/Ingress-/Routing-/Scope- und Watch-Filter-Tests | Status-Parserfehler ohne JSON; Watch-Hilfe behauptet fälschlich `--format`; R1/R4 |
| `image list/pull/bake/seal` | `image_cli.rs`, `image/*`, `builder.rs` | U: `cli_tests.rs` (Pull-/Seal-Parser und Golden), Image-/Builder-Fixtures | Lokaler/VM-Bake-Erfolg nicht als vollständiger CLI-Prozesstest abgedeckt; kein Stub |
| **`image rebake`** | `image_cli.rs::image_rebake_command` | Handler erzwingt Bake + Seal; Bake-Doku erwähnt ihn | Topic-Hilfe und CLI-Spec fehlen; Fehler heißen `image.bake`/`image.seal`; kein eigener Rebake-Test; R3 |
| `vm create` | `vm_create.rs`, `cloud_init.rs` | U: `cli_tests.rs` (Golden v2, Disk/Identity/Resources/Mounts/Cloud-init, Native-APFS-Fixtures) | Kein v1-Zwang: `vm.create` ist ausdrücklich v2 |
| `vm list/start/stop/restart/delete/inspect`, Host-Alias `ps` | `vm/{inventory,lifecycle}.rs` | I: `vm_lifecycle_cli.rs`; U: `vm/tests.rs` (Mock-Lifecycle, Orphan/Delete, Manifest) | Restart ist zusammengesetzt; kein eigener vollständiger Restart-Prozesstest |
| `vm exec/transfer/attach/services/ps`; `vm services status/start/stop/restart` | `vm/{guest,terminal}.rs` | I: `vm_lifecycle_cli.rs` Exec/Inspect/Systemd/Guest-PS/Transfer; U: TTY-Parser/Detach | Echte PTY-/Console-/Guest-Systemd-Mutationen bleiben Live-Abnahme; Ausnahmen vom Envelope dokumentiert |
| `vm logs` inkl. Source-/Follow-/Filter-Optionen; `vm modify`; `vm mount/unmount/mounts`; `vm agent upgrade`; `vm probe/health/stats` | `vm/{logs,lifecycle,mounts,guest}.rs`, `guest_utils.rs` | U: Serial-Tail/Redaktion, Modify/Manifest, Agent-Upgrade-Parser; unterstützende Guest-/Mount-Tests | Keine vollständige CLI-Erfolgsmatrix für diese einzelnen Guest-RPCs; kein Stub. Serial-Follow + JSON ist dokumentiert Exit3 |
| `net create/attach/list/detach/delete`, `net default show/set` | `network.rs` | U: Parser, Default-/List-Envelope; Netzwerk-Integration über Stack/VM-Tests | Nicht jedes Primitive hat einen eigenen Prozess-Erfolgstest; kein fehlender Handler |
| `route apply/plan/status` | `route.rs` | U: Parser/Policy-/Status-Envelope | Alle Fehler auf stderr, `status=error`, fehlende `summary`; R1 |
| `dns status/query/install-resolver/uninstall-resolver/install-bind-helper/uninstall-bind-helper` | `dns.rs` | U: Status-RPC, UDP-A/AAAA/PTR/NXDOMAIN, Query-Golden, Resolver-Ownership/Idempotenz/Reverse-Scope, Bind-Helper-Parser/Plist | Root-/LaunchDaemon-Weg nicht in CLI-Livetest; kein Stub |
| `docker ps/inspect/start/stop/restart/run`, expliziter und unbekannter Passthrough | `docker.rs` | I: `identity_cli.rs::docker_json_errors_work_before_a_context_or_engine_exists`; Parser/Builder im Modul | Kein umfassender Fake-Docker-Prozesstest für alle strukturierten Erfolgsverben; Passthrough ist explizite Ausgabe-/Exitcode-Ausnahme |
| `port list` | `port.rs` | Handler/RPC vorhanden; kein eigener Unit-/Prozess-Vertragstest in `port.rs` oder `tests/` | Früher Parsefehler ohne JSON; R1/R6 |
| `services status/start/stop/restart [all/net/edge/supervisor]` | `services.rs` | I: `services_cli.rs`; U: Zielparser und Start-/Stopreihenfolge; separate Installer-Fixtures | Frühe Parser-/Zielfehler ohne JSON; R1. Echte LaunchAgents nicht durch diesen Audit betätigt |
| `certs ca init/install`, `certs mint/fingerprint` | `certs.rs` | I: `identity_cli.rs` lokale CA/Mint/Fingerprint plus Fehler; U: Zertifikats-/SAN-Tests | Keychain-Install ist eigener privilegierter Live-Weg; drei Stubs siehe unten |
| `oidc status/clients` | `oidc.rs` | I: `identity_cli.rs` Status/Clients/invalid YAML; U: stabile Secrets/Claims/Ownership | Keine neue Vertragslücke für diese beiden implementierten Verben |
| `debug sources list`, `debug doctor`, `debug postgres/mysql query`, `debug nats streams/sub/kv get/kv watch` | `debug/{mod,sources,sql,nats,prereq}.rs` | U: SQL-Read-only-/Multi-Statement-Prüfung; Config-/Quellenvalidierung | Kein eigener Prozess-Vertragstest für DB-/NATS-Erfolgsdaten; bounded Watch/Sub sammelt ein JSON-Dokument, kein offener Stream |
| `events subscribe` | `events.rs` | I: `events_cli.rs` fragmentiertes NDJSON/additive Typen; U: Filter/Options | Eigenständiger Event-Stream-Vertrag, #19; nicht als fehlendes Ein-Dokument-JSON zählen |
| `diagnose --output` | `diagnose.rs` | I: `diagnose_cli.rs` explicit/private/bounded/no-overwrite; U: Projektion/Redaktion | Implementiert; kein neuer Feature-Rest |
| `kubernetes status/kubeconfig` | `kubernetes/cli.rs` | I: `kubernetes_cli.rs` privater Export/no-secret stdout und Degraded/Exit24 | In zentraler CLI-Spec fehlen Payload/Exits; WARN-Ausnahme R2 |
| `controller status/reconcile/watch` | `controllers/runtime.rs` | I: `controller_status_cli.rs`; U: Runtime-/Storage-/Ownership-/Provider-Tests | Zentrale CLI-Spec und Watch-NDJSON-/Exitsemantik fehlen; R2/R4 |
| `skill`, `skill --install-local/--install-global` | `skill.rs` | I: `skill_cli.rs` Export/Help/Flags/Install nach Temp-HOME | Bewusst Klartext, kein JSON-Envelope; kein Rest |

## Befund und Abnahmekriterien R1–R6 (inzwischen umgesetzt)

### R1 — Fehlerausgabe bestehender Commands vervollständigen

Kein neues Feature: bestehende Fehlerpfade müssen denselben stdout-Vertrag
wie Erfolgsantworten einhalten. Prozess-Exitcodes beibehalten.

| Betroffene Oberfläche | Heute reproduziert / Quelle | Abnahmekriterium |
|---|---|---|
| `version` | Ungültiges Flag + JSON → Exit2, stdout leer; `main.rs` | Ein stdout-Envelope `version`, `fail`, Exit2, `summary`; stderr nur Diagnostic |
| `doctor` | Ungültiges Flag + JSON → Exit3, stdout leer; `main.rs` | Entsprechend `doctor`, Exit3 |
| `stack init`, `stack {vm,net,volume,mount} {add,remove}` | Alle neun Pfade: Fehler-JSON **auf stderr**, ohne `summary`; `stack.rs::emit_failure` gilt auch für Validierungs-/Dateifehler | Vollständiges Envelope auf stdout für Parse- **und** Laufzeitfehler; bestehende Tests dürfen JSON nicht mehr aus stderr erwarten |
| `route plan/apply/status` | Fehler-JSON auf stderr, `status=error` außerhalb des Enum, keine `summary`; `route.rs::emit_failure` | stdout, `status=fail`, `summary`, identischer Exitcode; Runtime-/RPC-Fehler ebenfalls |
| `stack status`, Alias `status` | Frühe Parsefehler stdout leer; `observability.rs::stack_status_command` | Kanonisch `stack.status`, fail/Exit2 bzw. vorhandener spezifischer Parsercode |
| `port list` | Frühe Parsefehler stdout leer; `port.rs::command` | Kanonisch `port.list`, fail/Exit2 |
| `services status/start/stop/restart` | Frühe Parse-/Ziel-Fehler stdout leer; `services.rs::command` | Envelope bei Usage2 und ungültigem Ziel3 |
| Namespace ohne/mit unbekanntem Verb | Insbesondere `image nope --format json` liefert nur stderr; `vm nope` bereits JSON | Regel für nicht auflösbare/fehlende Verben in der Spec festlegen und Dispatcher einheitlich testen. Empfehlung: vorhandener Namespace als `command`, fail/Exit2; Root-unbekannt explizit dokumentieren |

Die ersten sieben Zeilen betreffen **21 konkrete JSON-fähige Pfade**.
Der 22. fehlende stdout-Envelope der Probe ist die gewollte
`stack watch`-Formatausnahme und gehört zu R4.

### R2 — Status-Ausnahmen ausdrücklich dokumentieren, Nonzero beibehalten

Die pauschale Regel „WARN hat Exit0; Hard-Fail muss `status=fail` sein“
passt bereits nicht zum bestehenden `stack.status`. Sie darf nicht durch
Umstellen aller Statusprüfungen auf Exit0 scheinbar repariert werden.

| Command / Zustand | Implementiert | Empfehlung |
|---|---|---|
| `stack.status` / Alias `status`: ok, degraded, critical | `ok/0`, `warn/1`, `fail/2` | Beibehalten; zentrale WARN-Regel um dokumentierte Health-Ausnahme ergänzen |
| `kubernetes.status`: alle ready / mindestens ein Cluster nicht ready | `ok/0`, `warn/24`; echte Fehler `fail/<Code>` | Beibehalten; Payload `data.clusters`, `summary.clusters`, Fehlercodes2/3/24 und Degraded-Semantik dokumentieren. Bestehender CLI-Test sichert warn/24 |
| `controller.status` und `controller.reconcile`: alle Claims Ready/Retained und keine Warnungen / sonst | `ok/0` bzw. `warn/24`; Ausführungsfehler `fail/24`, Parsefehler2 | Beibehalten und explizit dokumentieren. Pending/Failed/Warnungen bleiben für CI/Agenten Nonzero |
| `controller.watch --format json` | Ein Envelope pro Zyklus; einzelne Records warn/fail mit24, Prozess läuft weiter; Ctrl-C beendet mit0 | Als NDJSON-Snapshot-Stream spezifizieren: Record-`exit_code` bewertet Zyklus, finaler Prozesscode bewertet Streambeendigung. Keine Gleichheit jedes Records mit finalem Prozesscode versprechen |

`kubernetes kubeconfig` braucht zusätzlich die dokumentierten Argumente
`--cluster`, `--output`, `-C`, privaten create-new Export und die tatsächlichen
Fehlercodes2/3/24 in der zentralen CLI-Spec. Die existierende
Service-Controller-Spec allein definiert diesen CLI-Vertrag nicht vollständig.

### R3 — Vorhandenes `image rebake` in den Vertrag aufnehmen

- Das Verb ist registriert und implementiert, kein Stub:
  erzwungenes Bake + Seal unter demselben Tag.
- `help.rs::print_image` und `cli-contract-v1.md` führen es nicht auf.
  `docs/images/bake-contract-v1.md` verweist schon darauf.
- Reproduziert: `image rebake --audit-unknown --format json` → Exit2 und
  `command=image.bake`. Auch Bake-/Seal-Laufzeitfehler übernehmen die Namen
  ihrer Hilfsoperation statt `image.rebake`.
- Abschluss: Namespace-Hilfe + Spec ergänzen; kanonischen Command
  `image.rebake` in allen Antworten erhalten. Parser-/frühen Backend-Fehler
  als CLI-Prozess prüfen; vorhandene Bake-/Seal-Backendtests beibehalten.
  Ein vollständiger Rebake-Erfolgs-Prozesstest bleibt Teil der breiteren
  Image-Builder-Abnahme, kein Blocker für die Fehlerausgabe-Korrektur.

### R4 — Stream-/Help-Abweichungen korrigieren

- `stack watch` ist dokumentiertes NDJSON und akzeptiert **kein** `--format`.
  Die Schlusszeile der Stack-Hilfe behauptet jedoch, jedes Subcommand
  akzeptiere `--format human|json`. Auf die tatsächlichen Commands begrenzen.
- `controller watch --format json` ist NDJSON, nicht ein einziges Dokument;
  R2 und zentrale Spec müssen das sagen.
- `events subscribe`, `stack watch`, menschliches `vm logs --follow`,
  `vm attach`, `vm exec -it`, Docker-Passthrough, `validate --schema`,
  Help und Skill bleiben explizite Ausnahmen. Keine Umwandlung ihrer
  Protokolle in Ein-Dokument-JSON im Rahmen von #17.
- `controller reconcile --registered <generation>` ist ein interner,
  Supervisor-gesteuerter Aufruf; nicht als weiteres öffentliches Feature
  bewerben oder in den öffentlichen Scope aufnehmen.

### R5 — Drei Hinweis-Commands ehrlich kennzeichnen

| Command | Tatsächliches Verhalten | Release-Scope |
|---|---|---|
| `certs rollout` | Sofort fail/Exit2; verweist auf Apply/CA-Rollout | Eigenständiges Verb **nicht unterstützt**. Funktion ist bereits Teil von Apply |
| `certs verify` | Sofort fail/Exit2; verweist auf Guest-Exec/curl | Eigenständiges Verb **nicht unterstützt** |
| `oidc token` | Sofort fail/Exit2; verweist auf Browser/Auth Code + PKCE | Eigenständiges Token-Tool **nicht unterstützt** |

Alle drei liefern mit `--format json` bereits ein vollständiges Fehler-Envelope
mit ihrem eigenen Commandnamen. Die zentrale CLI-Spec benennt die Grenze.
`help.rs`, `docs/specs/certs-v1.md` und `docs/specs/oidc-v1.md` listen sie aber
noch wie normale Funktionen. Dort als Hinweis/nicht implementiert markieren
oder aus der Funktionsliste entfernen, bestehende Fehlerantworten kompatibel
behalten. Kleine Prozessregression für fail/Exit2 hinzufügen. Ihre Implementierung
ist ausdrücklich **keine Voraussetzung** für #17.

### R6 — Gezielte Abnahme statt aller denkbaren Live-Tests

Für #17 fehlen folgende konkrete Vertragsregressionen:

1. Parametrisierte Prozessprüfung für die 21 R1-Pfade mit unbekanntem Flag
   vor **und** nach `--format json`; exakt ein stdout-Dokument,
   Pflichtfelder, erlaubter Status, Prozess-/Envelope-Exit identisch.
2. Stack-Validierungs-/Dateifehler und Route-RPC-/Config-Fehler als sichere
   Temp-/Mock-Fälle; diese teilen die defekten Emitter mit Parsefehlern.
3. `port list` mit Fake-Supervisor: Erfolg und Socket-/RPC-Fehler.
4. Rebake-Commandname, die drei Stubs und neue Help-Topics wie R3/R5.
5. Controller-Status Pending/Ready/Warnung sowie zwei Watch-Zyklen und
   sauberes Stoppen; NDJSON-Record vs. Prozessende gemäß R2. Kubernetes
   warn/24 ist schon vorhanden und soll unverändert grün bleiben.

Breitere Live-Matrizen für Docker, privilegiertes DNS/CA, Image-Builder,
Guest-PTY/Systemd und DB-/NATS-Server sind eigene Abnahmeaufgaben. Sie sind
keine offene Feature-Sammelaufgabe unter #17 und dürfen nicht als bereits
durch die Unit-Suite verifiziert bezeichnet werden.

## Empfohlener Release-Abschluss

#17 kann geschlossen werden: R1–R6 sind umgesetzt, die Gesamt-Testabnahme ist grün,
vorhandener CLI-Umfang konsistent, Statusausnahmen
präzise, Stubs ausdrücklich ausgeschlossen, gezielte Regressionen grün.
Die pauschale Anforderung „alle irgendwann geplanten CLI-Verben implementieren“
gehört nicht in dieses Contract-Issue. Danach Builds/Pakete erneut erzeugen,
weil ausführbarer Code geändert wurde. Der frühere lokale Prüfbau `0.0.1`
enthält diese neuen Contract-Korrekturen noch nicht.
