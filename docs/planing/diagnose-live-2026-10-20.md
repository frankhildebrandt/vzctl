# Abschlussprobe #48 — 20.10.2026

**Ergebnis: bestanden.** Diagnoseexport und RAM-Budget mit der tatsächlich
installierten CLI gegen den bestehenden IQ-Store-Stack geprüft.

CLI: `/Users/frank/.local/bin/vzctl`
SHA256: `bfd0f9347504e15b4cff67f4eb943bae6601b686b56089d9d722feeef4eaf8ce`

Dies ist der installierte F26-Stand; die spätere F27-Korrektur ist für diese
Diagnose-/RAM-Funktionen nicht erforderlich.

## Ausgeführte Proben

| Aufruf | Ergebnis |
|---|---|
| `validate -C /Users/frank/Projects/monitos-iq-store-server --format json` | Exit 0, `status=ok`, stderr leer |
| `plan -C /Users/frank/Projects/monitos-iq-store-server --format json` | Exit 0, `status=ok`, stderr leer; `summary.resource_budget` vorhanden und geprüft |
| `diagnose --output <neuer privater Temp-Pfad>/diagnose.json --format json` | Exit 0, `status=ok`, stderr leer, keine Sammelwarnung |

Der Plan wurde nicht angewendet. Keine VM-, Dienst-, Netzwerk-, YAML- oder
Lab-Änderung; geschrieben wurden nur neue temporäre Nachweise und dieses Dokument.

## RAM-Budget

| Feld | MiB |
|---|---:|
| Hostkapazität | 65.536 |
| Vom Stack angefordert | 12.288 |
| Hostreserve | 13.108 |
| Rechnerisch nach Reserve für VMs | 52.428 |

Die Reserve entspricht `max(2048, ceil(Hostkapazität / 5))`. Alle Zahlenfelder
und die Differenzberechnung stimmen; `warnings` ist eine leere Liste.
Dies ist ein Kapazitätsvergleich, keine Messung aktuell freien Speichers.
Andere Stacks und laufende Anwendungen werden ausdrücklich nicht mitgezählt.
Die Überlast-Warnfälle sind zusätzlich durch die bestehenden Unit-Tests abgedeckt.

## Diagnoseexport

- Neue Datei, Modus `0600`, in neuem temporärem Verzeichnis mit Modus `0700`.
- Größe **6.810 Bytes**, Grenze **65.536 Bytes** eingehalten.
- Gültiges JSON mit `apiVersion=vzctl.dev/diagnose/v1`.
- Supervisor-, VM-Inventar- und Journal-Zusammenfassung erfolgreich gesammelt;
  keine Sammelwarnung.
- `collection.raw_logs`, `config_contents`, `environment` und `credentials`
  jeweils `false`. Die bestehende Allowlist-/Redaktionsimplementierung und
  der private, begrenzte Export sind zusätzlich durch Unit-/CLI-Prozesstests
  abgesichert.
- Keine Rohinhalte, Konfigurationen oder Geheimnisse in Toolausgabe oder
  Repository übernommen.

Private lokale Nachweise: `/tmp/vzctl-48-live-oaslbv9c/`. Die bereinigten
Prüffakten liegen dort in `verification.json`; der Diagnoseexport selbst
bleibt ausschließlich temporär und privat. Dieser Bericht enthält die
dauerhaften Abnahmefakten.

Die zuvor offene Abschlussprobe von #48 ist damit erfüllt. Issue-Änderungen
oder Veröffentlichungen wurden in diesem Prüfschritt nicht vorgenommen.
