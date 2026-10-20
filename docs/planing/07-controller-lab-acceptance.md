# K2–K7: isolierte Live-Abnahme

Stand: 20.10.2026. **Live-Abnahme begonnen, noch nicht abgeschlossen.** Neue CLI
und Daemons sind installiert, drei Lab-VMs laufen und beide K3s-Nodes sind bereit.
Der Bootstrap ist nach F31 erfolgreich abgeschlossen (Resume Exit 0, vier etablierte
CRDs, Namespace app aktiv). F19-Neustart und Provider-Setup sind inzwischen
bestanden. Nach F39 melden alle fünf Claims `Ready`; die echte Consumer-Abnahme
und die gezielte Fortsetzung des angehaltenen FullApply-Journals sind noch offen.
Aktuelle Nachweise stehen in [09-live-update-acceptance.md](09-live-update-acceptance.md).
Die folgenden Abschnitte beschreiben den ursprünglichen Aufbau, keine
Anleitung zum ungeprüften Wiederholen auf dem bestehenden Lab und keine pauschale
Erfolgsmeldung. Zugehöriger Vertrag: [service-controllers-v1](../specs/service-controllers-v1.md).

## Umfang, Voraussetzungen und harte Grenzen

Das Lab heißt ausschließlich `controller-lab`. Es nutzt drei neue VMs:

| VM | Aufgabe | vCPU | RAM | logischer Datenträger |
|---|---|---:|---:|---:|
| `controller-lab/ops` | PostgreSQL, MinIO, Docker, VM-Probe | 4 | 4 GiB | 30 GiB |
| `controller-lab/server` | K3s-Server | 2 | 3 GiB | 30 GiB |
| `controller-lab/worker` | K3s-Agent | 2 | 2 GiB | 30 GiB |

Zusätzlich: Host-/Dateicache, Image-Downloads, temporärer Go-Build im `ops`-Gast.
Mindestens 25 GiB tatsächlich freier Host-Speicherplatz; APFS-Klone sparen
anfänglich Platz, garantieren aber keinen kleinen Endverbrauch. **9 GiB zusätzliches
VM-RAM ist keine Aussage über derzeit verfügbaren RAM.** Auf diesem Host wurden
64 GiB physisch, sechs vorhandene VMs und während Builds rund 28 GiB komprimierter
Speicher beobachtet. Erst nach Abschluss der Builds RAM-/Speicherdruck und
Ressourcen der sechs Bestands-VMs erneut prüfen. Kein fremdes Lab dafür automatisch
stoppen; bei Speicherdruck den Lauf verschieben.

Netze: `lan=10.123.0.0/24`, Docker-Bridge `10.124.0.0/24`,
Pods `10.244.0.0/16`, Services `10.245.0.0/16`. Vor Beginn dürfen diese Netze
keine vorhandenen Netze/VPN-Routen überlappen. `ops` hat LAN `.10`, Server `.11`,
Worker `.12`, Docker-Bridge `.2`. Alle Provider laufen nur in neuen Gästen.
PostgreSQL/S3 werden nicht auf Host-Ports veröffentlicht; HTTPS nutzt den
bestehenden Edge-Multiplexer mit ausschließlich neuen `*.svc.controller-lab.vz.test`
Hosts. OIDC verwendet den freien Host-Loopback-Port `15556`.

Der Generator enthält jetzt eine Route `lan → containers via ops` und sechs
enge Allow-Regeln: LAN→Container TCP 8080/ICMP sowie Container→Internet DNS/HTTP/HTTPS
und Container→LAN DNS/HTTPS/PostgreSQL/K3s/MinIO. Alle anderen neuen Forward-Verbindungen
werden verworfen; bestehende Rückverbindungen bleiben erlaubt. Das ist zugleich
die deklarative Grundlage für den Router-Bootnachweis F19.

**F33 ist vor Aktivierung dieser Regeln installiert worden.** Der alte RouteCLI-
und Stack-Apply-Pfad konnte andere Projekte erfassen; das wurde im read-only
Labplan erkannt, ohne Regeln anzuwenden. Aktive Config und beide Labvorlagen
sind inzwischen konsistent ergänzt, Validate/Plan und ausschließliches
Lab-Apply erfolgreich. Nach F34: Regel-/Dateihashes und Boot-ID erfassen.
Dann nur `controller-lab/ops` geordnet neu starten.
Abnahme: neue Boot-ID, Restore-Service aktiv/enabled, identische eigene Regeln
und gespeicherte Routekonfiguration, Docker-DNS/HTTPS-Egress weiter verfügbar.
Keine Neti-Regel und kein bestehender VM-Helper darf dabei verändert werden.

Aktueller Live-Stand nach F33: gemeinsames CLI-/CP-Update installiert, neuer
Plan ausschließlich für ops, drei Labconfigs ergänzt und Stack-Apply erfolgreich.
Die Docker-Vorprobe stoppte den Ablauf **vor** dem Reboot. F34 ist belegt:
Container-DNS zeigt korrekt auf `10.123.0.0`, aber eine zusätzliche leere
`ip/filter/FORWARD`-Chain mit Policy Drop blockiert DNS und TCP gleichermaßen.
Unser Router-CloudInit enthält selbst `iptables -P FORWARD DROP`.
Die spätere `inet/vzctl`-Allow-Tabelle kann diese zweite Sperre nicht aufheben.
Der Fix verlagert Bootstrap-Deny in die eigene Tabelle und migriert den Altfall
nur mit engem Herkunfts-/Leerheitsnachweis in einer atomaren nft-Transaktion.
Kein manuelles pauschales Flush und kein ungeprüfter DNS-Override.
Für bestehende VMs installiert der Runtime-Helper ebenfalls Start-Guards:
persistentes Forwarding bleibt beim Boot zunächst aus; erst nach eigenen
Deny-Regeln wird es aktiviert. Bei fehlgeschlagener nft-Transaktion bleiben
diese eigenen konservativen Boot-Guards bestehen, während laufende Firewall
und gespeicherte Routen unverändert bleiben. Unklare Herkunft oder fremde
Regeln müssen dagegen vor persistenten Änderungen abgewiesen werden.

