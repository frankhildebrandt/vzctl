# Runtime-Abschlussübergabe · 20.10.2026

Maßgeblich ist [12-release-completion-goal.md](../12-release-completion-goal.md).
Diese Übergabe entstand ausschließlich durch Datei-/Quellenlesen und Hashprüfung.
Die Live-Pause besteht; keine Prüfung wurde neu ausgeführt, keine Runtime verändert.

## Belegt fertig

- F48-Full: VM, Docker und K3s jeweils PostgreSQL, S3, OIDC und HTTPS erfolgreich;
  normaler Pod-DNS-Weg und striktes TLS. MCP-Status/Diagnose erfolgreich.
  Die gebundenen Full-/MCP-Hashes stehen im Repeat-Beleg und in
  [apply-resume.md](apply-resume.md).
- Repeat-Ergebnis direkt gelesen und SHA256 geprüft:
  `/Users/frank/vzctl-labs/controller-lab/evidence/followup-repeat-f48-20261020T170352.422564Z/result.json`,
  `6740dace3e86e1d9f2897a9433e299b340bf35b1afe348259da6b1ed25234998`.
  `state=passed`, Journal Generation 10 abgeschlossen, gleicher Sollinhalt wie 9,
  fünf Ready-Claims und regulärer Controllerlauf. VM-/Dienstidentitäten,
  CoreDNS, Bindings und bestehende PostgreSQL-/S3-Daten erhalten;
  `repeat_without_recreation=true`, kein zusätzliches Apply/Reconcile.
  Consumerprobes wurden dabei bewusst aus dem erfolgreichen Full-Beleg übernommen.
- Native Prüfung gestoppter IQ-Store laut [ui-contracts.md](ui-contracts.md)
  abgeschlossen. Sechs VMs liefen beim Beleg; drei IQ-Store-VMs blieben gestoppt.
  Der frühere zu frühe Registrycheck bleibt als Skriptfehler dokumentiert;
  daraus folgt kein offener Produktfehler und kein weiteres Repeat-Apply.

## Verbleibende Abnahmen – genau diese Reihenfolge

Alle folgenden Aktionen gelten erst nach Aufhebung der Live-Pause durch Root.
Runtime-Mutationen bleiben bei einer Zuständigkeit und laufen nacheinander.

| Schritt | Fertig-Kriterium | Nächste direkte Aktion |
|---|---|---|
| Late | Exakt vier zuvor fehlende Kubernetes-Claims innerhalb 120 s durch den periodischen Controller Ready, ohne Apply/manuelles Reconcile; bestehende Bindings/Identitäten erhalten, Late-HTTPS erfolgreich. Offenen Event-/MCP-Livebeleg ergänzen. | Vorhandenen Followup-Runner mit `--phase late` an geprüften Repeat sowie Full/MCP binden; Ergebnis und Events/MCP sichern. |
| Fault 1: `secret-owner` | `OwnershipConflict`, fremd markiertes Secret samt UID/Daten unverändert, unabhängige Claims Ready; ursprünglicher Owner wiederhergestellt und Probes erfolgreich. | Vorhandenen Fault-Runner mit `--case secret-owner` an Late/Full/MCP binden. |
| Fault 2: `minio` | Ausfall als `ProviderUnavailable`/`ProviderTimeout` sichtbar, unabhängige DB/OIDC-Claims Ready; Dienst repariert, Claims/Probes wieder erfolgreich. | Nach Erfolg von Fault 1 vorhandenen Modus `--case minio` ausführen. |
| Fault 3: `k3s` | `KubernetesUnavailable` und Exit 24 sichtbar, Routen während Discovery-Ausfall erhalten; K3s repariert, Claims/Probes wieder erfolgreich. | Nach Erfolg von Fault 2 vorhandenen Modus `--case k3s` ausführen. |
| Fault 4: `foreign-db` | Fremde PostgreSQL-Datenbank bleibt Owner `postgres`, Claim meldet `OwnershipConflict`, kein Binding-Secret angelegt; Test-CR entfernt, Datenbank/Retained-Datensatz bleiben erhalten. | Zwingend zuletzt ausführen; alle drei erfolgreichen Fault-Vorgänger mit Pfad und SHA256 binden. |
| Retention | Exakt fünf YAML-Claims entfernt und wiederhergestellt; vier Late-Claims und fremder Retained-Datensatz erhalten. Nach Restore neun Ready plus bekanntes foreign Retained; bestehende Daten, Bindings/Credentials erhalten, Probes erfolgreich. | Vorhandenen Retention-Runner an vollständige Belegkette binden; Kandidat validieren und exakten Remove-/Restore-Plan prüfen. Nur eigenes `ops/proof`-Binding-Recreate zulässig. |
| Lifecycle | CP-Neustart mit wiederaufgenommener Registry und Persistenz belegt; danach graceful Lab-down ohne Purge mit deaktiviertem Controller und ruhendem Zustand. Geschützte Bestands-VMs bleiben erhalten. | Nach Retention vorhandenen Lifecycle-Runner zuerst `--phase cp-restart`, danach `--phase down` verwenden, jeweils erfolgreichen Vorgänger binden. |
| Langes UI-Apply | Ein konkreter neuer UI-Job samt eindeutigem Journal erfolgreich; Registry wieder aktiv, neun Ready plus foreign Retained und Daten/Identitäten erhalten. Native UI bleibt bedienbar und zeigt Fortschritt/Fehler/Events korrekt; erforderliche Consumer-Nachprobes erfolgreich. | Nach Down mit Root den tatsächlichen UI-Start abstimmen; vorhandenen `--phase observe-ui-apply --job-id …` nur beobachtend anbinden. Native Beobachtung und anschließend gesonderte Consumerprobes ergänzen; der Observer führt beides nicht aus. |

