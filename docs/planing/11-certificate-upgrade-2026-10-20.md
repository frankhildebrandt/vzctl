# F45: Geplanter Zertifikatsprofil-Upgrade und Live-Rollout

Stand: 2026-10-20. **Aktive Trustziele und alle drei Lab-Consumer live abgenommen; drei bewusst gestoppte IQ-Store-Trustziele bleiben ausstehend.** Der erste reale `prepare` brach vor jeder Mutation mit `candidate_file_hash` ab. Legacy-K7 hatte zwischenzeitlich die drei Lab-Leafpaare einschließlich Keys erneuert. Das bestätigte einen Ablaufkonflikt; der korrigierte Quiesce-Ablauf unten ersetzt den früheren zeitlichen Kandidatenplan.
Finding [#81](https://github.com/frankhildebrandt/vzctl/issues/81).
Der neue deklarative Apply für die tatsächliche CA-/ConfigMap-Solländerung ist ausdrücklich autorisiert. Das ist keine Wiederholung der bisherigen Consumerprobe. Kein `--force`, kein Resume einer alten Generation, keine direkte Änderung von SQLite oder historischen Belegen.

## Aktueller Abschluss nach F48

Der reguläre F48-Lab-Apply ist erfolgreich: privater Beleg
`f48-apply-20261020T164823Z/result.json`, SHA256
`ecc6813884475b7bf5fb9519f2534fcdee3330491bcc18d6c6d394dbf8f143a2`.
Die anschließende Full-Abnahme bestätigt PostgreSQL, S3 und OIDC aus VM, Docker
und Kubernetes sowie HTTP 200 an beiden HTTPS-Consumerendpunkten. Beleg:
`full-live-f48-verified-20261020T165021.477100Z/result.json`, SHA256
`534fac9e13afb1e01858d867fb65e4e99c82af7ec10b6d656fc44d352f245311`.
Auch MCP-Status und Diagnose sind abgenommen; Beleg
`mcp-live-20261020T165132.868386Z/result.json`, SHA256
`1303378af03124475813ae98bf1558611bfb9c0b416d6305404e037d063a00e8`.
Installierte CLI: `f7cdececf68f6a2239cb0563d0f8287842ef0faa0612c1228fb3a395b7d384ab`.

Die folgenden Ausgangs-, Fehler- und Vorbereitungsabschnitte sind historische
Nachweise. Erfolgreiche F47-Recovery und F48-DNS-Migration bleiben getrennte
Transitionsbelege; alte Failed-Receipts wurden nicht umgeschrieben. Der echte
Repeat-/Idempotenznachweis steht noch aus. IQ-Store wird nicht automatisch
gebootet; `global_trust_complete=false` bleibt bis zum späteren regulären
Up/Apply dieser drei VMs korrekt.

## Bestätigte Ursache und Ausgangszustand

Docker verwendet Python 3.13.16/OpenSSL 3.0.22 mit `X509_STRICT`. OIDC scheitert mit Fehler 85: `Missing Authority Key Identifier`. Dieselbe Verbindung funktioniert im VM-Consumer mit Python 3.12.3/OpenSSL 3.0.13 ohne dieses Prüfflag. Die bestehende CA enthält nur SKI und BasicConstraints; KeyUsage fehlt ebenfalls. `mint_leaf` setzte AKI nicht. Netzwerk, DNS, CA-Dateizugriff und die UID-/Readonly-Mount-Prüfungen sind erfolgreich.

Der einmalige geprüfte Consumer-Nachlauf belegt VM PostgreSQL/S3/OIDC erfolgreich; Docker PostgreSQL/S3 erfolgreich, OIDC als `URLError`/HTTP 503. Die Kubernetes-Consumerprobe wurde danach nicht gestartet. TLS-Prüfung bleibt unverändert streng; kein `-k`, keine entfernten Prüfflags.

Öffentliche Ausgangsidentitäten:

- CA DER SHA256: `139db11cae693c8e41f045105cf34cf777a74b062330fd34c4f09141332868f4`.
- CA PEM SHA256: `ed63fb7da7f85e33ca7b2e607be69d9170535c3c32a36bb9266dbbbb73ecc522`.
- Root SPKI SHA256: `a3faef21ef5be7df69635cf6fc8db879d642d1f3a61251f6210bbe294e9091b1`.
- Root-Key und alle neun Leaf-Keys passen derzeit zu ihren Zertifikaten. Das wurde ausschließlich durch Vergleich der abgeleiteten öffentlichen Schlüssel geprüft. Alle Leaf-Keydateien haben Modus 0600.
- Aktueller Lab-Plan: Exit 0, keine Aktionen, kein stderr. Die vollständige bestehende Apply-Generation 7 bleibt historische Basis.

Das private Inventar liegt außerhalb des Repositories unter `/tmp/vzctl-f45-trust-inventory.json`, SHA256 `eebcce1704d121abe807f4ba908751d09c005ccf59157d8457248b6b13c8da3e`. Es enthält Metadaten/Hashes, keine Schlüssel- oder Credentialinhalte. Der redigierte TLS-Befund liegt in `/tmp/vzctl-consumer-tls-diagnosis-20261020.json`, SHA256 `5cc9e952f45d3178f33cb1ea3564ca3f534b0cda8d8073c04c843565c276bfd6`.

## Erfasste Trust-Ziele

`STATE` bezeichnet den bestehenden vzctl-State; `LAB` das bestehende controller-lab-Verzeichnis.

| Ziel | Gelesener Istzustand | Erforderlicher Übergang |
|---|---|---|
| `STATE/ca/root/{ca.crt,ca.key,fingerprint}` | Eine globale CA; Root-Key passt | Neues CA-Zertifikatsprofil und DER-Fingerprint; Root-Key byteidentisch |
| `STATE/ca/trust/vzctl-local.crt` | Gleiche aktuelle CA | Exakte Kopie der neuen öffentlichen CA |
| `STATE/ca/issued/<host>/{cert.pem,key.pem,meta.json}` | Neun vollständige Paare | Alle neun Leaf-Profile korrigieren; vorhandene Keys/SANs erhalten |
| Login-Keychain | Genau ein Zertifikat mit aktuellem CA-Fingerprint | Neue CA explizit vertrauen; alten Fingerprint gezielt ersetzen, keine pauschale CN-Löschung |
| System-Keychain | Kein entsprechendes CA-Zertifikat | Keine neue globale Installation erforderlich |
| Neun VM-Systemstores | Je CA-Datei, passender `/var/lib/vzctl/ca.fingerprint`, genau ein Root im Systembundle | CA-Injection und Bundle-Aktualisierung; pro VM Fingerprint und Trust nachweisen |
| `LAB/assets/ca.crt` | Aktuelle CA | Neue CA, Mode 0644; explizite Datei-Hashtransition |
| OPS `/opt/vzctl-lab/ca.crt` | Aktuelle CA | Atomare öffentliche Dateiersetzung; keine Änderung von Probe/Requirements/Bindings |
| Docker `proof:/opt/lab/ca.crt` | Readonly-Verzeichnismount derselben OPS-Datei | Neue Datei muss im bestehenden Container sichtbar werden; kein Container-Recreate erforderlich |
| `LAB/kubernetes/code.yaml`, ConfigMap `app/proof-code` | Aktuelle CA in `data.ca.crt` | Ausschließlich dieses Datenfeld deklarativ ändern; übrige Daten/Labels/Owner erhalten |
| Kubernetes `proof:/opt/lab/ca.crt` | Gleicher CA-Hash wie ConfigMap | Kubelet-Projection begrenzt abwarten; bestehende Deployment-/Pod-UID schützen |

VM-Inventar: `controller-lab/{ops,server,worker}`, `iq-store/{client-noble,client-resolute,docker}`, `neti/{main-node,neti-external,neti-home}`. **Aktualisierte Nutzerbaseline 14:34:55 UTC:** Lab und Neti laufen weiter (sechs VMs); die drei IQ-Store-VMs wurden vom Nutzer gestoppt. Ihre gespeicherten PID-Felder sind kein Laufnachweis. Ausschließlich eine erfolgreiche direkte Supervisor-Abfrage bindet den Zustand. Identitäten/Ressourcen aller neun, laufende PIDs der sechs und der gestoppte Zustand der übrigen drei bleiben geschützt; der Rollout startet keine VM.

Leaf-Inventar: `admin.svc.iq-store.vz.test`, `auth.svc.controller-lab.vz.test`, `docker.svc.controller-lab.vz.test`, `kube.svc.controller-lab.vz.test`, `main-node.svc.neti.vz.test`, `minio.svc.iq-store.vz.test`, `monitos.svc.neti.vz.test`, `s3.svc.iq-store.vz.test`, `store.svc.iq-store.vz.test`.

Die vier registrierten Stackkonfigurationen enthalten außer dem bekannten Lab keine weiteren expliziten CA-Datei-/Trust-Umgebungsreferenzen. Das ist **keine** Bestätigung über beliebige manuelle Browser-, Java-, App- oder nicht registrierte Truststores. Solche Fremdkopien werden nicht gesucht oder verändert. Backups, alte Evidence und historische Provider-/F19-Belege bleiben unverändert.

## Produkt- und Dateisicherheits-Gates

Der geprüfte Produktbefehl `vzctl certs ca upgrade --output DIR` erzeugt ausschließlich eine neue private Kandidatenwurzel anlegen, einschließlich vollständigem `ca/` und abschließendem Manifest; keine aktive Promotion oder Truständerung.

Vor einer Livefreigabe müssen folgende Eigenschaften getestet/reviewed sein:

1. Vorhandene ungültige oder halbe CA-/Leafpaare werden erklärt abgelehnt. `ensure_ca(false)` darf weder eine halbe Installation still ersetzen noch die alte CA ungeprüft als gesund melden.
2. Der endgültige Kandidat bewahrt die Root-Keybytes, Root-SPKI, Subject, SKI und Gültigkeit. Alle Leaf-Keybytes/SANs bleiben relativ zum endgültigen Quell-Snapshot **nach** CP-Quiesce erhalten. Genau die drei Lab-Legacypaare dürfen zuvor durch den alten bekannten K7-Writer regulär erneuert werden; daraus folgt keine Freigabe beliebiger Schlüssel- oder Quelländerungen. Private Dateien 0600, Kandidatenverzeichnis 0700, keine Symlink-Übernahme und kein Überschreiben vorhandener Ausgabeziele.
3. Alle CA-/Leaf-Dateien und Metadaten sind vollständig, bevor das Manifest als Commit-Marker erscheint. Abbruch hinterlässt nur einen unfreigegebenen Kandidaten; Wiederverwendung ohne explizite Prüfung ist verboten.
4. Strikte vollständige Chainverifikation gegen die neue CA und alle neun Leafs, einschließlich EKU/KeyUsage/AKI/SKI/SAN/Keymatch. Nicht nur Extension-Präsenz testen. Ein ausgelassenes Altprofilmerkmal muss in einem Negativtest scheitern.
5. Normales Minting darf kein neues Zertifikat mit altem Schlüssel oder umgekehrt für einen Leser sichtbar machen. Einzelne atomare Dateirenames sind noch keine atomare Veröffentlichung eines Paars mit neuem Schlüssel. Schlüsselwiederverwendung oder eine tatsächliche Generation-/Paar-Promotion ist nötig.
6. Root-Keybytevergleich und öffentliche SPKI-Identität erscheinen als Bool/Hash im privaten Manifest, nie als Keyinhalt. Alle alten/neuen öffentlichen Zertifikatsfingerprints werden explizit gebunden.

## Geplanter Ablauf mit klaren Haltepunkten

### 1. Kandidat, Rückfallmaterial und Writer-Koordination

- Gebauten, geprüften CLI-Stand zunächst separat verwenden. Ein vorzeitig installiertes CLI mit strikter Altprofil-Ablehnung könnte laufendes K7-Reconcile gegen die noch alte CA stören.
- Der bereits gebaute private Kandidat bleibt unveränderte historische Profilreferenz. Er ist nach der laufenden Legacy-Rotation ausdrücklich **kein** promotierbarer finaler Quellsnapshot. `prepare` bindet seine validierten Profile, globale Root/CA/Trustdateien, vollständiges Inventar und die sechs Nicht-Lab-Paare sowie SAN-/Subject-/Metadatenvertrag der drei Lab-Hosts. Keine aktive CA-/Keychain-/VMänderung in dieser Phase.
- CLI und öffentliche Lab-Inputs bereits vorbereitend privat sichern. Der endgültige vollständige CA-/Keybackup wird erst nach CP-Stop aus der dann stabilen Quelle erstellt. Bestehende Evidence und der historische Referenzkandidat bleiben unverändert.
- Snapshot bindet installierte Binaries, aktive Stackkonfigurationen, neun VM-IDs/Zustände/Ressourcen und die sechs laufenden VM-PIDs, Net/Edge/CP/Caddy-Prozesse, Controllerregistrierung, Journal und fehlende aktive Lease/CLI-Mutation.
- Vor Promotion alle Zertifikatschreiber ausschließen. K7 kann über den bestehenden Ingresspfad minten. Entweder ein getesteter gemeinsamer Writer-Lock außerhalb des auszutauschenden CA-Baums oder ein explizit begrenzter CP-Stop mit anschließender dokumentierter neuer CP-PID. Nicht bloß hoffen, dass kein 30-s-Reconcile stattfindet. VMs und Net bleiben dabei aktiv.
- Das vollständige aktive `ca/`-Datei- und Verzeichnisinventar muss exakt durch das Manifest und dessen notwendige Elternverzeichnisse abgedeckt sein. Unbekannte Dateien, versteckte Zusätze, zusätzliche leere Verzeichnisse, Symlinks oder Hardlinks blockieren den vollständigen Generationstausch. Nichts wird still ausgelassen. Die alte Generation bleibt privat zurücktauschbar.
- Nach tatsächlichem CP-Stop und Abschluss aller CLI-Writer zuerst stabile Root-/Nicht-Lab-/Inventarguards prüfen. Genau `auth`, `docker`, `kube` unter `svc.controller-lab.vz.test` dürfen als vollständige Legacy-Paare erneuert sein. Für diese drei werden Rootsignatur, Keymatch, Subject/SAN, Metadatenfingerprint und erwartete Legacy-Gültigkeit unabhängig geprüft. Einzelner fremder Key, andere SANs, unbekannte Metadaten, Root-/Nicht-Lab- oder Inventaränderung blockieren. Danach den finalen Kandidaten **einmal unter Quiesce** mit der geprüften CLI erzeugen, alle zehn Ketten strikt prüfen und Source-/Manifest-/Keyhashes in einem neuen effektiven Plan binden. Kein Rennen gegen das 30-Sekunden-Intervall, keine Umdeutung des alten Manifests.

### 2. Lab-Solländerung vor Aktivierung validieren und planen

- Kandidatenkopie von `kubernetes/code.yaml` erzeugen: ausschließlich `data.ca.crt` wird ersetzt; Probe, Requirements, Name, Namespace und Owner bleiben unverändert.
- Öffentliche CA-Dateikopie und Code-Manifest erhalten eigene alte/neue SHA256-Werte. `hypernetwork.config.yaml`/`full.yaml` bleiben byteidentisch, soweit der endgültige Kandidatenplan keine andere begründete Änderung verlangt.
- Kandidatenmanifest privat strukturell validieren: ausschließlich `data.ca.crt` darf sich ändern. Ein kopiertes Staging-Lab ist kein gleichwertiger Plan: Input-Ressourcen benutzen absolute Pfade. Deshalb nach kompatibler CA-/CLI-Promotion und CP-Neustart genau `code.yaml` atomar ersetzen, unmittelbar reales `validate` und `plan` ausführen und genau ein nicht-breaking `update/input` für den ursprünglichen absoluten `code.yaml`-Pfad verlangen. Erst danach Apply. Bei unerwartetem Plan vor Apply die Inputdatei per Hashguard zurücksetzen. K7 appliziert diese Nutzer-ConfigMap nicht eigenständig. Dieser begrenzte Aktivierungsschritt ist ausdrücklich Teil der Migration.
- VM-Recreate, Netzwerk-/Hardwareänderung, fremde Claims, Provider- oder Secretänderung, Container-Recreate und jede unerwartete oder breaking Aktion blockieren. Kein `--force` und kein pauschales Akzeptieren einer Kategorie.
- Ein neuer deklarativer Apply ist für diese notwendige Solländerung autorisiert. Er erhält eine neue Generation/Baseline. Die bisherige abgeschlossene Generation wird nicht editiert oder resumed.

### 3. Finaler Kandidat unter Quiesce, atomare Aktivierung und TLS-Reload

- Vor jeder Veröffentlichung muss der unter Quiesce erzeugte finale Kandidat samt vollständigem Sourcebackup, Strictnachweis und `effective-plan.json` abgeschlossen sein. Fehler während dieses Kandidatenbaus verändern weder CA noch installierte CLI und starten den CP wieder; ein unbekannt veränderter Schutzbestand verlangt manuelle Klärung.
- Den vollständig geprüften `ca/`-Baum als eine Generation auf demselben Dateisystem veröffentlichen. Für einen echten atomaren Verzeichnistausch ist macOS `renameatx_np(RENAME_SWAP)` oder ein gleichwertig geprüfter Mechanismus erforderlich; ein einfaches zweistufiges `mv old; mv new` hinterlässt eine Lücke.
- Das Werkzeug muss einen isolierten Abbruch-/Rollbacktest bestehen. Falls stattdessen eine feinere Promotion verwendet wird, muss deren Paar-/Lock-/Crashvertrag vorher separat bewiesen sein. Keine Freigabe auf Grundlage mehrerer ungesicherter `fs::write`/truncate-Aufrufe.
- Root- und Leaf-Keybytes nach Promotion erneut gegen Altbestand prüfen. Neuer Fingerprint muss zu `ca.crt` und Trustkopie passen; Manifest vollständig.
- **PEM-Ersetzung allein reloadet Caddy nicht:** `EmbeddedProcessManager.ensure` überspringt bei identischer Spec und laufendem Prozess den Neustart; die Spec enthält den Caddyfilepfad, keinen PEM-Digest. Ein unverändertes `ingress.ensure` reicht deshalb nicht als Reloadnachweis.
- Ein explizites geprüftes Caddy-Reload ist Teil der Aktivierung. Die installierte Caddy-Hilfe bestätigt `reload --config <path> --adapter caddyfile --address <admin-listener> --force`; dieses Caddy-Flag erzwingt das erneute Einlesen unveränderter Konfiguration und ist kein Freigabeflag für einen vzctl-Apply. Vor einem solchen Lauf müssen Adminlistener und exakter laufender Managed-Caddy-PID zusammenpassen; aktiven Configpfad/Hash aus dem Prozessvertrag binden, Konfiguration gegen neue PEMs validieren. Kein beliebiges Adminziel verwenden.
- Danach tatsächlich ausgelieferte Leaf-Fingerprints/SANs für alle im gebundenen Caddyfile aktiven Hosts prüfen. Die neun Kandidatenketten werden unabhängig davon alle lokal strikt verifiziert; ein durch Nutzer-Down entfernter Host wird nicht künstlich reaktiviert. Wenn der eng geprüfte Reloadpfad nicht verfügbar ist, abbrechen und einen separaten Edge-Neustartplan prüfen; kein automatischer Fallback. Ein Edge-Neustart betrifft auch eingebettete OIDC-Prozesse und kann deren flüchtige Login-Sessions beenden. Net und neun VMs dürfen in keinem Fall neu starten.
- Das kurze Quiesce-Fenster endet **hier**: CA-Baum und kompatible CLI sind vollständig installiert und der TLS-Reload ist abgeschlossen. CP wieder starten und neue PID/Health/Registrybindung im Receipt festhalten. Erst danach folgen `ca_inject` und der neue deklarative Apply, die beide den laufenden CP benötigen. K7 darf jetzt mit dem neuen kompatiblen Profil arbeiten. Produktseitig müssen identische gültige Leafprofile/SANs idempotent bleiben, vorhandene Leafkeys immer erhalten werden; diese Zusage ist vor Livefreigabe zu testen.

### 4. Vertrauenskopien und neue deklarative Baseline

Voraussetzung: CP ist nach Abschnitt 3 wieder erreichbar; neue CLI und globales CA-Profil sind kompatibel. Das frühere CP-Quiesce gilt nicht für die folgenden RPC-/Apply-Schritte.

- Login-Keychain ausschließlich anhand des alten/neuen DER-Fingerprints behandeln; neue Vertrauensentscheidung prüfen und alte CA nicht als gleichnamigen zweiten unkontrollierten Trustanker liegenlassen. Berechtigungspfad durch Root koordinieren. Keine System-Keychainänderung ohne tatsächlichen Bedarf.
- Ausschließlich die sechs laufenden Gäste über den vorhandenen `ca_inject`-Vertrag bedienen; gegebenenfalls dokumentierter Legacy-Fallback. `certs rollout`/`certs verify` sind derzeit Stubs und dürfen nicht als erfolgreiche Rolloutbefehle verwendet werden.
- Die drei gestoppten IQ-Store-VMs bleiben explizit `pending_stopped`. Ihre Trustkopien werden beim späteren regulären Stack-`up`/`apply` über `ensure_ca_rollout` migriert; kein automatischer Start. Ein bloßes `vm start` garantiert diese Trustmigration nicht. Bis zum späteren Nachweis darf kein global vollständiger Trustrollout gemeldet werden.
- Pro laufendem Gast: neue Zertifikatsdatei, neuer Fingerprint, Systembundle enthält genau die neue CA, strikte Verifikation erfolgreich. Der Helper ersetzt zuerst die CA und aktualisiert danach Bundle/Fingerprint; ein Fehler in einem späteren Schritt ist **teilweise Migration**, kein Erfolg. Je VM einen Abschlussmarker erst nach allen Prüfungen schreiben.
- Lab-Asset und OPS-Kopie atomar ersetzen; ihre Hashes danach auch aus dem bestehenden Docker-Mount lesen. Da `/opt/vzctl-lab` als Verzeichnis gemountet ist und die Probe den SSLContext pro Request erzeugt, ist hierfür kein Docker-Recreate erforderlich.
- Den vorab geprüften neuen Lab-Apply exakt einmal durchführen. Keine automatische Wiederholung bei Fehler; sichere Phasen/Ergebnisdateien, kein alter Resume. Er soll die neue deklarative ConfigMap und Input-Baseline herstellen und darf keine ungeplanten VM-/Provideränderungen erzeugen.
- Nach ConfigMap-Update auf neue CA im bestehenden Pod begrenzt warten; Projektionsverzögerung ist kein Grund für einen ungeplanten Pod-/Deployment-Restart. Nach Ablauf konkrete Phase melden.
- CP/K7 bleibt während dieses Trust-/Apply-Fensters bewusst aktiv. Nach jedem Schritt Registry/Generation und die geschützten VM-/Net-Identitäten nachweisen; bei einem neuen Fehler stoppen und Teilzustand melden. Erst wenn CA-Kopien, neue Apply-Generation, Ressourcenidentitäten und Planbaseline stimmen, Migration als abgeschlossen markieren. Letzter vollständiger Plan muss leer sein; die neue Baseline ist ein eigener Beleg.

### 5. Abnahme und Folgebelege

- Zuerst lesend: alle Bestandsstack-Healthchecks, Host-/Guest-DNS, TLS-Handshake/Discovery unter Docker-Python-Strictprüfung und ausgelieferte Zertifikate. Keine Nutzdatenprobe zum Verschleiern eines Trustfehlers.
- Danach genau eine gesondert geprüfte Consumer-Abnahme mit den sicheren Phasen/Projektionen aus `/tmp/vzctl-controller-full-verified.py` (SHA256 `a55f3fff7268d8a2087586d8c603d3a60b67b385a64ffbf86ac563b35000990d`) als Referenz. Neuer Runner muss die neue Zertifikat-/CLI-/ggf. CP-/Edge-Transition und neue Apply-Generation explizit binden; der alte Runner wird nicht editiert oder erneut ausgeführt.
- Guest-Exit 1 und HTTP 503 behalten ihre allowlisted Teilresultate. Erfolg benötigt Transport-Exit 0, Guest-Exit 0 beziehungsweise HTTP 200 und genau PostgreSQL/S3/OIDC=true. Niemals rohe Responsebodies, Token, Location, Providerfehler oder Credentials ausgeben.
- Neue erfolgreiche Receiptkette mit alten historischen Belegen verknüpfen; neue Source-/CA-/ConfigMaphashes als Transition dokumentieren. Das ursprüngliche `code-before.json` bleibt historische Kopie. Repeat/Late müssen die explizite CA-Transition verstehen; bloß ihre Hashpins ändern wäre falsch.
- Reihenfolge bleibt erfolgreiche Full-Consumer-Abnahme → MCP mit genau fünf Claims → Repeat → Late mit neun Claims → getrennte Ownership/Fault-/Retention-/Lifecyclefälle.

## Rückfall und Abbruch

| Ausfallpunkt | Reaktion |
|---|---|
| Kandidatenbau/Strictprüfung | Kandidat ungefreigeben lassen; Livezustand unverändert |
| Promotion vor Reload | Aktive Baumidentität/Commit-Marker prüfen; unter Writer-Sperre exakt alte geprüfte Generation zurücktauschen |
| Caddy-Validierung/Reload | Kein Erfolg; alte CA-/Leafgeneration wiederherstellen und explizit reloaden, ausgelieferte Fingerprints prüfen |
| Einzelner Guest-/Keychain-/Lab-Trustschritt | Teilstatus und genaue abgeschlossene Ziele erhalten; nur eigene zuvor gesicherte Zielversionen zurückrollen, nicht fremde spätere Änderungen überschreiben |
| Deklarativer Apply fehlschlägt | Neue Generation/Fehler konservieren, nicht automatisch resume/abort. Ressourcen-/Trustteilstand prüfen; Rücknahme benötigt ebenfalls passende deklarative Solländerung |
| Consumerprobe fehlschlägt | Sichere Phase/Teilresultat melden; keine Wiederholung, kein Abschalten von TLSprüfungen |

Root-Key bleibt im Vorwärts- und Rückfallpfad unverändert. Identisches SPKI/Subject erhält die kryptografische Beziehung, ist aber **kein** Ersatz für Tests gemischter alter/neuer Trustzustände. Rollback gilt erst als fertig, wenn die tatsächlich publizierten Certs, Systemstores und die zugehörige deklarative Baseline wieder nachvollziehbar zusammenpassen.

## Aktueller Live-Stand und offene Gates

Die geprüfte Promotion wurde von Root erfolgreich ausgeführt. Die neue CA-/Leafgeneration und CLI `ccb10e53…` sind aktiv; Root-/Leaf-Keyerhalt bezieht sich auf die endgültige Quelle nach CP-Quiesce. Der neue CP hat PID 64686. Die sechs laufenden VM-PIDs sowie Net 95742, Edge 30115 und Caddy 43773 blieben unverändert; drei IQ-Store-VMs bleiben nutzergewollt gestoppt. Die bisherige Controllergeneration `ffef26e4-a24d-4d2f-8359-c50a337a358a` meldete nach dem Neustart einen frischen erfolgreichen Versuch mit fünf Ready Claims.

Der private Beleg `f45-profile-upgrade/promote-result.json` hat SHA256 `784fe97d76e19bd2df0b04c3c0f40bfb36adfe62257cb079cee2f69b29388428`. Anschließend wurden die sechs laufenden Guest-Systemstores erfolgreich migriert. Die drei gestoppten Trustziele bleiben ausdrücklich offen.

Die ursprüngliche Keychainphase brach an der Inventarprüfung ab. Eine separat geprüfte Recovery ersetzte die exakt bekannte alte Zertifikatsinstanz bei unveränderten TrustSettings; der neue Keychainabschluss ist erfolgreich (privater Recoverybeleg SHA beginnt `9cb0…`). Der danach ausgeführte Lab-Apply erreichte Generation 8, Journal `c21fade8-a4e8-4e71-9852-aff12f748fd9`, scheiterte aber in `ensure_kubernetes` mit F47 / [#83](https://github.com/frankhildebrandt/vzctl/issues/83).

F47 ist direkt durch die Kubernetes-Besitzmetadaten belegt: `app/proof-code` besitzt zwei Einträge für `vzctl-kubernetes`, einmal `Apply`, einmal `Update`; beide beanspruchen `data.ca.crt` und die Manifest-Digestannotation. Der frühere imperative Create-Eintrag blockiert die spätere SSA-Wertänderung trotz gleichen Managernamens. ConfigMap und Pod enthalten weiterhin die alte CA; aktive Host-CA, sechs Guest-Systemstores, Labasset sowie OPS-/Docker-CA sind bereits neu. Die fünf Claims bleiben Ready, die Registrygeneration unverändert. Drei Neti-HTTPS-Prüfungen waren HTTP 200 / TLS 0; die sechs laufenden VMs und Dienste blieben unverändert. Keine weitere Consumerprobe wurde gestartet.

Der vorbereitete F47-Fix übergibt ausschließlich eigene Update-Felder, die im aktuellen Soll oder bereits im eigenen Apply-Besitz liegen. Zuvor per Apply verwaltete, jetzt entfernte Felder bleiben dadurch regulär durch SSA entfernbar; nicht deklarierte Serverdefaults bleiben im Update-Besitz. Falls nötig stellt eine unveränderte Projektion der aktuellen Werte zunächst Apply-Besitz her. Der anschließende selektive JSON-Patch testet UID, ResourceVersion und vollständige ManagedFields; fremde Manager sowie Ressourcenwerte bleiben unverändert. Es gibt weder Force noch Ressourcen-Recreate oder globalen ManagedFields-Reset.

Die echten isolierten API-Proben belegten zwei Normalisierungen: Kubernetes lässt den eigenen Update-Strukturmarker `.` bei granularen Maps sowie beim Listencontainer weg, obwohl alle konkreten `f:`-Blätter bzw. `k:`-/`v:`-Einträge im Apply-Besitz stehen. Der Fix unterscheidet anhand des tatsächlichen Werttyps und Pfadkontexts diese Container von den Listenelementen. Deren eigene Mitgliedschaftsmarker, atomare und Dot-only-Knoten sowie fehlende oder mehrdeutige Elemente bleiben streng geprüft. Beide gemessenen Fieldsets wurden zuerst als rote Regression reproduziert und anschließend grün abgesichert.

Die **echte isolierte Kubernetes-Vertragsabnahme ist jetzt vollständig bestanden**: alle zehn Fälle, sechs bestätigte Migrationen, vollständiges Cleanup und unveränderte Produktionsruntime, Generation 8 sowie Inputs. Geprüft wurden Legacy-only- und gemischter Update-/Apply-Besitz, Create, Änderung, Idempotenz, Fremdfelder, Fremdowner, Service-Defaults, UID-Schutz und RV-Retry. Der UID-Gegenfall verwendet bei neuer UID denselben Sollwert, damit ein unabhängiger Feldkonflikt den Identitätsnachweis nicht verdecken kann; Ersatzobjekt und fremder Manager bleiben unverändert.

Der private Erfolgsbeleg `evidence/f47-k3s-contract-20261020T161232.899758Z/result.json` hat SHA256 `a690a20fdeaf255c82f0db57bdf6830bfefc40dfbea21d07e735581e5a8057fa`. Die vorherigen fehlgeschlagenen Testbelege bleiben erhalten. Der Produktfix ist API-seitig abgesichert. Anschließend bestanden die warningfreie Workspaceprüfung mit 489 Tests (zwei ignoriert; der Live-Harness wurde separat erfolgreich ausgeführt) und der Releasebuild. Die CLI `fc921a24463d9baa7f57b09d6aaa6f1223b4bf51a5a2260c717482fd576a30d4` ist jetzt installiert. Der private Installationsbeleg `pre-f47-cli/result.json` hat SHA256 `5bcf3712339050091c2b8c1cfd75d5df8a5ed912ec26c3ad03e35a2768fa6404`; Vorher-/Nachherbaseline sind identisch, ohne Dienst- oder VM-Neustart.

Der einmalige Recoveryrunner ist erfolgreich abgeschlossen. `f47-recovery/result.json` hat SHA256 `56bdba86a394760175659feffe1e77b6e2a4cb5382874a599396bb6b7b4546a3`. Journal-ID, Generation 8 und Payload blieben identisch; der Journalstatus ist `done`. Die neue Registrygeneration `1636a8e1-2584-4ab5-aeb7-1db5bd445265` meldet fünf Ready Claims ohne Warnungen. Produktionsressourcen-UIDs blieben gleich, ConfigMap und bestehender Pod verwenden die neue CA, der Abschlussplan ist leer.

Die anschließende Consumerabnahme ist noch nicht vollständig bestanden: VM und Docker prüfen PostgreSQL, S3 und OIDC erfolgreich; Docker-HTTPS liefert 200. Im Kubernetes-Consumer bestehen PostgreSQL und S3, während OIDC mit `URLError` und der Gesamtendpunkt mit HTTPS 503 scheitert. Die Ursache ist separat als F48 / [#84](https://github.com/frankhildebrandt/vzctl/issues/84) belegt: CoreDNS erbte den K3s-Fallbackresolver statt Stack-DNS. Die direkte Anfrage an die Bridge-DNS-Adresse und die direkte strikte TLS-/Discoveryprüfung aus dem Pod sind erfolgreich. Der native K3s-Resolverpin mit begrenztem CoreDNS-Rollout wird separat implementiert und geprüft; noch keine Liveänderung für F48. Das widerlegt den erfolgreichen F47-Recoverybeleg nicht, erlaubt aber noch keinen vollständigen Full-/MCP-/Folgeabschluss.

Der neue, ausdrücklich gebundene Recoverybeleg soll den Abschluss derselben fehlgeschlagenen Generation 8 per regulärem `--resume` dokumentieren. Historische Fehler-/Trustbelege bleiben unverändert; ein nachträglich erfundener ursprünglicher `lab-apply-result.json`-Erfolg ist ausgeschlossen.

Noch offen sind:

- Ursache und Abschluss des Kubernetes-OIDC-Consumerfehlers. Echte Kubernetes-Vertragsabnahme, Workspaceprüfung, CLI-Austausch, Gen8-Recovery, CA-Projektion und leerer Abschlussplan sind bereits belegt.
- Einmalige strikte Consumerabnahme und danach MCP, Repeat/Late sowie getrennte Fault-/Retention-/Lifecycleprüfungen auf der neuen Baseline.
- Spätere Trustmigration der drei gestoppten IQ-Store-VMs beim regulären Stack-Up/Apply, ohne automatischen Start im aktuellen Slice.

Die folgenden Angaben zur ursprünglichen Referenz und Vorbereitungsbaseline sind historische Belege; sie ersetzen keine aktuellen Receipts.

## Runner-Vorbereitung

`/tmp/vzctl-ca-rollout-quiesced.py` wird in getrennten Phasen vorbereitet: `prepare`, `promote`, `guest-trust`, `keychain`, `lab-apply`. Jede mutierende Phase benötigt einen ausdrücklich geprüften Aufruf sowie SHA-gebundene Vorgängerbelege; keine Phase läuft beim Import. Der historische Runner `9c73459cbe4757ac484e8222f967bd4e7bf02212349782dae1628761c5ad0a9f` bleibt unverändert. Sein realer `prepare` verweigerte die veraltete Quelle korrekt, ohne Workdir/CP-Stop/Mutation. Der neue Quiesce-Runner ist SHA256 `7d92a477bc3057031791cadebcfa3d80556b85e6ab236c911071ae5fbe3343ed`; unabhängiger Delta-Review abgeschlossen, 90 Producer- und 133 Consumerchecks durch Edge erneut bestanden. `/tmp/vzctl-ca-rollout-quiesced-offline.py` bestätigt 90 Checks: echte atomare Verzeichnistausche, sieben Promotion-/Fehlerpfade, tatsächliche Rotation der drei Lab-Keys/Zertifikate und anschließend den echten Rust-Kandidatenbau gegen einen isolierten Temp-State. Die Vorbereitung ist noch keine Livefreigabe.


Artefakte und historische Vorbereitungsbaseline:

- Finale CLI F45/F46: `ccb10e53bcbb72370e91fde7c38c09e000f737c38beec7211cae88e903d2ac1b`.
- Historische private Profilreferenz, nicht mehr final promotierbar: `/private/tmp/vzctl-f45-candidate-20261020T145701Z`.
- Historisches Referenzmanifest: `9eaaa01293e983154d3bb29aaa9e390d7a5f471c07ec94b32c55e37c3ff86ae6`.
- Historischer tatsächlicher Strictnachweis aller zehn Referenzketten: `/private/tmp/vzctl-f45-candidate-strict.json`, SHA256 `785ac9313c12d5e207e0325fd2b4f00341d0b9d046d5470599192b696f9fe6eb`.
- Root hat die frische Runtimebaseline lesend geprüft: sechs laufende VMs, drei gestoppte IQ-Store-VMs, CP 10852, Net 95742, Edge 30115, Caddy 43773. Das aktive Caddyfile enthält fünf Hosts (drei Lab-, zwei Neti-Hosts); sein Adminlistener wurde dem Managed-Prozess eindeutig zugeordnet. `prepare` bindet diese Werte erneut, keine veralteten PID-Konstanten.

`promote-result.json` bindet die unveränderte Controllerregistrierung und einen tatsächlich nach dem CP-Neustart abgeschlossenen periodischen Versuch mit fünf Ready Claims. Ein fehlgeschlagener Start kann bereits einen CP-Prozess erzeugt haben: vor **jedem** Rücktausch wird der reale CP-Prozessbestand erneut erfasst, der Dienst gestoppt und ein leerer Bestand nachgewiesen. Der Rückfall prüft die alten servierten DER-Fingerprints, Signatur und Hostnamen mit dem bekannten Altprofil; er behauptet ausdrücklich keinen Strict-Erfolg. Produkt- und Consumer-TLS bleiben unverändert streng.

`lab-apply-result.json` enthält SHA-gebundene Plan-/Manifest-/Promotion-/Trustbelege, aktuelle Binaryhashes, alte/neue CA-/Codehashes, vollständige vorherige/aktuelle Labinput-Hashes, die neue abgeschlossene Journalgeneration samt Payloadhash, neue Registergeneration und geschützte VM-/Workloadidentitäten. Die drei gestoppten Trustziele bleiben `pending_stopped_trust`; `global_trust_complete=false` ist bis zu deren späterem regulären Up/Apply ausdrücklich Teil des Vertrags.


Im korrigierten v2-Vertrag bindet `plan.json` die vorbereitete stabile Quelle und historische Profilreferenz. Erst nach CP-Quiesce entsteht `work/final-candidate/manifest.json`. `effective-plan.json` bewahrt alle vorbereiteten Planfelder außer dem Zustandsmarker und ergänzt `prepared_plan_sha256`, finales `manifest_sha256`, vollständige endgültige Quell-/Kandidateninventare, die drei beobachteten Lab-Paare und neue CA-/Codehashes. `promote-result.json` und `lab-apply-result.json` binden diesen effektiven Plan per SHA256. Damit beziehen sich Consumer-/MCP-Folgebelege auf den tatsächlichen finalen Kandidaten statt auf eine veraltete Vorstufe. Der Abschlusszeitpunkt wird erst **nach** allen Registry-/Prozess-/Input-Snapshots gesetzt; ein parallel fertiggestellter K7-Versuch kann dadurch nicht nach dem angeblichen Receiptabschluss liegen.
