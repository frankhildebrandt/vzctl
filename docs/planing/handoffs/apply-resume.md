# Übergabe Apply/Resume – 20.10.2026

## Letzter belegter Stand

Die bereits begonnene lesende Repeat-Nachabnahme ist mit Exit 0 beendet. Es wurde kein weiteres Apply oder Reconcile ausgeführt.

- Ergebnis: `/Users/frank/vzctl-labs/controller-lab/evidence/followup-repeat-f48-20261020T170352.422564Z/result.json`
- SHA256: `6740dace3e86e1d9f2897a9433e299b340bf35b1afe348259da6b1ed25234998`
- Journal: Generation 10, abgeschlossen; gegenüber Generation 9 gleicher Sollinhalt.
- Registry: `c09a43a1-4daa-47b4-b4e2-cd3bb45ed57a`; fünf Claims Ready, regulärer Hintergrundlauf erfolgreich.
- VM-Zustände und PIDs, Dienstprozesse, K3s-Startzeiten, CoreDNS-Identität, Anwendungsidentitäten, Bindings, Eingaben und bestehende Daten unverändert; Plan leer.
- Sechs VMs laufen, drei IQ-Store-VMs bleiben vom Nutzer gestoppt.
- Historische `failed.json` und `preflight.json` bleiben unverändert. Das ergänzende Ergebnis dokumentiert ausdrücklich `additional_apply_or_reconcile_performed=false`.

Der ursprüngliche Repeat hatte sein Apply bereits abgeschlossen. Seine sofortige Registry-Prüfung war zu früh: Eine Registrierung löscht `lastAttemptAt`; der reguläre nächste Controllerlauf startet nach dem konfigurierten Intervall. Dies ist kein belegter Produktfehler.

## Grenzen und verbleibende Arbeit

Die Nachabnahme wiederholte keine schreibenden Consumerprobes. Sie bindet stattdessen den bereits erfolgreichen F48-Full-Beleg:

- Full: `evidence/full-live-f48-verified-20261020T165021.477100Z/result.json`
- Full-SHA256: `534fac9e13afb1e01858d867fb65e4e99c82af7ec10b6d656fc44d352f245311`
- MCP: `evidence/mcp-live-20261020T165132.868386Z/result.json`
- MCP-SHA256: `1303378af03124475813ae98bf1558611bfb9c0b416d6305404e037d063a00e8`

Diese beiden Belege liegen unter `/Users/frank/vzctl-labs/controller-lab/`. Full bestätigt VM/Docker/K3s jeweils PostgreSQL, S3 und OIDC sowie HTTPS 200. MCP bestätigt öffentliche Bindings, fünf Ready-Claims, zwei Ready-Nodes und das redigierte Diagnosebundle.

Late-Claims, Ownership/Fault, Retention und Lifecycle sind weiterhin getrennte, noch nicht live abgeschlossene Abnahmeschritte. IQ-Store-Trust bleibt bis zum regulären späteren Stack-Apply offen; gestoppte VMs nicht eigenständig starten.

## Vorhandene Werkzeuge

Keine neuen Runner anlegen. Die vorhandenen Dateien verwenden trotz ihrer technischen F45-Dateinamen den F48-Vertrag:

- `/private/tmp/vzctl-controller-f45-baseline.py` — SHA256 `80154a31772c155d69b79bdeec189de998d420a5ed84859ea74a3abcf960b569`
- `/private/tmp/vzctl-controller-followup-f45-live.py` — SHA256 `e8894a94fafddb235f61097e1849f7f3082f05929eb55643b8c99fbd34db2685`
- `/private/tmp/vzctl-controller-ownership-fault-f45-live.py` — SHA256 `2e1300cacaee3a804de731966200c02915b9dd5338b85cbebe40d63da6fec38c`

Der Adapter berücksichtigt die realen generierten Neti-Caddy-Aliase und schützt fremde Blöcke vollständig. Docker-Mountidentität verwendet die kanonische Funktion des geprüften F48-Consumers; bloße Arrayreihenfolge ist kein Unterschied.

## Nächste direkte Aktion

Parent übernimmt den erfolgreichen Repeat-Beleg in Dashboard und passende Issue-Abnahmen. Bei fortgesetztem Auftrag folgt der vorhandene Modus `late`, gebunden an dieses erfolgreiche Repeat-Ergebnis und die unveränderten Full-/MCP-Belege. Kein weiteres Repeat-Apply zur Reparatur der bereits abgeschlossenen Generation 10.

Vor künftigen Apply-Nachprüfungen darf der vorhandene Runner begrenzt auf den ersten regulären Registrylauf warten; kein zusätzlicher Recovery-Wrapper ist erforderlich. Diese Anpassung wurde hier nicht mehr begonnen.

## Abschluss des bisherigen Agents

Keine eigenen Prüfprozesse mehr aktiv. Keine neuen Produktänderungen, Builds, Liveaktionen oder Arbeitspakete begonnen. Ergebnis und Übergabe sind an Parent gemeldet.