## Vorhandene Werkzeuge und Grenzen

Die fünf Runner/Adapter unter `/private/tmp/` sind vorhanden; ihre SHA256 stimmen
mit [apply-resume.md](apply-resume.md) und [ui-contracts.md](ui-contracts.md) überein:
`vzctl-controller-f45-baseline.py`, `vzctl-controller-followup-f45-live.py`,
`vzctl-controller-ownership-fault-f45-live.py`, `vzctl-controller-retention-f45.py`,
`vzctl-controller-lifecycle-f45.py`. Technische F45-Namen enthalten den F48-Vertrag.
Keine neuen Runner, Wrapper oder Frameworks nötig; erfolgreiche Checks nicht wiederholen.

Bei Fehler anhalten, Beleg und private Restorekopie erhalten, Journal/Identitäten
prüfen; kein blindes Retry/Restore-Apply. Keine gestoppte IQ-Store-VM starten.
IQ-Store-Trust bleibt bis zum regulären späteren Stack-Apply offen;
`global_trust_complete=false` darf nicht als vollständig abgenommen gelten.

Nach jedem erfolgreichen Schritt übernimmt Root Beleg, GitHub-Kriterien und Canvas.
Dieser Runtime-Stand allein ist kein Gesamtabschluss: Paket-/Issue-Abgleich,
externer sauberer macOS-26-Installationsnachweis und Auslieferungsentscheidungen
bleiben gemäß Abschlussplan getrennt offen.

## Fortsetzung: Late abgeschlossen · 20.10.2026, 20:11 MESZ

Root hat nach Nutzerauftrag „fortsetzen“ die Live-Pause aufgehoben. Genau ein
vorhandener Late-Lauf hat vier fehlende CRs angelegt; kein Repeat-Apply,
zusätzliches Claim-Create oder manuelles Reconcile. Alle neun Claims Ready;
konservative Bereitschaftsobergrenze **8,663142 s**. Journal Generation 10 und
Registry unverändert. Bestehende Bindings, Daten, Seeds, CoreDNS, VM-/Dienst-
und Anwendungsidentitäten erhalten; drei IQ-Store-VMs weiter gestoppt.

- Ergebnis: `/Users/frank/vzctl-labs/controller-lab/evidence/followup-late-f48-20261020T180633.949929Z/result.json`
- Ergebnis-SHA256: `80a3838ad504490e3b9b676379a02dc3c7f30d27a8c358cbedc2120308302f0b`
- Echte `controller.status.changed` mit neun Ready-Claims:
  `evidence/late-events-f48-20261020.ndjson`, SHA256
  `288b13bf28c3a72b689f0e86f1ed75ed2719c37e06e0bf110642d0dc6a8cbbe8`.
  Capture 0600, Subscription sauber beendet, stderr leer.