Der Generator verweigert ein vorhandenes Zielverzeichnis und schreibt Credentials
privat. Er installiert nichts. JSON-Syntax in `*.yaml` ist gültiges YAML;
`hypernetwork.config.yaml` bleibt alleinige aktive Source of Truth.
`bootstrap.yaml`/`full.yaml` sind vorbereitete Umschaltvorlagen.

Read-only bestätigt: `ubuntu-24.04:v1` ist im Aliasmanifest registriert,
das Raw und der passende Sealmarker (`sealed=true`, `read_only=true`) existieren.
Der Plan pinnt `from: ubuntu-24.04`, `tag: v1`; vorhandene Images werden weder
überschrieben noch neu versiegelt. Voraussetzung ist der F29-Fix zur
unveränderten Wiederverwendung eines gültigen Pins unabhängig von der
Guest-Toolkit-Version.

Abnahmebefund F29 vom 20.10.2026: Die erste Vorprüfung prüfte bei `v2` nur die
Raw-Datei. Das war unzureichend: `image list` und Aliasmanifest registrieren
ausschließlich `v1`. Der Lab-Apply begann deshalb einen Neubake und wurde gezielt
mit SIGTERM beendet, noch vor Lab-VM-Provisionierung. Alle sechs Bestands-VMs
liefen weiter; Alias-/Raw-Metadaten blieben unverändert. Das unvollständige
Lab-Journal wurde mit `apply --abort` geschlossen. Davon getrennt bestand ein
Codefehler: Auch gültige registrierte Pins wurden bei abweichender oder fehlender
Toolkit-Version automatisch neu gebaut. Der F29-Fix beseitigt diesen zweiten
Fehler; der Generator korrigiert den ersten durch den nachgewiesenen Tag `v1`.
Kein Manifest wurde erfunden und kein Image adoptiert.

Der anschließende v1-Bootstrap übersprang Image-Pull/Bake/Seal in 3 ms, startete
die drei Lab-VMs und aktualisierte ihre Guest-Utils. F27 wurde im ops-Gast durch
ausführbares `iwatch 0.3.0` und das persistierte Utils-Manifest bestätigt.
Beim ersten K3s-Node stoppte er mit Exit 24 (F30): Das AWK-Programm zur
Interface-Auswahl hatte ein ungeschlossenes Shell-Zitat. Die RPC-Kette erhält
das argv-Array unverändert; es lag kein zusätzlicher Join im Transport vor.
Der Fix quotiert das vollständige AWK-Programm über den vorhandenen Quote-Helper.
Die Regression führt `/bin/sh` mit einem lokalen `ip`-Fixture aus; die
Reconcile-Tests prüfen außerdem die Syntax aller erzeugten Shellskripte.
Vor der nächsten Fortsetzung muss die korrigierte CLI installiert sein.

Die Fortsetzung nach F30 startete beide K3s-Nodes erfolgreich:
`server` und `worker` meldeten `Ready=True`, Version `v1.37.1+k3s1`.
F31 trat beim anschließenden CRD-Apply auf: Der API-Statuscontroller änderte die
`resourceVersion` zwischen Create und Server-side Apply. Die erste Ingress-CRD
war danach bereits `Established=True`. Der Fix wiederholt ausschließlich diesen
Versionskonflikt, mit höchstens drei Apply-Versuchen insgesamt. Jeder Retry liest
neu, prüft Owner, unveränderte UID und neue Versionsnummer und sendet UID/RV
auch im SSA-Payload. Andere API-Fehler, verschwundene oder ersetzte Ressourcen
brechen ab. `--force-conflicts` bleibt ausgeschlossen.
Die Live-Fortsetzung zeigte außerdem, dass `kubectl get -f - -o json` vorhandene
Objekte als `kind: List` mit genau einem Eintrag zurückgibt. F31 normalisiert
diese GET-Antwort vor der Prüfung; leere GET-Listen bedeuten Abwesenheit,
mehrere oder fehlerhafte Einträge einen Fehler. CREATE muss ein konkretes,
passendes Objekt liefern. Ein fehlendes Objekt während eines Retries wird
niemals erneut angelegt oder übernommen.

Bootstrap ist nach F31 per API abgenommen: beide Nodes `Ready`, vier CRDs
`Established=True`, Namespace `app` aktiv. Der separate F19-Vorschlag
`f19-proposal.yaml` ist validiert; mit `route plan --router controller-lab/ops`
enthält er nur diesen Router und sechs Allow-Regeln für die beiden Lab-CIDRs.
Vor seiner Aktivierung deckte der ungefilterte Read-only-Plan F33 auf:
Er hätte zusätzlich bestehende Neti-Policies entfernt. Der Fix begrenzt
Route-CLI und Stack-Apply auf `spec.project`, und der Supervisor lehnt
ungefiltertes Plan/Apply sowie fremde explizite Router ab. Vor jeder Aktivierung
müssen **CLI und Supervisor** diesen Fix enthalten; `route status` bleibt global
lesbar. Beide Komponenten sind inzwischen aktualisiert und die Lab-Regeln
aktiviert; der verbleibende F34-Konflikt ist oben beschrieben.

## 1. Vorprüfung und Dateien

Alle Host-Kommandos in derselben Bash-Sitzung ausführen. `jq`, `python3`, `curl`
und Docker-CLI werden benötigt. Kube-Kommandos nutzen das K3s-Binary im Gast;
ein Host-`kubectl` ist nicht erforderlich.

```bash
set -euo pipefail
REPO=/Users/frank/Projects/vzctl
VZCTL="$HOME/.local/bin/vzctl"
LAB="$HOME/vzctl-labs/controller-lab"
STATE="${VZCTL_STATE_DIR:-$HOME/Library/Application Support/vzctl}"
command -v jq python3 curl docker
"$VZCTL" version
"$VZCTL" services status --format json
"$VZCTL" doctor --format json
"$VZCTL" vm list --format json
"$VZCTL" net list --format json
"$VZCTL" image list --format json
sysctl hw.memsize
vm_stat
memory_pressure -Q
df -h "$HOME"
netstat -rn -f inet
lsof -nP -iTCP:15556 -sTCP:LISTEN || true
python3 - "$STATE" <<'PY'
import hashlib,json,pathlib,sys
s=pathlib.Path(sys.argv[1])
assert not (s/"controllers"/hashlib.sha256(b"controller-lab").hexdigest()).exists(), "prior lab controller state exists"
images=s/"images"
alias=json.loads((images/"aliases/ubuntu-24.04.json").read_text())
tag=alias["tags"]["v1"]
raw=images/tag["sealed_image"]["path"]
marker=pathlib.Path(tag["sealed_image"]["marker"])
seal=json.loads(marker.read_text())
assert tag["sealed"] is True and raw.is_file(), "registered sealed v1 raw required"
assert seal["sealed"] is True and seal["read_only"] is True, "valid seal marker required"
assert pathlib.Path(seal["source_path"]) == raw, "seal marker must describe this raw"
PY
```

