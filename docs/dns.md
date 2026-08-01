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

- **UDP** (`:53`): Helper bindet und gibt den Socket-FD zurück (Guest-DNS auf
  Bridge-`.0`).
- **TCP** (`:80`/`:443`): Helper legt bei Bedarf Host-Service-Alias `.1` auf dem
  Bridge an, bindet+listens und streamt akzeptierte Client-FDs über dieselbe
  UDS-Verbindung. Ingress-`*.svc` ist Split-Horizon:
  Host-Listener (`127.0.0.1:15353`) → `127.0.0.1` (Mac kann Bridge-`.1` nicht
  dialen: `EHOSTDOWN`); Guest-Listener (`.0:53`) → Host-Service-`.1` (vmnet
  verwirft Guest-TCP zu `.0`; Loopback wäre im Guest tot, falls mDNSResponder
  die Query stiehlt).

```sh
sudo vzctl dns install-bind-helper
sudo vzctl dns uninstall-bind-helper
```

Ohne Helper schlägt `.0:53` mit `Permission denied` fehl (`dns_ok=false`);
Guest-Ingress über `.0:80/:443` bleibt dann ebenfalls ohne Proxy-Listener.
Für unprivilegierte Dev-Läufe ohne Helper:

```sh
VZCTL_DNS_GUEST_PORT=15353
```

Der produktive Guest-Pfad setzt Port 53 voraus (Guest-`nameserver` ist
Bridge-`.0` ohne Port-Override). Netze mit `backend: docker` haben keine
Host-Bridge-IP und bekommen weder Guest-DNS-Listener noch `*.svc`-Gateway-Records.

## Autoritative Zone

Aus jeder Attachment-Row eines aktiven Netzes entsteht:

```text
{vm}.{net}.{project}.vz.test.  15 IN A {attachment-ip}
```

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

### Direkter CLI-Query

`vzctl dns query` baut selbst ein DNS-Paket und sendet es per UDP direkt an
den Host-Listener. Es liest weder `/etc/resolver` noch den libc-Resolver:

```sh
vzctl dns query web.dmz.edge-dmz.vz.test
vzctl dns query --type A --server 127.0.0.1:15353 \
  web.dmz.edge-dmz.vz.test
vzctl dns query --type AAAA web.dmz.edge-dmz.vz.test --format json
```

Default-Server ist `127.0.0.1:15353`, Default-Typ `A`; unterstützt werden `A`
und `AAAA`. Die Human-Ausgabe verwendet das Format
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