- Late-HTTPS 200; vorhandene PostgreSQL-/S3-/OIDC-Consumerprobe erfolgreich.

Der erste Runner stoppte nach Ready an `unexpected_late_caddy_delta`:
seine Annahme eines gleichen NodePorts für zwei Ingress-Claims war falsch.
Nur die zwei Late-HTTP-/HTTPS-Blöcke kamen hinzu; bestehende Blöcke bytegleich.
Der eigene verwaltete Late-Service verwendet `10.123.0.11:31605`, die bestehende
Kube-Route `10.123.0.11:31040`. Service-Owner/UID, Late-CR-UID, TCP8080,
Selector und Ready-Endpoint auf dieselbe proof-Pod-UID wurden unabhängig geprüft.

Auf Rootfreigabe wurde ausschließlich der bestehende Adapter eng korrigiert.
**Neuer maßgeblicher Adapter-SHA256:**
`6689bc49e926aaa2729467712975e591d008092259a86d3260cabc30caa12e05`.
Der alte Hash `80154a…` bleibt historisch im ursprünglichen Preflight erhalten.
Fünf negative Offlinefälle verwerfen falschen NodePort, fremde Zusatzroute,
Änderung eines Bestandsblocks, fremden Service-Owner und falsche Service-UID.
Keine Port- oder Routendifferenz wird pauschal freigegeben.

Die ergänzende Nachabnahme wiederholte keine erfolgreiche Mutation;
nur die zuvor fehlende ursprüngliche Late-HTTPS-Consumerprobe wurde ausgeführt.
`failed.json` (SHA256 `40951efeacdebaccfe81943904b68325809ab34a39e7653439e3b60ddccf392a`)
und `preflight.json` bleiben unverändert. Ihr historisches Phasenlabel
`preflight` beschreibt den tatsächlichen Fehlerort nach Ready nicht korrekt;
Quellenpfad, CR-Zeitstempel und Event belegen den Ablauf.

Nächster direkter Schritt: vorhandenen Fault-Runner, unverändert SHA256
`2e1300cacaee3a804de731966200c02915b9dd5338b85cbebe40d63da6fec38c`,
genau einmal mit `--case secret-owner` an Full `534fac…`, diesen Late-Beleg,
Followup `e8894a…` und **neuen Adapterhash** binden. Danach Ergebnis melden;
anschließend MinIO → K3s → foreign-db zuletzt, dann Retention → Lifecycle/UI-Apply.
Kein Faultfall wurde in diesem Paket begonnen. Keine Prüfprozesse mehr aktiv.

## Fortsetzung: vier Faultfälle abgeschlossen · 20.10.2026, 20:20 MESZ

Root hat die vier vorhandenen Fälle nacheinander freigegeben.
`secret-owner` → `minio` → `k3s` → `foreign-db` wurden jeweils genau einmal
ausgeführt; vor jeder Mutation Restore-Timer, anschließend Finally-Restore.
Kein Apply/Resume, kein VM-Neustart und keine Produktänderung.

Alle Belege unter `/Users/frank/vzctl-labs/controller-lab/evidence/`:

| Fall / Ergebnisdatei | SHA256 | Belegter Ausgang |
|---|---|---|
| `fault-secret-owner-20261020T181333.397822Z/result.json` | `a4d23a1933ed900680414b9518c23cac7ad0f9e6e7a5899d4ae5d099ee77cc2c` | OwnershipConflict/Exit24, fremd markiertes Secret samt UID/Daten geschützt, ursprünglicher Owner wiederhergestellt. |
| `fault-minio-20261020T181516.967081Z/result.json` | `aa5b0f54d8199b4e59fd22b564caad9d90f31c10544799a9594031bb0c7a1086` | ProviderUnavailable/Exit24, unabhängige DB/OIDC Ready; Dienst repariert. |
| `fault-k3s-20261020T181553.662987Z/result.json` | `cfc1caa51dfaf92e57c466984c4ee132b54d8bc822424b7896e502410e3555ac` | KubernetesUnavailable/Exit24, Routen erhalten; Dienst repariert, nur dessen erlaubter Startzeitpunkt geändert. |
| `fault-foreign-db-20261020T181636.383891Z/result.json` | `fd3bcb09eb95fdbc208522149c81d848ba53135d56e71bfc30e4bf3bc14b2430` | OwnershipConflict, kein Binding, CR entfernt, Datenbank mit Owner postgres erhalten; bekannter Retained-Datensatz. |