Gate: neue installierte Controller-/Kubernetes-CLI verfügbar, neue Daemons
gesund, kein Projekt `controller-lab`, kein Listener auf `15556`, keine
CIDR-Kollision und ausreichende Hostreserve. `memory_pressure -Q` ist nur eine
Abfrage. Bei vorhandenem Lab oder erhaltenem Controller-State **nicht** blind
darüberinstallieren; stattdessen dessen bewusste Fortsetzung separat planen.

```bash
mkdir -p "$HOME/vzctl-labs"
python3 "$REPO/docs/planing/controller-lab/prepare.py" "$LAB"
"$VZCTL" validate -C "$LAB" --format json >"$LAB/evidence/validate-bootstrap.json"
"$VZCTL" validate -C "$LAB/full.yaml" --format json >"$LAB/evidence/validate-full.json"
"$VZCTL" plan -C "$LAB" --format json >"$LAB/evidence/plan-bootstrap.json"
```

Plan vor Apply prüfen: genau die drei Lab-VMs und ihre beiden Netze, gepinntes
K3s, keine Mutationen an Bestandsprojekten. RAM-Budget-Warnung bewertet nur
Gesamtkapazität, nicht aktuelle freie Hostkapazität.

## 2. Bootstrap, Provider und vollständiger Apply

```bash
"$VZCTL" apply -C "$LAB" --progress plain
"$VZCTL" kubernetes status -C "$LAB" --format json >"$LAB/evidence/kubernetes-bootstrap.json"
```

Erwartet: Server und Agent `Ready`, exakte Kubelet-Version `v1.37.1+k3s1`,
Namespace `app`, vier etablierte `services.vzctl.dev`-CRDs. Ohne verfügbare
DNS-Bind-/Resolver-Helfer vor weiterer Provisionierung anhalten und die
Hostvoraussetzung durch den Betreiber herstellen.

Provider bewusst **vor** Claims installieren: Controller provisionieren Datenbank,
Bucket und Zugänge; sie installieren keine PostgreSQL-/MinIO-Server.
Der Provider-Build läuft als begrenzter systemd-Job im neuen Gast, damit ein
CLI-Timeout keinen unbeaufsichtigten Shellprozess zurücklässt.
Das private Staging liegt unter `/run`: Der Guest-Agent hat `PrivateTmp=yes`,
deshalb sieht eine separate systemd-Unit seine `/tmp`-Dateien nicht.
Vorhandenes Staging oder ein früherer Versuch verlangt eine separat geprüfte
Recovery; die folgenden Befehle sind kein automatischer Wiederholungsablauf.

```bash
OPS=controller-lab/ops
SERVER=controller-lab/server
"$VZCTL" vm exec "$OPS" -- sudo -n install -d -o vzctl-agent -g vzctl-agent -m 700 /run/vzctl-lab-provider-stage
"$VZCTL" vm transfer "$OPS" "$LAB/secrets/admin.json" "$OPS:/run/vzctl-lab-provider-stage/admin.json"
"$VZCTL" vm exec "$OPS" -- chmod 600 /run/vzctl-lab-provider-stage/admin.json
"$VZCTL" vm transfer "$OPS" "$LAB/assets/provider-setup.sh" "$OPS:/run/vzctl-lab-provider-stage/provider-setup.sh"
"$VZCTL" vm transfer "$OPS" "$LAB/assets/requirements.txt" "$OPS:/run/vzctl-lab-provider-stage/requirements.txt"
"$VZCTL" vm exec "$OPS" -- sudo -n systemd-run --no-block --unit=vzctl-lab-setup \
  --property=Type=oneshot --property=RemainAfterExit=yes \
  --property=TimeoutStartSec=1800 --property=RuntimeMaxSec=1900 \
  --property=TimeoutStopSec=30 --property=KillMode=control-group \
  '--property=ExecStopPost=/bin/rm -f -- /run/vzctl-lab-provider-stage/admin.json' \
  --property=StandardOutput=null --property=StandardError=null \
  /bin/bash /run/vzctl-lab-provider-stage/provider-setup.sh
"$VZCTL" vm services "$OPS" status vzctl-lab-setup.service
```

`vm services … status` wiederholt lesen, bis `ActiveState=active`,
`SubState=exited`, `Result=success`, `ExecMainStatus=0`. Maximal 32 Minuten;
bei `failed` stoppen. Verbindlicher maschinenlesbarer Abschluss:

```bash
"$VZCTL" vm exec "$OPS" -- sudo -n systemctl show vzctl-lab-setup.service \
  --property=ActiveState,SubState,Result,ExecMainStatus
"$VZCTL" vm exec "$OPS" -- sudo -n systemctl is-active postgresql vzctl-lab-minio.service
"$VZCTL" vm exec "$OPS" -- sudo -n psql --version
"$VZCTL" vm exec "$OPS" -- /usr/local/bin/mc --version
```

Das Script installiert Ubuntu-Pakete aus den konfigurierten Ubuntu-24.04-Repositories,
baut MinIO/mc aus festen offiziellen Commits und erzeugt nur den separaten Bucket
`controller-lab-forbidden` für die Negativprobe. Administratorwerte erscheinen
nicht in Argumenten, Logs oder Status. Nach Erfolg wird die temporäre
Credential-Datei gelöscht. Exakte Paketversionen für die Abnahme erfassen:

