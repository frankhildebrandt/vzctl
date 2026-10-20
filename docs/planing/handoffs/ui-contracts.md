# Übergabe UI-Contracts · 20.10.2026

Dieser Agent beendet seine Arbeit. Zuletzt wurden ausschließlich der aktuelle
Paketprüfbericht vervollständigt und diese Übergabe geschrieben. Keine neuen
Builds, Tests oder Runtimeaktionen nach der Pause. Der neue Agent muss den
aktuellen Stand bei Root übernehmen; diese Übergabe autorisiert keinen Live-Lauf.

## Abgeschlossen

- F46 UI nativ: vollständig gestoppter IQ-Store neutral, abhängige Checks übersprungen; laufende Neti-/Lab-Dienste weiterhin bewertet. Native Nachweise bei Root.
- F48 Full und MCP tatsächlich erfolgreich. Belege in [09-live-update-acceptance.md](../09-live-update-acceptance.md) und [10-next-slice.md](../10-next-slice.md). Full-SHA `534fac9e13afb1e01858d867fb65e4e99c82af7ec10b6d656fc44d352f245311`, Apply-SHA `ecc6813884475b7bf5fb9519f2534fcdee3330491bcc18d6c6d394dbf8f143a2`.
- F48-Consumer `/tmp/vzctl-controller-full-f48-verified.py`, SHA `c60e813d6981ae53e77b69d04a10b45e71ff4743bf76e4dfdd2dfe2ad86e5d7b`, unabhängig freigegeben. 88 neue +133 Referenzchecks. Normales ClusterFirst-DNS und strikte Discovery vor sicheren VM/Docker/K3s-Probes. Aktive CoreDNS-Pods werden ohne Terminating/terminal gezählt, alle müssen Ready sein.
- Lokale F48-Dev-Pakete fertig/geprüft; Details und abweichender Paket-/Live-CLIhash in [package-verification-2026-10-20.md](../package-verification-2026-10-20.md). Kein offener eigener Prozess oder DMG-Mount. Sauberer separater macOS-26-Mac bleibt extern offen.

## Vorbereitete Retention-/Lifecycle-Dateien

Technische Dateinamen bleiben bewusst `f45`; Inhalt/Receipts sind F48. Keine neue
Variantenkette anlegen und keine historischen Receipts/Helper überschreiben.

| Datei | Eingefrorener SHA256 |
|---|---|
| `/tmp/vzctl-controller-retention-f45.py` | `1058d12514aaaa2d44fb41612bd24e56b71029fa0296a0cbf8f980f1cab52d41` |
| `/tmp/vzctl-controller-lifecycle-f45.py` | `6a99a5abdb788dba25dd04a348fb1d9035b8f0fb2831776a4153df4f6e500d85` |

Edge hat diesen Quellenstand ohne Blocker freigegeben. Vorhandene Offlinebelege:
`/tmp/vzctl-controller-retention-f48-offline.log` (102 Checks +4 eingebettete
Pythonprogramme) und `/tmp/vzctl-controller-lifecycle-f48-offline.log` (138 Checks).
Beide Runner waren durch diesen Agenten **nicht live ausgeführt**.

Gemeinsame Baseline gehört Apply-Agent:
`/tmp/vzctl-controller-f45-baseline.py`, zuletzt mitgeteilter freigegebener SHA
`80154a31772c155d69b79bdeec189de998d420a5ed84859ea74a3abcf960b569`.
Dieser letzte kleine Fix akzeptiert exakt erzeugte Neti-Aliasheader; keine weitere
Änderung an meinen Runnern nötig, nur expliziter Adapterhash beim Aufruf.
Neueren Stand/Repeat-Ausgang unbedingt bei Root/Apply übernehmen. Keine alten
Prozesspins oder historischen 9-running-Annahmen verwenden: Basis ist sechs
laufende VMs plus drei bewusst gestoppte IQ-Store-VMs.

## Fortsetzung und Schutzverträge

1. Tatsächlicher Repeat-Erfolg → Late → vier Faultfälle; `foreign-db` zwingend zuletzt mit Hashbindung an die drei Vorgänger. Anschließend Retention, dann Lifecycle. Offlinefreigabe ersetzt keinen dieser Live-Belege.
2. Retention entfernt exakt fünf YAML-Claims, erhält vier Late-Claims und den belegten fremden Retained-Datensatz. Vor Aktivierung Kandidat validieren und exakten Plan prüfen; höchstens eigenes `ops/proof`-Binding-Recreate. Kein VM-/Hostrestart/Purge.
3. Restore ist vorbereitet, aber bei Fehler kein blindes automatisches Apply. Private Originalkopie, Fehlerbeleg und Identitäts-/Inputguards erhalten. Nach Restore neun Ready plus bekannter foreign Retained; vorhandene Daten/Bindings/Credentials vor schreibenden Probes lesen.
4. Normale/restored Mount-Identität nutzt `adapter.contract.canonical_mount_hash`: alle Felder erhalten, nur nach Destination sortiert, keine doppelten Ziele, ID/User und vier readonly Bind-Mounts geprüft. Removed-Phase erlaubt exakt einen readonly `/opt/lab`-Mount und hasht das vollständige Objekt. Lifecycle nutzt dieselbe `ret.runtime_identity`.
5. Caddy darf ausschließlich über geprüfte eigene Routendifferenz und belegte PID-/Configtransition wechseln; andere VMs/Net/Edge/CP bleiben geschützt. Vorhandene Adapter-API unverändert nutzen.
6. Lifecycle trennt CP-Neustart, graceful Lab-down und reine Beobachtung eines konkret extern gestarteten neuen UI-Apply-Jobs. Observer löst kein Apply aus. Sechs Bestands-VMs bleiben geschützt; zulässige Lab-Reboot-/Podänderungen bleiben phasenbezogen. Kein Purge. Vor langem UI-Apply Root-Abstimmung.
7. Sichere Probehelfer erhalten begrenzte, allowlistete Teilergebnisse bei Exit1/HTTP503. Keine Secrets/Kubeconfig/Rohpayloads ausgeben. Auf Schema-/Hashfehlern stoppen; keine Gates pauschal lockern.

Root besitzt Dashboard/GitHub und alle Livefreigaben. Aktuellen Gesamtstatus aus
den vorderen Abschnitten von 09/10 übernehmen; ältere chronologische Abschnitte
darin können absichtlich historische Fehlerstände beschreiben.