Die ersten drei Fälle haben jeweils alle VM/Docker/K3s-Consumerprobes und
HTTPS200 nach Recovery erfolgreich abgeschlossen. Nach Foreign wurden diese
K3s-Recoveryprobes anhand ihres SHA übernommen; Daten, Bindings und vollständige
Runtime-/Anwendungsidentitäten wurden erneut **nur lesend** verglichen.
Neun Ready plus foreign Retained, Journal Generation 10 und Registry erhalten;
sechs VMs laufen, drei IQ-Store-VMs weiterhin gestoppt. Keine Prozesse mehr aktiv.

Foreign benötigte diese Nachabnahme wegen eines Fehlers ausschließlich im
Prüfskript: Sein Foreign-Zweig übergibt den Rückgabewert von `controller reconcile`
an die Statusprojektion, obwohl nur `controller status` die Registry-Felder
`enabled`/`generation`/`lastAttemptAt` enthält. Historisches `failed.json` bleibt
mit SHA256 `86cf4f7885917e4faa5a682596eb584f757943c850c705131d05a411b3c06bb9`
erhalten. Unabhängig gespeicherter Conflict-Datensatz und private Guest-UID-
Quittung belegen den eigentlichen Negativfall. CR und Secret fehlen, Timer
inaktiv, DB-Owner postgres; Registry frisch und enabled. Keine Wiederholung,
neues Reconcile oder Consumerprobe. Der unveränderte Fault-Runner ist erledigt;
für die weiteren Phasen ist keine neue Variante nötig.

Root hat zusätzlich den neuen Zustand über echte MCP-Stdiotools lesend geprüft:
`mcp-late-20261020T181307.392723Z/result.json`, SHA256
`77c7426ce1c959b99efc93000e5ade1c93dd0de526f3d7f2155f3360bc642723`.
`controller_status`/`kubernetes_status`/`diagnose` erfolgreich, neun Ready-Claims,
zwei Ready-Nodes und keine Credentials in der Diagnose. Damit ist die offene
Late-Event-/MCP-Ergänzung ebenfalls belegt.

**Nächste direkte Aktion nach Root-Abgleich:** genau einmal vorhandenen
`/private/tmp/vzctl-controller-retention-f45.py` (SHA256
`1058d12514aaaa2d44fb41612bd24e56b71029fa0296a0cbf8f980f1cab52d41`)
an Full/Late und diese vier Faultbelege binden; Adapter-Pin
`6689bc49e926aaa2729467712975e591d008092259a86d3260cabc30caa12e05` verwenden.
Fertig bei erfolgreichem exaktem Remove/Restore der fünf YAML-Claims,
vier erhaltenen Late-Claims plus foreign Retained, unveränderten bestehenden
Daten/Bindings/Credentials und erfolgreichen Probes. Kein automatisches Restore
bei Fehler, kein Purge oder Host-/VM-Neustart. Retention und Lifecycle/UI-Apply
bleiben offen; dieses Paket endet vor Retention.

## Fortsetzung: Retention abgeschlossen · 20.10.2026

Nach Root-Freigabe genau ein vorhandener Retention-Lauf, SHA `1058d125…`,
mit neuem Adapter `6689bc…` und allen echten Full-/Late-/Fault-Belegen.
Keine Änderung an Runner oder Produkt. Beide privaten Kandidaten vor Aktivierung
validiert; vor jedem Apply exakt fünf Claim-Aktionen plus ausschließlich eigenes
`ops/proof`-Binding-Recreate geprüft. Genau zwei Applys, keine Wiederholung.