```bash
"$VZCTL" vm exec "$OPS" -- dpkg-query -W postgresql postgresql-client python3 \
  >"$LAB/evidence/provider-packages.txt"
# Nur noch nicht referenzierte Consumer-Dateien vorbereiten; aktive YAML erhalten.
python3 - "$LAB" "$STATE/ca/root/ca.crt" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1])
pem=pathlib.Path(sys.argv[2]).read_text()
assert "-----BEGIN CERTIFICATE-----" in pem and "PRIVATE KEY" not in pem
assert (root/"hypernetwork.config.yaml").read_bytes()==(root/"bootstrap.yaml").read_bytes()
(root/"assets/ca.crt").write_text(pem)
(root/"assets/ca.crt").chmod(0o644)
path=root/"kubernetes/code.yaml"
code=json.loads(path.read_text())
code["data"]["ca.crt"]=pem
path.write_text(json.dumps(code,indent=2)+"\n")
path.chmod(0o600)
PY
"$VZCTL" vm transfer "$OPS" "$LAB/assets/probe.py" "$OPS:/tmp/vzctl-lab-probe.py"
"$VZCTL" vm transfer "$OPS" "$LAB/assets/ca.crt" "$OPS:/tmp/vzctl-lab-ca.crt"
"$VZCTL" vm exec "$OPS" -- sudo -n install -m 644 /tmp/vzctl-lab-probe.py /opt/vzctl-lab/probe.py
"$VZCTL" vm exec "$OPS" -- sudo -n install -m 644 /tmp/vzctl-lab-ca.crt /opt/vzctl-lab/ca.crt
"$VZCTL" validate -C "$LAB/full.yaml" --format json >"$LAB/evidence/validate-full.json"
"$VZCTL" plan -C "$LAB/full.yaml" --format json >"$LAB/evidence/plan-full.json"
```

Plan vor Aktivierung auf die vorgesehenen Lab-Ressourcen prüfen. Der registrierte
Controller liest die aktive YAML periodisch; bereits ihr Austausch kann Ressourcen
provisionieren. Erst nach erfolgreicher Prüfung:

```bash
python3 "$REPO/docs/planing/controller-lab/prepare.py" "$LAB" \
  --activate "$STATE/ca/root/ca.crt"
"$VZCTL" apply -C "$LAB" --progress plain
```

Erwartet: fünf YAML-Claims `Ready`; VM-Bindings, drei schreibgeschützte
Docker-Binding-Verzeichnisse und drei Namespace-Secrets; Docker-Container und
Kubernetes-Deployment laufen als UID 10001. Der erste Python-Paketdownload kann
einige Minuten dauern. Kein `--force-conflicts`, keine implizite Datenlöschung.
Bei temporärem Apply-Fehler Ursache beheben und **denselben** Lauf mit
`"$VZCTL" apply -C "$LAB" --resume --progress plain` fortsetzen.

## 3. Echte Consumer-Probes

```bash
kctl() {
  "$VZCTL" vm exec "$SERVER" --timeout-ms 120000 -- sudo -n /usr/local/bin/k3s \
    kubectl --request-timeout=90s "$@"
}
kctl get nodes -o wide
kctl get crd ingressclaims.services.vzctl.dev oidcclaims.services.vzctl.dev \
  postgresclaims.services.vzctl.dev s3claims.services.vzctl.dev
kctl -n app rollout status deployment/proof --timeout=80s
"$VZCTL" vm exec "$OPS" --timeout-ms 120000 -- sudo -n \
  /opt/vzctl-lab/venv/bin/python /opt/vzctl-lab/probe.py \
  >"$LAB/evidence/probe-vm.json"
curl --fail --silent --show-error --max-time 120 --cacert "$LAB/assets/ca.crt" \
  https://docker.svc.controller-lab.vz.test/ >"$LAB/evidence/probe-docker.json"
curl --fail --silent --show-error --max-time 120 --cacert "$LAB/assets/ca.crt" \
  https://kube.svc.controller-lab.vz.test/ >"$LAB/evidence/probe-kubernetes.json"
for file in "$LAB"/evidence/probe-*.json; do
  jq -e '.postgres == true and .s3 == true and .oidc == true' "$file"
done
```

Die Probe prüft **in jeder Consumer-Umgebung**:

- PostgreSQL: Login mit eigenem Claim-User, Tabelle anlegen, Schreiben/Lesen;
  kein Superuser-/CreateDB-/CreateRole-Recht.
- S3: Objekt schreiben/lesen; Zugriff auf existierenden fremden Bucket verweigert.
- OIDC: TLS-validierte Discovery, Authorization Code + S256-PKCE, Dev-User-Auswahl,
  Tokenaustausch mit Claim-Secret, RS256-JWKS-Signatur, Issuer/Audience/Nonce,
  authentisiertes Userinfo. Das Dev-IdP hat absichtlich keine Passwortanmeldung.
- Host→Docker und Host→K3s laufen zusätzlich über echte HTTPS-Routen mit
  CA-Prüfung; **kein** `curl -k`. App-Fehler geben nur Fehlerklassennamen zurück.

```bash
"$VZCTL" controller status -C "$LAB" --format json >"$LAB/evidence/controller-ready.json"
"$VZCTL" kubernetes status -C "$LAB" --format json >"$LAB/evidence/kubernetes-ready.json"
"$VZCTL" kubernetes kubeconfig -C "$LAB" --cluster dev \
  --output "$LAB/kubeconfig.yaml" --format json >"$LAB/evidence/kubeconfig-export.json"
test "$(stat -f '%Lp' "$LAB/kubeconfig.yaml")" = 600
"$VZCTL" vm exec "$OPS" -- sudo -n docker inspect proof \
  --format '{{.Id}} {{.Config.User}} {{json .Mounts}}' >"$LAB/evidence/docker-before.txt"
```

Gate: CLI-Status `status=ok`, `exit_code=0`; alle fünf Claims `Ready`,
keine Warnungen; Cluster `Ready`. Mounts enthalten ausschließlich zugewiesene
Claim-Verzeichnisse als `RW=false`; Docker-User `10001:10001`. Kubeconfig nie
ausgeben; vorhandene Exportdateien werden nicht überschrieben.

## 4. Wiederholung und Credential-Stabilität

Nur Hashes privater Binding-Dateien erfassen, niemals deren Inhalte:

