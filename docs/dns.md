# vz-edge DNS v1

Issue [#26](https://github.com/frankhildebrandt/vzctl/issues/26) implementiert
den `vz-edge`-owned DNS aus ADR 0002. Der Server spricht UDP und hält parallel
den Host-Listener sowie je aktivem vmnet einen Guest-Listener:

| Listener | Default | Zweck |
|---|---|---|
| Host | `127.0.0.1:15353` | macOS `/etc/resolver` |
| Guest | Bridge-`.0:53` | Standard-DNS der Guests |

Port 53 (UDP) und Ingress-Ports 80/443 (TCP) sind privilegiert. Der
unprivilegierte `vz-edge`-LaunchAgent nutzt den Root-LaunchDaemon
`vz-dns-bind` (SCM_RIGHTS):

- **UDP** (`:53`): PF leitet Guest-Pakete auf Bridge-`.0` transparent zum
  exklusiven `vz-edge`-Backend `:15054` um. Dadurch kann macOS
  `mDNSResponder` mit seinem Wildcard-Listener nicht den Host-Horizont liefern.
- **TCP** (`:80`/`:443`): Helper legt bei Bedarf Host-Service-Alias `.1` auf dem
  Bridge an, bindet+listens und streamt akzeptierte Client-FDs über dieselbe
  UDS-Verbindung. Ingress-`*.svc` ist Split-Horizon:
  Host-Listener (`127.0.0.1:15353`) → `127.0.0.1` (Mac kann Bridge-`.1` nicht
  dialen: `EHOSTDOWN`); Guest-Listener (`.0:53`) → Host-Service-`.1` (vmnet
  verwirft Guest-TCP zu `.0`; der PF-Redirect verhindert, dass mDNSResponder
  stattdessen die Host-Loopback-Antwort liefert).

`vz-edge` ordnet jedem Guest-Listener fest sein vmnet und Projekt zu. Eine
Ingress-Query an `10.90.0.0:53` liefert deshalb ausschließlich `10.90.0.1`;
ein Ingress-Name eines anderen Projekts liefert `NXDOMAIN`. Jedes aktive
vmnet erhält den `.1`-Alias unabhängig von Attachments. Ein verwalteter
PF-Anchor erlaubt dort nur die konfigurierten Ingress-TCP-Ports und blockiert
sonstige Host-Dienste sowie UDP/ICMP. Logische `backend: docker`-Netze besitzen
keinen Alias; Container verwenden `.1` der primären vmnet-NIC ihrer Docker-VM.

Der Helper akzeptiert dafür idempotente, kanonische Netzwerkoperationen:

```json
{"op":"alias.ensure","cidr":"10.90.0.0/24"}
{"op":"alias.remove","cidr":"10.90.0.0/24"}
{"op":"firewall.reconcile.v2","bindings":[{"cidr":"10.90.0.0/24","allowed_sources":["10.90.0.0/24"],"tcp_ports":[80,443],"dns_port":53,"dns_backend_port":15054}]}
```

Aliases und PF-Token liegen geschützt unter `/var/run/vzctl/`; `/etc/pf.conf`
bleibt unverändert. Der Helper ist für aktive vmnet-Netze erforderlich, weil
`.1` ohne erfolgreich geladenen Schutz nicht veröffentlicht wird.

```sh
sudo vzctl dns install-bind-helper
sudo vzctl dns uninstall-bind-helper
```

Ohne Helper schlägt die geschützte PF-/Alias-Generation vor Veröffentlichung
fehl (`dns_ok=false`); Guest-DNS und Guest-Ingress bleiben beim Last-Known-Good.
Für unprivilegierte Dev-Läufe ohne Helper:

```sh
VZCTL_DNS_GUEST_PORT=15353
```

Der Backend-Port ist mit `VZCTL_DNS_GUEST_BACKEND_PORT` überschreibbar. Bei
einem unprivilegierten `VZCTL_DNS_GUEST_PORT` sind Public- und Backend-Port
standardmäßig identisch und es entsteht keine PF-Umleitung.

Der produktive Guest-Pfad setzt Port 53 voraus (Guest-`nameserver` ist
Bridge-`.0` ohne Port-Override). Netze mit `backend: docker` haben keine
Host-Bridge-IP und bekommen weder Guest-DNS-Listener noch `*.svc`-Gateway-Records.

## Autoritative Zone

`spec.project` ist der kanonische DNS-Stackname. Aus jeder Attachment-Row
eines aktiven vmnet-Netzes entsteht:

```text
{vm}.{net}.{project}.vz.test.  15 IN A {attachment-ip}
*.{vm}.{net}.{project}.vz.test. 15 IN A {attachment-ip}
```

Laufende, deklarierte Docker-/Compose-Container erhalten nach `stack apply`
dieselbe Form mit ihrem Docker-Backend-Netz:

```text
{container}.{docker-net}.{project}.vz.test.   15 IN A {container-ip}
*.{container}.{docker-net}.{project}.vz.test. 15 IN A {container-ip}
```

Auf dem Guest-DNS-Listener des eigenen Netzes reicht zusätzlich der einzelne
VM-/Containername (`web`, `redis`). Kurzformen sind auf dem Mac und auf fremden
Netzen nicht autoritativ. Container-Kurznamen sind an die primäre vmnet-NIC
ihrer Docker-VM gebunden. Vollqualifizierte Namen und Wildcards funktionieren
auf Host und Guests.

Für jede IPv4-Adresse entsteht außerdem ein PTR auf den kanonischen Namen,
zum Beispiel `10.0.80.10.in-addr.arpa → web.dmz.shop.vz.test`. Unbekannte
Reverse-Namen werden wie andere externe Queries zum Upstream weitergeleitet.

`svc` ist für Ingress (`{service}.svc.{project}.vz.test`) reserviert. VM-,
Container- und Netzwerknamen müssen gültige kleingeschriebene DNS-Labels sein
und dürfen nicht `svc` heißen; ungültige deklarative Namen stoppen die
Validierung, ungültige Runtime-Container stoppen die Veröffentlichung.

`attachment.project` gewinnt vor `network.project`. Fehlt das Projekt oder ist
ein Teil kein gültiges DNS-Label, wird kein Record erzeugt. Die Zone wird nach
`net create|attach|detach|delete`, Default-Netz-Änderungen und automatischen
Attachments im laufenden Supervisor neu gebaut. Listener, deren Bind gleich
bleibt, werden dabei nicht neu gestartet.

Ein Attachment kann zusätzlich Service-Aliase tragen:

```bash
vzctl net attach api-1 --network lan --ip 10.90.0.10 \
  --project shop --label vzctl.dev/dns-services=api,metrics
```

Das erzeugt `api.svc.shop.vz.test` und `metrics.svc.shop.vz.test`. Mehrere
Attachments dürfen denselben Service bereitstellen; die Antwort enthält dann
alle A-Adressen.

Die TTL ist per `VZCTL_DNS_TTL` konfigurierbar und wird auf 5–30 Sekunden
begrenzt. Unbekannte Namen unter `.vz.test` liefern autoritativ `NXDOMAIN`;
AAAA für einen vorhandenen A-Namen liefert eine leere autoritative Antwort.

## Forwarding

Nicht interne Queries werden als unveränderte UDP-DNS-Pakete weitergeleitet:

| Einstellung | Bedeutung |
|---|---|
| `VZCTL_DNS_UPSTREAM=system` | IPv4-Nameserver aus `/etc/resolv.conf`, bei jeder Query neu gelesen |
| `VZCTL_DNS_UPSTREAM=10.0.0.53` | ein expliziter UDP-Upstream auf Port 53 |
| `VZCTL_DNS_UPSTREAM=10.0.0.53:5353,10.0.0.54` | geordnete Upstream-Liste |

Pro Upstream gilt ein Timeout von zwei Sekunden; danach folgt der nächste.
Ohne Antwort liefert der Server `SERVFAIL`.

### VPN und Split DNS

`system` bildet nur `/etc/resolv.conf` ab. macOS verwaltet resolver-spezifische
Routen und VPN-Split-DNS zusätzlich in der Dynamic Store; diese Auswahl wird
in v1 nicht nachgebaut. Ein VPN-Name kann deshalb am vzctl-DNS scheitern oder am
falschen Upstream landen, obwohl `getaddrinfo` auf dem Host funktioniert.
Für reproduzierbare Labs muss der gewünschte VPN-/Corporate-Resolver explizit
über `VZCTL_DNS_UPSTREAM` gesetzt werden. Änderungen an `/etc/resolv.conf`
werden ohne Supervisor-Restart übernommen.

Der echte System-Upstream lässt sich opt-in prüfen:

```sh
VZCTL_DNS_LAB=1 swift test --package-path daemon \
  --filter systemUpstreamLabResolvesExternalName
```

## macOS-Systemresolver

Issue [#27](https://github.com/frankhildebrandt/vzctl/issues/27) verwaltet
pro Projekt eine scoped Resolver-Datei:

```text
# /etc/resolver/edge-dmz.vz.test
# managed-by: vzctl
# project: edge-dmz
# owner: config-…
nameserver 127.0.0.1
port 15353
```

Im Environment-Verzeichnis wird `spec.project` aus
`hypernetwork.config.yaml` gelesen:

```sh
sudo vzctl dns install-resolver
sudo vzctl dns uninstall-resolver
```

Über die Supervisor-REST-API. Bei Permission Denied zeigt macOS einen
Admin-Dialog (osascript); dasselbe gilt für Apply/`ensure_dns`:

```http
POST /v1/dns/resolver
{"config":"/path/to/stack"}

DELETE /v1/dns/resolver?config=/path/to/stack
```

Alternativ sind `--config <path>` und `--project <dns-label>` möglich.
`--project` muss mit `spec.project` übereinstimmen, falls eine Config gelesen
wird. `VZCTL_DNS_PORT` muss bei `vz-edge` und beim Installieren identisch
gesetzt sein; Default ist `15353`. Beide Commands unterstützen
`--format human|json`.

Install und Cleanup sind idempotent. vzctl schreibt atomar mit Modus `0644`
und folgt weder einem Resolver-Datei-Symlink noch einem
`/etc/resolver`-Symlink. Beim Uninstall wird nur eine Datei mit passendem
`managed-by`-, Projekt- und Config-Owner-Marker gelöscht. Fremde Dateien werden
mit Exit `19` als Kollision gemeldet und nie überschrieben oder entfernt.

Zwei unterschiedlich benannte Projekte erhalten unterschiedliche Dateien und
Zones. Zwei Configs mit demselben `spec.project` würden dieselbe Zone
beanspruchen; der Config-Owner-Marker macht das zum harten Konflikt. Das Projekt
muss deshalb repository-/hostweit eindeutig sein.

Zusätzlich verwaltet vzctl gemeinsam für alle Projekte
`/etc/resolver/in-addr.arpa`. Damit erreichen macOS-Systemabfragen für IPv4-PTR
den Host-Listener. Diese Datei bleibt bestehen, solange mindestens ein
verwalteter Projekt-Resolver existiert, und wird beim letzten Uninstall
ownership-geprüft entfernt. Eine fremde Reverse-Resolver-Datei wird nie
überschrieben.

## mDNS

Der PF-Anchor lässt IPv4-mDNS (`224.0.0.251:5353/UDP`) aus den dem vmnet
zugeordneten Quellnetzen passieren. mDNS bleibt jedoch gemäß RFC 6762
link-lokal: vzctl routet oder reflektiert Multicast nicht zwischen vmnet,
Docker-Bridges und dem Mac und veröffentlicht `.vz.test` nicht als `.local`.
Die oben beschriebenen A-/Wildcard-/PTR-Namen laufen über Unicast-DNS auf
`.0:53`; vorhandene `.local`-mDNS-Nutzung im selben Layer-2-Netz bleibt davon
unberührt.

Der spätere Stack-Reconciler (#34) ruft bei `down --purge` denselben
ownership-geprüften Cleanup-Pfad auf. Bis dieser P3-Command vorhanden ist,
erfolgt der Cleanup explizit mit `dns uninstall-resolver`.

### Smoke-Test

Mit laufendem Supervisor und aktiver `web`-VM:

```sh
sudo vzctl dns install-resolver
dscacheutil -q host -a name web.dmz.edge-dmz.vz.test
curl http://web.dmz.edge-dmz.vz.test
```

`curl`, Browser und andere libc-Clients verwenden den macOS-Systemresolver.
`dig` bildet die macOS-Split-DNS-Auswahl dagegen nicht zuverlässig ab und kann
`/etc/resolver` ohne explizites `@server` umgehen.

Der opt-in Multi-Net-/Docker-Smoke-Test erwartet den laufenden Referenz-Stack,
prüft DNS-Horizonte, `.1`-Netzmasken, PF-Portabschirmung und Idempotenz:

```sh
make smoke-split-dns
# zusätzlich Helper-Crash + Reconcile:
VZCTL_SMOKE_CRASH_HELPER=1 make smoke-split-dns
```

### Direkter CLI-Query

`vzctl dns query` baut selbst ein DNS-Paket und sendet es per UDP direkt an
den Host-Listener. Es liest weder `/etc/resolver` noch den libc-Resolver:

```sh
vzctl dns query web.dmz.edge-dmz.vz.test
vzctl dns query --type A --server 127.0.0.1:15353 \
  web.dmz.edge-dmz.vz.test
vzctl dns query --type AAAA web.dmz.edge-dmz.vz.test --format json
vzctl dns query --type PTR 10.0.80.10.in-addr.arpa
```

Default-Server ist `127.0.0.1:15353`, Default-Typ `A`; unterstützt werden `A`
`AAAA` und `PTR`. Die Human-Ausgabe verwendet das Format
`NAME TTL CLASS TYPE DATA`. Das CLI-v1-JSON enthält Query, RCODE,
Authoritative-/Truncated-Flags und `answers[]`:

```json
{
  "apiVersion": "vzctl.dev/v1",
  "command": "dns.query",
  "status": "ok",
  "exit_code": 0,
  "summary": {
    "message": "web.dmz.edge-dmz.vz.test A via 127.0.0.1:15353: 1 answer(s), NOERROR",
    "answers": 1,
    "rcode": "NOERROR"
  },
  "query": {
    "name": "web.dmz.edge-dmz.vz.test",
    "type": "A",
    "server": "127.0.0.1:15353"
  },
  "rcode": "NOERROR",
  "rcode_code": 0,
  "authoritative": true,
  "truncated": false,
  "answers": [
    {
      "name": "web.dmz.edge-dmz.vz.test",
      "type": "A",
      "class": "IN",
      "ttl": 15,
      "data": "10.80.0.10"
    }
  ]
}
```

`NOERROR` liefert Exit `0`, auch ohne Answers. `NXDOMAIN`, `SERVFAIL` und
andere Fehler-RCODES liefern Exit `20`, behalten RCODE und Answers aber im
Fail-Envelope. Timeout, ungültige Antworten und das UDP-`TC`-Bit liefern
ebenfalls Exit `20`; Usage ist `2`, ungültiger Input `3`.

## Guest-Konfiguration

`vzctl vm create` übernimmt das Projekt aus dem ausgewählten Network- bzw.
Attachment-Record und rendert den privaten NoCloud-Seed pro Clone:

```yaml
version: 2
ethernets:
  nic0:
    match:
      macaddress: "02:…"
    set-name: enp0s1
    dhcp4: false
    dhcp6: false
    addresses:
      - 10.80.0.10/24
    routes:
      - to: default
        via: 10.80.0.0
        on-link: true
    nameservers:
      addresses:
        - 10.80.0.0
      search:
        - edge-dmz.vz.test
```

Damit ist Bridge-`.0` der einzige primäre Guest-Resolver; Host-Resolver werden
nicht in den Seed kopiert. Ein Netz ohne `project` erhält weiterhin `.0` als
Nameserver, aber keine erfundene Search-Zone. Für interne Records und
Search-Auflösung muss das Netz deshalb mit `vzctl net create --project
<project>` angelegt werden. Das gilt auch, wenn dieses Netz anschließend als
Default-Netz ausgewählt wird.

## Health und Events

`daemon.health` enthält `dns_ok` und ein `dns`-Objekt mit Listenern,
Record-/Zone-Zahl, TTL, Upstream und `last_error`. `ok=false` markiert einen
degradierten Supervisor, etwa wenn `.0:53` nicht gebunden werden konnte
(fehlender `dns install-bind-helper` oder Bridge noch nicht up).

`vzctl doctor` prüft denselben Pfad als Check `dns.bind_helper` (Warn + Hint
`sudo vzctl dns install-bind-helper`). Die UI-Doctor-Seite und das DNS-Status-
Tile bieten denselben Install-Button (Admin-Dialog).

Jeder erfolgreiche Reload emittiert `dns.reloaded`; ein Snapshot- oder
Bind-Fehler emittiert `dns.reload_failed`. Stirbt der Supervisor, verschwinden
alle DNS-Sockets sofort. Das ist die akzeptierte Alpha-Semantik: UDS-Health und
Event-Stream sind dann ebenfalls nicht erreichbar, während VM-Helper
weiterlaufen.

## Grenzen v1

- UDP only; TCP-Fallback und DNSSEC-Validierung sind nicht implementiert.
- Der Forwarder unterstützt IPv4-Upstreams.