Entfernte Phase: vier Late-Claims Ready, fünf YAML-Claims plus foreign Retained;
Docker-/Kube-HTTPS entzogen, Late-Host und Late-OIDC-Client erhalten.
Daten, Bindings und Credentials lesend unverändert geprüft.
Nach Restore neun Ready plus foreign Retained; Original-YAML bytegleich,
SHA256 `4746f3a0301db79b80200f445fdb240fea82f35aea71a453dd1fe5f1c587a8fd`.
Originaler Caddy-Konfigurationshash zurück; Journal Generation 12 abgeschlossen.
Alle VM/Docker/K3s-Consumerprobes erfolgreich, beide HTTPS-Proben 200.
Vier Late-CRs, bestehende Daten, Bindings/Credentials und geschützte Runtime-
Identitäten erhalten. Sechs VMs laufen, drei IQ-Store-VMs bleiben gestoppt.

- Ergebnis: `/Users/frank/vzctl-labs/controller-lab/evidence/retention-live-f48-20261020T182200.125655Z/result.json`
- SHA256: `c7ffa96123639338c13fc27513f0ac3c8ea9ec526edb0eae943f3c3f0db18114`
- Ergänzend rein lesend: `post-restore-readonly.json` im selben Verzeichnis,
  SHA256 `67705b86fab0e7df9f9ee27a0ee842e0cdeb34d2fd6f307433409f6f811222bb`.
  Originalbytes exakt, Seeds unverändert, fremde Datenbank weiterhin Owner postgres.

Keine Host-/VM-/Dienstneustarts, kein Purge, kein Fehler und kein aktiver Prozess.
Dieser sichere Restore ist der Anschluss für Lifecycle; Retention ist fertig.

Nächster direkter Schritt nach Root-Abgleich: vorhandenen
`/private/tmp/vzctl-controller-lifecycle-f45.py` (SHA256
`6a99a5abdb788dba25dd04a348fb1d9035b8f0fb2831776a4153df4f6e500d85`)
zunächst ausschließlich `--phase cp-restart` mit Retention-Runnerpin
`1058d12514aaaa2d44fb41612bd24e56b71029fa0296a0cbf8f980f1cab52d41`,
Adapter `6689bc49e926aaa2729467712975e591d008092259a86d3260cabc30caa12e05`
und vollständiger Belegkette ausführen. Fertig bei gesundem CP, frischem
regulärem Controllerlauf und unveränderten Daten/Bindings/Workloads.
Danach `down` und UI-Apply getrennt koordinieren; der UI-Observer startet kein
Apply und belegt keine native Bedienbarkeit. Lifecycle noch nicht begonnen.

## Fortsetzung: Lifecycle bis Down abgeschlossen · 20.10.2026

CP-Neustart genau einmal regulär durchgeführt und bestanden:
`/Users/frank/vzctl-labs/controller-lab/evidence/lifecycle-f48-cp-restart-20261020T182747.606692Z/result.json`,
SHA256 `3eae1d8a0d6d137ac28c6e9c60d029b5b04f400ecf300e6563b07572ed33148e`.
Persistierte Registry übernommen, frischer erfolgreicher Controllerlauf;
sämtliche geschützten VM-/Net-/Edge-/Caddy-/Anwendungsidentitäten sowie Daten
und Bindings erhalten. Alle Consumerprobes erfolgreich, beide HTTPS-Proben 200.

Danach genau ein graceful Lab-down ohne Purge. Drei Lab-VMs gestoppt,
IQ-Store weiterhin dreimal gestoppt, Neti mit drei unveränderten VM-PIDs aktiv.
Registry disabled/null, alte Lab-Helper beendet, Net/Edge/Supervisor unverändert.
Acht Lab-Caddy-Header entfernt, fremde Blöcke bytegleich.
Journal Generation 13 abgeschlossen, keine Lease; Credentials/Bindings erhalten.
35 Sekunden Quieszenz tatsächlich lesend geprüft; keine Betriebsaktion wiederholt.

Down-Ergebnis:
`/Users/frank/vzctl-labs/controller-lab/evidence/lifecycle-f48-down-20261020T182901.716132Z/result.json`,
SHA256 `129d0a760ae380095feb68763e0833f216cb899f07969dd242715911e27c7a97`.
Aktueller Startplan enthält exakt drei nicht-brechende `start vm` für
`controller-lab/ops`, `controller-lab/server`, `controller-lab/worker`.
Kein automatisches Up/Apply, keine Prüfprozesse aktiv; auf konkreten UI-Job warten.