```bash
binding_hashes() {
  python3 - "$STATE" <<'PY'
import hashlib,json,pathlib,sys
p=pathlib.Path(sys.argv[1])/"controllers"/hashlib.sha256(b"controller-lab").hexdigest()/"bindings"
print(json.dumps({f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(p.glob("*.json"))},sort_keys=True))
PY
}
runtime_identity() {
  kctl get nodes -o json | jq -S '[.items[] | {name:.metadata.name,uid:.metadata.uid}] | sort_by(.name)'
  kctl -n app get deployment/proof -o json | jq -S '{uid:.metadata.uid}'
  kctl -n app get pods -l app=proof -o json | jq -S '[.items[] | {uid:.metadata.uid}] | sort_by(.uid)'
  "$VZCTL" vm exec "$SERVER" -- sudo -n systemctl show vzctl-k3s.service --property=ExecMainStartTimestamp
  "$VZCTL" vm exec controller-lab/worker -- sudo -n systemctl show vzctl-k3s.service --property=ExecMainStartTimestamp
}
binding_hashes >"$LAB/evidence/bindings-before.json"
runtime_identity >"$LAB/evidence/runtime-before.txt"
"$VZCTL" apply -C "$LAB" --progress plain
"$VZCTL" controller reconcile -C "$LAB" --format json >"$LAB/evidence/reconcile-repeat.json"
binding_hashes >"$LAB/evidence/bindings-after.json"
cmp "$LAB/evidence/bindings-before.json" "$LAB/evidence/bindings-after.json"
"$VZCTL" vm exec "$OPS" -- sudo -n docker inspect proof \
  --format '{{.Id}} {{.Config.User}} {{json .Mounts}}' >"$LAB/evidence/docker-after.txt"
cmp "$LAB/evidence/docker-before.txt" "$LAB/evidence/docker-after.txt"
runtime_identity >"$LAB/evidence/runtime-after.txt"
cmp "$LAB/evidence/runtime-before.txt" "$LAB/evidence/runtime-after.txt"
```

Gate: kein Secretwechsel, kein Docker-Recreate; K3s-Node-UIDs, Deployment-/Pod-UIDs
und K3s-systemd-`ExecMainStartTimestamp` bleiben vor/nach Wiederholung gleich.
DB-Tabelle/Objekte bleiben erhalten; Abschnitt 3 erneut ausführen. Das Host-State
enthält Credential-Dateien: Hashlisten sind Lab-Evidence und bleiben ebenfalls privat.

## 5. K7: neuer Kubernetes-Bedarf ohne weiteres Apply

Die vier späten Claims sind **nicht** in `manifests` referenziert. Sie werden
absichtlich nach erfolgreichem Apply über die Kubernetes-API angelegt.

```bash
"$VZCTL" vm transfer "$SERVER" "$LAB/late-claims.yaml" "$SERVER:/tmp/vzctl-lab-late-claims.yaml"
kctl apply -f /tmp/vzctl-lab-late-claims.yaml
# Nur lesen: kein controller reconcile/watch und kein zweites vzctl apply.
for attempt in $(seq 1 24); do
  "$VZCTL" controller status -C "$LAB" --format json >"$LAB/evidence/controller-late.json" || test "$?" = 24
  if jq -e '(.data.claims | length) == 9 and all(.data.claims[]; .state == "Ready")' \
       "$LAB/evidence/controller-late.json" >/dev/null; then break; fi
  sleep 5
done
jq -e '(.data.claims | length) == 9 and all(.data.claims[]; .state == "Ready")' \
  "$LAB/evidence/controller-late.json"
kctl -n app get postgresclaims,s3claims,oidcclaims,ingressclaims -o json \
  >"$LAB/evidence/kubernetes-claim-status.json"
curl --fail --silent --show-error --max-time 120 --cacert "$LAB/assets/ca.crt" \
  https://late.svc.controller-lab.vz.test/ >"$LAB/evidence/probe-late.json"
```

Gate: innerhalb 120 s vier zusätzliche Claims `Ready`, Status-`observedGeneration`
entspricht der Resource-Generation, Secretreferenzen liegen in Namespace `app`.
`data.supervisor.enabled=true`, `generation` nicht leer, `lastAttemptAt` aktuell.
Die periodische CP-Ausführung arbeitet mit 30-s-Takt und spätestens 300-s-Deadline;
ein langsamer Lauf wird als Zeitüberschreitung dokumentiert, nicht als Erfolg.
Für diesen kleinen lokalen Test sind 120 s die Abnahmegrenze.

In einer zweiten Sitzung:

```bash
"$VZCTL" events subscribe --filter 'controller.*,apply.*'
```

Erwartet: `controller.status.changed` bei semantischer Änderung, kein neues
Status-Event allein wegen Zeitstempelwechsel. Im verbundenen MCP-Client konkret
`controller_status` mit `{"stack_path":"…/controller-lab"}` und
`kubernetes_status` mit `{"stack_path":"…/controller-lab","cluster":"dev"}` ausführen;
`stack_path` ist dabei der tatsächliche absolute Wert von `$LAB`.
`tools/list` muss beide Tools als read-only anbieten. Antworten enthalten
Status/Conditions, keine Binding-Credentials, Secretobjekte oder Kubeconfig.

## 6. Ownership, Ausfall und Wiederanlauf

### Fremde Datenbank wird nicht übernommen

```bash
"$VZCTL" vm exec "$OPS" -- sudo -n -u postgres createdb controller_foreign
python3 - "$LAB" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1])/"foreign-claim.yaml"
p.write_text(json.dumps({"apiVersion":"services.vzctl.dev/v1alpha1","kind":"PostgresClaim",
 "metadata":{"name":"foreign-db","namespace":"app"},
 "spec":{"provider":"database","database":"controller_foreign","bindingSecret":"foreign-db"}}))
PY
"$VZCTL" vm transfer "$SERVER" "$LAB/foreign-claim.yaml" "$SERVER:/tmp/vzctl-lab-foreign.yaml"
kctl apply -f /tmp/vzctl-lab-foreign.yaml
```

Innerhalb 120 s über `kctl -n app get postgresclaim foreign-db -o json` beobachten:
Condition `OwnershipConflict`, kein `Ready`, kein Binding-Secret.
Andere neun Claims bleiben funktionsfähig. Datenbankbesitzer muss `postgres` bleiben:

