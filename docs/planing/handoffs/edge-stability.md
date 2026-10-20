# Übergabe Edge/Stability – 20.10.2026

Stand: 17:10 UTC. Auf Nutzeranweisung beendet; keine weitere Livephase begonnen.

## Eigene Prozesse und tatsächliche Event-Ergebnisse

Die lesende Subscription `vzctl events subscribe --filter 'controller.*,apply.*'`
lief vom **17:07:09 bis 17:09:56 UTC**. Der eigene Collector PID 15368 wurde
kontrolliert per SIGTERM beendet; er beendete seine Subscription PID 15370 mit
Exitcode 0. Abschließende Prozessprüfung: beide PIDs nicht mehr vorhanden.
Keine Produktdienste oder VMs wurden dabei verändert.

Private Evidenz: `/private/tmp/vzctl-f48-edge-events-20261020T170709Z/`.

- `result.json`: SHA256 `811c290647840f8b67f6f9c725365868f13126212386facf617d464bdec99155`
- `events.jsonl`: **0 Bytes / keine empfangenen Events**
- `diagnostics.log`: **0 Bytes**
- Abbruchgrund: `requested_stop`

Damit ist weder ein echtes `controller.status.changed` noch die Abwesenheit
von Timestamp-Spam bei nachweislich erfolgten periodischen Versuchen belegt.
Es wurden keine synthetischen Events erzeugt. Die geplante MCP-Nachabnahme nach
Late wurde **nicht begonnen**.

## Bereits abgeschlossener Late-/Fault-Review

Nur Quellenreview, keine eigene Ausführung der Phasen:

- Gemeinsame Baseline `/private/tmp/vzctl-controller-f45-baseline.py`,
  SHA256 `80154a31772c155d69b79bdeec189de998d420a5ed84859ea74a3abcf960b569`.
  Bindet den echten F48-Full-Beleg an den F48-Apply-Beleg; kanonische Docker-
  Mounts, sechs laufende und drei absichtlich gestoppte VMs bleiben geschützt.
  Das geprüfte Caddy-Delta erlaubt genau den realen optionalen Alias
  `<erstes Hostlabel>.localhost`; fremde Blöcke werden weiter exakt verglichen.
- Followup `/tmp/vzctl-controller-followup-f45-live.py`, zuletzt gemeldeter
  SHA256 `e8894a94fafddb235f61097e1849f7f3082f05929eb55643b8c99fbd34db2685`.
  Late benötigt einen tatsächlich erfolgreichen Repeat-Abschluss, erstellt
  genau vier Claims und wartet auf den periodischen Controller; kein weiterer
  Stack-Apply oder automatischer Resume. Bestehende Bindings und Ressourcen
  bleiben geschützt.
- Fault `/tmp/vzctl-controller-ownership-fault-f45-live.py`,
  SHA256 `2e1300cacaee3a804de731966200c02915b9dd5338b85cbebe40d63da6fec38c`.
  Reihenfolge: `secret-owner`, `minio`, `k3s`, **`foreign-db` zuletzt**.
  Jede Phase benötigt echte erfolgreiche Vorgängerbelege. Wiederherstellung
  ist zeitlich begrenzt vorbereitet und zusätzlich im Fehlerpfad vorgesehen.
  Secret-Restore prüft UID, Datenhash, Owner und ResourceVersion.
  Foreign-Claim-Cleanup löscht ausschließlich die belegte erzeugte UID;
  ohne UID-Beleg erfolgt keine Löschung allein anhand des Namens.
  Die fremde Datenbank wird bewusst erhalten.

Die konkret angefragte Aufrufkompatibilität ist gegeben:
Fault übergibt `initial['installed_sha256']` als Dictionary an
`base.installed_hashes`, nicht einen einzelnen CLI-Hash.

Kein verbleibender konkreter Live-Blocker aus diesem engen Quellenreview.
Das ist keine Aussage über eine erfolgte Live-Abnahme. Vor Ausführung müssen
der dann aktuelle Stand und die tatsächlichen Vorgängerbelege benutzt werden.

## Repeat: Erfolg des Apply getrennt vom Abschlussbeleg halten

Root meldete den tatsächlichen Repeat-Apply als erfolgreich. Die anschließende
Runner-Abnahme scheiterte zu früh an `lastAttemptAt: null` der neu registrierten
Controllergeneration (`cp_registry_not_ready`). Apply-Agent übernahm ausschließlich
die lesende Nachabnahme von Generation 10 und dem ersten Hintergrundlauf.

Zum Ende dieses Agents liegt hier **kein abschließender neuer Repeat-Beleg**
vor. Den fehlgeschlagenen historischen Beleg nicht umschreiben und keinen
erneuten Apply auslösen. Late erst anhand des tatsächlichen Abschlussbelegs
fortsetzen. Der vorherige Caddy-Parser-Abbruch war eine separate Vorprüfung
vor Mutation und darf nicht als allgemeine Retry-Freigabe verwendet werden.

## Belastbare letzte F48-Belege

- Apply: `~/vzctl-labs/controller-lab/evidence/f48-apply-20261020T164823Z/result.json`,
  SHA256 `ecc6813884475b7bf5fb9519f2534fcdee3330491bcc18d6c6d394dbf8f143a2`.
- Full: `~/vzctl-labs/controller-lab/evidence/full-live-f48-verified-20261020T165021.477100Z/result.json`,
  SHA256 `534fac9e13afb1e01858d867fb65e4e99c82af7ec10b6d656fc44d352f245311`.
- MCP: `~/vzctl-labs/controller-lab/evidence/mcp-live-20261020T165132.868386Z/result.json`,
  SHA256 `1303378af03124475813ae98bf1558611bfb9c0b416d6305404e037d063a00e8`.
- Eigene lesende Nachprüfung:
  `/private/tmp/vzctl-f48-edge-after-apply-result.json`,
  SHA256 `97ee6d83b590943bc4c0b0db8430c3fa16ee0963c454cc2fd013db4059b94e05`.
  Bestätigte damalige Installations-/Prozessbaseline sowie Neti-DNS und
  HTTPS aus allen drei Neti-Gästen: HTTP 200, TLS-Prüfung erfolgreich.

Installierte F48-CLI:
`f7cdececf68f6a2239cb0563d0f8287842ef0faa0612c1228fb3a395b7d384ab`.
Die drei IQ-Store-VMs sind absichtlich gestoppt; nicht für Prüfungen starten.

## Weitere bereits geprüfte Folgeanschlüsse

Retention `/tmp/vzctl-controller-retention-f45.py`,
SHA256 `1058d12514aaaa2d44fb41612bd24e56b71029fa0296a0cbf8f980f1cab52d41`,
und Lifecycle `/tmp/vzctl-controller-lifecycle-f45.py`,
SHA256 `6a99a5abdb788dba25dd04a348fb1d9035b8f0fb2831776a4153df4f6e500d85`:
enger F48-Schema-/Mount-Delta-Review abgeschlossen. Keine Live-Abnahme;
Late und alle vier Fault-Belege bleiben Voraussetzungen.

Für eine später ausdrücklich beauftragte MCP-Nachabnahme existiert bereits
`/private/tmp/vzctl-controller-mcp-smoke.py`. Historische Ausgabedateien nicht
überschreiben; eigenes privates Evidenzverzeichnis verwenden. Kein weiterer
Runnerrahmen ist erforderlich.