Zwei enge Korrekturen ausschließlich am vorhandenen Lifecycle-Prüfer:
Belegnamen als exakte bekannte Menge einschließlich `k3s`; Startplan gegen
qualifizierte `LAB_VMS` statt kurzer Namen. **Aktueller maßgeblicher SHA256:**
`d2dfdf9ff9a113206bf4c84037ebf4e82b00655f6390f56dde6c0aadbb3624d3`.
Duplikate/Unbekannte sowie fremde/kurze VM-Namen, Breaking und Extras abgewiesen.
Ursprünglicher rein lokaler Inputfehler bleibt unter
`evidence/lifecycle-input-preflight-f48-20261020.json`, SHA256
`88d5dcc5002f8a2add989aea05a7eecd2272303b453df54b490b127689a023a9`.
Historisches Down-`failed.json` bleibt mit SHA256
`e758e0d4e9c95e546057f108bfab4f5e77dace0840ba6cc544cc78d90d4264cc`.
Es nennt nur `down_disabled`; der ursprüngliche Einzelgatecode ist unbekannt.
Aktuelle tatsächliche Stop-/Registry-/Helper-/Routenguards und Quieszenz bestanden;
kein Produktfehler und keine Änderung am Produkt daraus behauptet.

Nächste direkte Aktion: Root startet UI-Apply für controller-lab und liefert
die echte Job-ID. Vorhandenen Lifecycle-Runner ausschließlich mit
`--phase observe-ui-apply --job-id <UUID>`, aktuellem Quellhash oben,
Retentionpin `1058d125…`, Adapterpin `6689bc…` und neun SHA-gebundenen Receipts
(`full`, `late`, vier Faultfälle, `retention`, `cp`, `down`) aufrufen.
Observer startet kein Apply und keine Consumerprobe. Native Bedienbarkeit
und erforderliche Consumer-Nachprobes nach Wiederanlauf separat ergänzen.

## Abschluss: natives UI-Apply und Nachprobes bestanden · 20.10.2026

Root hat das UI-Apply genau einmal ausgelöst, 20:39:54–20:40:31 MESZ (37 s).
Invocation-ID `ebbea683-ed47-4c7a-9cec-3c21050f1917` ist exakt Journal-ID
der Generation 14; die erste direkte Leseaufnahme zeigte running/ensure_service_claims.
Native aktive Bedienbarkeit, Fortschritt, gesperrte/freigegebene Aktionen und
Abschluss stehen in [ui-native-apply-2026-10-20.md](../ui-native-apply-2026-10-20.md).

Bestehenden Observer auf diese echte Invocation-/Journal-ID gebunden; keine
REST-jobId erfunden und kein neuer Runner/Proxy/Instrumentierungsweg angelegt.
Der Observerloop startete nach Abschluss und meldet korrekt `observed_active=false`;
die aktive Phase ist unabhängig nativ belegt. Aktueller Lifecycle-Prüfer-SHA256:
`aca08d472d2184649ca597c80155dda046b06ee28ce225550011e51c2fa908ec`.
Nur enges alternatives `--invocation-id`, eindeutiger neuer Journalbezug und
fester sanitisiert ausgegebener Gatecode wurden ergänzt. Produktquellen unverändert.

- Observer: `evidence/lifecycle-f48-observe-ui-apply-20261020T184206.679011Z/result.json`,
  SHA256 `90ca73dd28fbf96b1f7a3c7779e2a10a3e8cda6236ff4c120f4890dc7cf91deb`.
- Nachprobes: `evidence/post-ui-consumers-f48-20261020T184404.823467Z/result.json`,
  SHA256 `e805dfb334c549a279049b3079a9b4f00872fc0921e7c70ab6a028a2c8b4e845`.

Alle privaten Pfade relativ zu `/Users/frank/vzctl-labs/controller-lab/`.
Neue Registry frisch aktiv, neun Ready plus foreign Retained, Generation 14 done.
Nach separater Root-Freigabe genau einmal vorhandene Consumerprobes:
VM/Docker/K3s jeweils PostgreSQL/S3/OIDC wahr, beide HTTPS200.
Bestehende Daten zuerst gelesen, anschließend Persistenz/Bindings/Credentials/
Eingaben/geschützte Runtime unverändert. Sechs VMs laufen, drei IQ-Store-VMs
bleiben gestoppt. Kein zusätzlicher Job, Apply, Reconcile oder Lifecycle-Eingriff.