```bash
"$VZCTL" vm exec "$OPS" -- sudo -n -u postgres psql -X -At -c \
  "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname='controller_foreign'"
test -z "$(kctl -n app get secret foreign-db --ignore-not-found)"
kctl -n app delete postgresclaim foreign-db
```

`controller_foreign` absichtlich bis zum Purge behalten; weder Annotationen
fälschen noch einen fremden Besitzer „reparieren“.

### Fremdes Binding wird nicht überschrieben

```bash
kctl -n app annotate secret db services.vzctl.dev/owner=foreign-owner --overwrite
"$VZCTL" controller reconcile -C "$LAB" --format json >"$LAB/evidence/ownership-secret.json" || test "$?" = 24
jq -e 'any(.data.claims[]; .name == "db" and any(.conditions[]; .reason == "OwnershipConflict"))' \
  "$LAB/evidence/ownership-secret.json"
kctl -n app get secret db -o 'jsonpath={.metadata.annotations.services\.vzctl\.dev/owner}'
kctl -n app annotate secret db services.vzctl.dev/owner=vzctl:controller-lab/db --overwrite
"$VZCTL" controller reconcile -C "$LAB" --format json
```

Gate: während Konflikt bleibt `foreign-owner` unverändert; bestehende Daten/Secrets
werden nicht übernommen oder rotiert. Hier wird ausschließlich das eigene
Lab-Secret kontrolliert markiert und danach auf seine zuvor bekannte Identität
zurückgesetzt.

### Provider-Ausfall und Recovery

```bash
"$VZCTL" vm services "$OPS" stop vzctl-lab-minio.service
"$VZCTL" controller reconcile -C "$LAB" --format json >"$LAB/evidence/provider-down.json" || test "$?" = 24
jq -e 'any(.data.claims[]; .name == "files" and .state == "Degraded")' "$LAB/evidence/provider-down.json"
"$VZCTL" vm services "$OPS" start vzctl-lab-minio.service
"$VZCTL" controller reconcile -C "$LAB" --format json >"$LAB/evidence/provider-recovered.json"
```

Erwartet: Provider-Code statt Secrets/roher mc-Ausgabe, CLI Exit 24 beim Ausfall;
unabhängige PostgreSQL-/OIDC-Claims bleiben bereit. Danach Abschnitt 3:
alle realen Probes wieder grün, DB-Daten/Objekte/Credentials unverändert.
Zusätzlich K3s-Serverdienst stoppen/starten:

```bash
"$VZCTL" vm services "$SERVER" stop vzctl-k3s.service
"$VZCTL" controller reconcile -C "$LAB" --format json >"$LAB/evidence/kubernetes-down.json" || test "$?" = 24
"$VZCTL" kubernetes status -C "$LAB" --format json >"$LAB/evidence/kubernetes-down-status.json" || test "$?" = 24
"$VZCTL" vm services "$SERVER" start vzctl-k3s.service
```

Gate: Discovery-Warnung/`Degraded`, kein stilles `Ready`; bestehende Ingress-
Veröffentlichung wird bei unvollständiger Discovery nicht entfernt. Ein fehlendes
Backend darf trotzdem HTTP 502/503 liefern — das ist kein Löschungsbeweis.
Nach API-Readiness `controller reconcile` und Abschnitt 3 wiederholen.

### Claim-Entfernung: Daten behalten, Veröffentlichungen zurücknehmen

Nach erfolgreichen Consumer-Probes, vor Down/Apply: die fünf YAML-Claims gezielt
entfernen. Die vier späten Kubernetes-Claims bleiben unverändert. Vorher private
Binding-Hashes und die aktuelle Config sichern. Nicht den gesamten Stack stoppen.

```bash
binding_hashes >"$LAB/evidence/bindings-before-removal.json"
python3 - "$LAB" <<'PY'
import json,os,pathlib,sys
root=pathlib.Path(sys.argv[1])
active=root/"hypernetwork.config.yaml"
config=json.loads(active.read_text())
assert config["spec"]["project"]=="controller-lab"
assert set(config["spec"]["serviceClaims"])=={"db","files","login","docker-web","kube-web"}
backup=root/"evidence/config-before-claim-removal.json"
with backup.open("x") as f:
    os.chmod(backup,0o600)
    f.write(active.read_text())
config["spec"]["serviceClaims"]={}
staged=root/"removal.yaml"
assert not staged.exists()
staged.write_text(json.dumps(config,indent=2)+"\n")
staged.chmod(0o600)
PY
"$VZCTL" validate -C "$LAB/removal.yaml" --format json >"$LAB/evidence/validate-removal.json"
"$VZCTL" plan -C "$LAB/removal.yaml" --format json >"$LAB/evidence/plan-removal.json"
```

Vor Aktivierung den Plan auf die fünf entfernten Claims und abgeleiteten Bindings
prüfen. Ein erforderliches Container-Recreate darf ausschließlich den eigenen
Lab-Container `ops/proof` betreffen. Andere destruktive Änderungen brechen diese
Abnahme ab. Die aktive YAML bleibt bis dahin unverändert. Danach atomisch aktivieren:

```bash
python3 - "$LAB" <<'PY'
import os,pathlib,sys
root=pathlib.Path(sys.argv[1])
active=root/"hypernetwork.config.yaml"
assert active.read_bytes()==(root/"evidence/config-before-claim-removal.json").read_bytes()
os.replace(root/"removal.yaml",active)
PY
# --force ist hier ausschließlich für das zuvor geprüfte proof-Recreate vorgesehen.
"$VZCTL" apply -C "$LAB" --force --progress plain
"$VZCTL" controller status -C "$LAB" --format json >"$LAB/evidence/controller-removed.json"
binding_hashes >"$LAB/evidence/bindings-after-removal.json"
cmp "$LAB/evidence/bindings-before-removal.json" "$LAB/evidence/bindings-after-removal.json"
```

Verbindliche Live-Nachweise:

- Die fünf entfernten YAML-Claims haben `Retained`/`ClaimRemoved`, die vier
  Kubernetes-Claims bleiben `Ready`. Keine neue Credential-Datei.
- Mit dem erhaltenen VM-Binding **nur lesen**: SQL-Zeile
  `lab_proof.id='proof'` ist weiter `controller-lab`; S3-Objekt
  `lab-proof/vm` enthält weiterhin `controller-lab`. Hier nicht die normale
  Probe verwenden: Sie legt die Testdaten bei Bedarf neu an.
- Die aktive Edge-Konfiguration enthält die Hosts `docker.svc.controller-lab.vz.test`
  und `kube.svc.controller-lab.vz.test` nicht mehr. HTTPS darf keine erfolgreiche
  Consumer-Antwort liefern. Ein TLS-/DNS-Fehler allein ersetzt den Konfigurationsbeleg
  nicht; Status-/Konfigurationsauszug auf Hostnamen begrenzen.
- Eine zuvor erfolgreiche OIDC-Authorization-Anfrage für `client_id=controller-lab`
  und dessen gültige Redirect-URI liefert jetzt HTTP 400 `unknown client_id`.
  Discovery und der unabhängige späte OIDC-Client funktionieren weiter.
- Docker darf bei expliziter Entfernung seiner Binding-Mounts neu erstellt werden;
  das ist vom unveränderten Wiederholungs-Apply in Abschnitt 4 getrennt.

Anschließend die gesicherte Config zunächst als Kandidat neben der aktiven YAML
bereitstellen (Mode 0600), validieren und den Plan prüfen. Erst dann atomisch
aktivieren und Apply ausführen. Alle neun Claims werden wieder bereit.
Binding-Hashes bleiben identisch; SQL-Zeile und S3-Objekt erneut **vor** den
schreibenden Consumer-Probes lesen. Danach Abschnitt 3 vollständig wiederholen.
Bei Fehlern keinen Claim-Purge oder manuellen Credential-Neubau ausführen.

### CP-Neustart und Down-Grenze

Der folgende Supervisor-Neustart betrifft den ganzen Host. **Nur in einem mit
dem Betreiber abgestimmten Wartungsfenster, ohne laufende Jobs anderer Stacks**:

```bash
"$VZCTL" services restart supervisor --format json
"$VZCTL" controller status -C "$LAB" --format json
```

Gate nach spätestens 120 s: Registry weiterhin aktiviert mit gleicher Generation,
neuer erfolgreicher `lastAttemptAt`, neun aktive Claims bereit, kein manuelles
Reconcile nötig. Ein echter Host-Reboot ist ein separater geplanter
Wartungstest; dafür wird kein automatischer Reboot-Befehl bereitgestellt.

Danach lab-lokal:

```bash
"$VZCTL" down -C "$LAB" --progress plain
"$VZCTL" controller status -C "$LAB" --format json >"$LAB/evidence/controller-down.json" || test "$?" = 24
jq -e '.data.supervisor.enabled == false' "$LAB/evidence/controller-down.json"
sleep 35
"$VZCTL" controller status -C "$LAB" --format json >"$LAB/evidence/controller-down-later.json" || test "$?" = 24
jq -e '.data.supervisor.enabled == false' "$LAB/evidence/controller-down-later.json"
"$VZCTL" apply -C "$LAB" --progress plain
```

Gate: Down deaktiviert Registry vor VM-Mutationen; kein periodischer Wiederstart,
keine neuen Provider-/Edge-Mutationen nach abgeschlossenem Down. Eine bereits
gestartete RPC ist nicht rückrollbar; `down` muss ggf. begrenzt abbrechen, statt
Cancellation fälschlich zu bestätigen. Erneutes explizites Apply registriert eine
neue Generation. Daten und Credentials über normales Down/Apply erhalten.

## 7. Cleanup und Abschlussbeleg

Vor Purge die erwarteten drei Lab-VMs und Lab-Netze nochmals im Plan/Inventar
identifizieren. Purge ist hier ausdrücklich datenvernichtend: auch MinIO,
PostgreSQL, die Testobjekte und K3s in diesen drei Gästen werden gelöscht.

```bash
python3 - "$LAB/hypernetwork.config.yaml" <<'PY'
import json,sys
c=json.load(open(sys.argv[1]))
assert c["spec"]["project"]=="controller-lab"
assert set(c["spec"]["vms"])=={"ops","server","worker"}
assert set(c["spec"]["networks"])=={"lan","containers"}
PY
"$VZCTL" down -C "$LAB" --purge --progress plain
"$VZCTL" vm list --format json >"$LAB/evidence/vms-after-purge.json"
"$VZCTL" net list --format json >"$LAB/evidence/networks-after-purge.json"
"$VZCTL" controller status -C "$LAB" --format json >"$LAB/evidence/controller-after-purge.json" || test "$?" = 24
```

Gate: keine `controller-lab/*`-VMs, keine Lab-Netz-Referenzen oder Hostroutes,
Registry deaktiviert. Alle sechs vorherigen VMs/Netze unverändert. Gemeinsame
sealed Images, Host-CA, DNS-Helfer und Resolver **nicht** löschen. Private
Controller-State-/Credential-Dateien werden nach Contract möglicherweise
behalten; dies ist kein Beleg für eine laufende Registrierung. Kein pauschales
`rm -rf "$STATE"` und kein globales `docker prune`.

Das private `$LAB` mit Evidenz, Admin-Dateien und Kubeconfig bis zur Auswertung
behalten. Danach nur dieses genaue Lab-Verzeichnis gezielt löschen; vorher
keine Secretdateien in Issues/Canvas kopieren. Dokumentierter Abschluss pro Gate:
`bestanden | fehlgeschlagen | nicht ausgeführt`, Zeitpunkt, CLI-/Daemon-Version,
Evidence-Datei, Fehlercode und ggf. verknüpftes Finding. Ein übersprungener
CP-Neustart bleibt ausdrücklich offen.

## Geprüfte Quellen und statische Validierung

Am 20.10.2026 geprüft:

- K3s [v1.37.1+k3s1](https://github.com/k3s-io/k3s/releases/tag/v1.37.1%2Bk3s1),
  [offizielle ARM64-SHA-Datei](https://github.com/k3s-io/k3s/releases/download/v1.37.1%2Bk3s1/sha256sum-arm64.txt):
  `a1561ca4aef8b99f5588a840d4468ac39c8c1cd92470bbcb9634823fa221741a`.
- MinIO [RELEASE.2025-04-22T22-12-26Z](https://github.com/minio/minio/releases/tag/RELEASE.2025-04-22T22-12-26Z):
  [Commit `0d7408fc9969caf07de6a8c3a84f9fbb10a6739e`](https://github.com/minio/minio/commit/0d7408fc9969caf07de6a8c3a84f9fbb10a6739e)
  vom **22.04.2025, 22:12:26 UTC**; Release veröffentlicht
  **22.04.2025, 22:44:34 UTC**.
  mc [RELEASE.2025-04-16T18-13-26Z](https://github.com/minio/mc/releases/tag/RELEASE.2025-04-16T18-13-26Z):
  [Commit `b00526b153a31b36767991a4f5ce2cced435ee8e`](https://github.com/minio/mc/commit/b00526b153a31b36767991a4f5ce2cced435ee8e)
  vom **16.04.2025, 18:13:26 UTC**; Release veröffentlicht
  **22.04.2025, 22:11:04 UTC**. Tagname und Veröffentlichungsdatum sind bei mc
  ausdrücklich verschieden; beide Daten wurden über die offizielle GitHub-API
  (`releases/tags`, annotierter `git/ref/tags`, `git/commits`) geprüft.
  Das sind feste Lab-Kompatibilitätsstände, keine Aussage über aktuelle
  Produktionsfreigaben. Frühere Binary-Archivlinks antworteten HTTP 410;
  deshalb Quellbuild mit vorhandenem `go.sum`, `-mod=readonly` und `GOTOOLCHAIN=local`.
- [Go-Downloadmetadaten](https://go.dev/dl/?mode=json&include=all):
  `go1.24.2.linux-arm64.tar.gz`,
  SHA256 `756274ea4b68fa5535eb9fe2559889287d725a8da63c6aae4d5f23778c229f4b`.
  Die offiziellen MinIO-Quellen verlangen Go 1.24, mc Go 1.23; Go 1.24.2 erfüllt beide.
- [Offizielles Python-Image](https://hub.docker.com/_/python):
  `3.13-slim-bookworm`, geprüfter Multiarch-Digest
  `sha256:a1165e272e578941b84abc79e4ab38a0305cd12803a5c4247979ac7655f4d641`
  enthält `linux/arm64/v8`. Der Generator verwendet den Digest.
- Testclient-Versionen über PyPI bestätigt:
  [pg8000 1.31.5](https://pypi.org/project/pg8000/1.31.5/),
  [boto3 1.43.115](https://pypi.org/project/boto3/1.43.115/),
  [PyJWT 2.15.1](https://pypi.org/project/PyJWT/2.15.1/),
  [cryptography 50.0.2](https://pypi.org/project/cryptography/50.0.2/).
  Direkte und transitive Python-Pakete stehen vollständig mit Versionen und
  Wheel-SHA256 in [requirements.txt](controller-lab/requirements.txt).
  VM, Docker und K3s verwenden dieselbe Datei mit `--require-hashes`.
  Ubuntu-Pakete bleiben durch die konfigurierten Paketquellen bestimmt; ihre
  installierten Versionen gehören zur Live-Evidenz.

### Gezielte Advisory-Prüfung der Lab-Python-Abhängigkeiten

Am 20.10.2026 mit isoliertem pip 26.2.1 im **Dry-run**, ohne Installation von
Consumer-Paketen oder Start einer VM: Auflösung für CPython **3.12 und 3.13,
Linux ARM64**, ausschließlich Wheels. Anschließend alle **14 aufgelösten Pakete**
mit exakter Version gegen **OSV `querybatch` und PyPI/OSV-Advisory-Metadaten**
geprüft. Ergebnis für die neuen Pins: **0 bekannte Advisories** in beiden Quellen.
Die Hashdatei wurde zusätzlich in beiden Zielauflösungen geprüft.

Die früheren Pins wurden gezielt ersetzt:

| Paket | Alter Pin | Neuer Pin | Befund |
|---|---|---|---|
| pg8000 | 1.31.2 | 1.31.5 | u. a. [GHSA-wq2g-r956-j8cc](https://github.com/advisories/GHSA-wq2g-r956-j8cc), korrigiert ab 1.31.5 |
| PyJWT | 2.10.1 | 2.15.1 | mehrere Advisory-Einträge, u. a. [GHSA-752w-5fwx-jx9f](https://github.com/advisories/GHSA-752w-5fwx-jx9f) und [GHSA-jq35-7prp-9v3f](https://github.com/advisories/GHSA-jq35-7prp-9v3f) |
| cryptography | 44.0.2 | 50.0.2 | mehrere Advisory-Einträge, u. a. [GHSA-r6ph-v2qm-q3c2](https://github.com/advisories/GHSA-r6ph-v2qm-q3c2) |
| boto3 | 1.37.38 | 1.43.115 | kein eigener Advisory-Treffer; aktualisiert und kompletter transitiver Baum geprüft |

Maschinenlesbarer Beleg mit alten IDs/Fixversionen, neuem Inventar und leerer
OSV-Trefferliste: [dependency-audit.json](controller-lab/dependency-audit.json).
Die Anzahl der Metadateneinträge ist wegen GHSA/PYSEC-Duplikaten **keine Anzahl
eindeutiger Schwachstellen**. Dies ist eine Versionsprüfung bekannter Meldungen,
kein Nachweis einer konkreten Ausnutzbarkeit der alten Pins in der Probe und
keine Garantie gegen unbekannte Fehler. MinIO, Go und Ubuntu-Pakete waren nicht
Gegenstand dieser eng begrenzten Python-Prüfung.

Die Befehle wurden mit aktueller `vzctl help`-Ausgabe und Code abgeglichen.
Beide generierten Configvarianten bestehen `vzctl validate` mit Exit 0,
einschließlich lokaler Manifestreferenzen; Python-Syntax und Shell-Syntax
wurden geprüft. MinIO-Build, Paketinstallation, Guest-Boot, TLS-/Netz-/Provider-
Probes und Live-Recovery sind **noch nicht ausgeführt**.