Ein transparenter Performancehinweis bleibt im abschließenden Health-Snapshot:
Worker ready, Service/Token okay, Queue 0, gespeicherte Exec-P99 8511 ms;
dadurch AGENT_DEGRADED und Stack-Exit1. Der Agent nutzt die letzten 32 Execs
ohne Zeitverfall, P99 ist deren Maximum, Schwelle 5000 ms. Kein aktueller
Backlog/Agentdefekt belegt; Root akzeptiert dies als Performancebeobachtung.
Kein Upgrade, Restart oder Exec-Spam; keine weiteren Status-/Metrikprüfungen.

Runtime-Paket vollständig abgeschlossen, keine Prozesse mehr aktiv.
Nächster Schritt gehört Root: GitHub-Kriterien und Release-Übergabe abgleichen.
Externe Erstinstallation/Update auf separatem macOS-26-Mac bleibt offen;
IQ-Store-Trust erst beim späteren regulären Apply, seine VMs nicht starten.

### Tatsächlich verwendeter Observer-Aufruf (Archiv, nicht wiederholen)

Quellenpin `aca08d…`, Retentionpin `1058d125…`, Adapterpin `6689bc…`.
Root hatte die vollständige neunfache Belegbindung angefordert:

```sh
python3 -B /private/tmp/vzctl-controller-lifecycle-f45.py --execute --phase observe-ui-apply \
  --retention-sha256 1058d12514aaaa2d44fb41612bd24e56b71029fa0296a0cbf8f980f1cab52d41 \
  --adapter-sha256 6689bc49e926aaa2729467712975e591d008092259a86d3260cabc30caa12e05 \
  --receipt full /Users/frank/vzctl-labs/controller-lab/evidence/full-live-f48-verified-20261020T165021.477100Z/result.json 534fac9e13afb1e01858d867fb65e4e99c82af7ec10b6d656fc44d352f245311 \
  --receipt late /Users/frank/vzctl-labs/controller-lab/evidence/followup-late-f48-20261020T180633.949929Z/result.json 80a3838ad504490e3b9b676379a02dc3c7f30d27a8c358cbedc2120308302f0b \
  --receipt secret-owner /Users/frank/vzctl-labs/controller-lab/evidence/fault-secret-owner-20261020T181333.397822Z/result.json a4d23a1933ed900680414b9518c23cac7ad0f9e6e7a5899d4ae5d099ee77cc2c \
  --receipt minio /Users/frank/vzctl-labs/controller-lab/evidence/fault-minio-20261020T181516.967081Z/result.json aa5b0f54d8199b4e59fd22b564caad9d90f31c10544799a9594031bb0c7a1086 \
  --receipt k3s /Users/frank/vzctl-labs/controller-lab/evidence/fault-k3s-20261020T181553.662987Z/result.json cfc1caa51dfaf92e57c466984c4ee132b54d8bc822424b7896e502410e3555ac \
  --receipt foreign-db /Users/frank/vzctl-labs/controller-lab/evidence/fault-foreign-db-20261020T181636.383891Z/result.json fd3bcb09eb95fdbc208522149c81d848ba53135d56e71bfc30e4bf3bc14b2430 \
  --receipt retention /Users/frank/vzctl-labs/controller-lab/evidence/retention-live-f48-20261020T182200.125655Z/result.json c7ffa96123639338c13fc27513f0ac3c8ea9ec526edb0eae943f3c3f0db18114 \
  --receipt cp /Users/frank/vzctl-labs/controller-lab/evidence/lifecycle-f48-cp-restart-20261020T182747.606692Z/result.json 3eae1d8a0d6d137ac28c6e9c60d029b5b04f400ecf300e6563b07572ed33148e \
  --receipt down /Users/frank/vzctl-labs/controller-lab/evidence/lifecycle-f48-down-20261020T182901.716132Z/result.json 129d0a760ae380095feb68763e0833f216cb899f07969dd242715911e27c7a97 \
  --invocation-id ebbea683-ed47-4c7a-9cec-3c21050f1917
```
