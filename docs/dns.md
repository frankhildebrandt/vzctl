# Supervisor-DNS v1

Issue [#26](https://github.com/frankhildebrandt/vzctl/issues/26) implementiert
den Supervisor-owned DNS aus ADR 0002. Der Server spricht UDP und hält parallel
den Host-Listener sowie je aktivem vmnet einen Guest-Listener:

| Listener | Default | Zweck |
|---|---|---|
| Host | `127.0.0.1:15353` | macOS `/etc/resolver` ab #27 |
| Guest | Bridge-`.0:53` | Standard-DNS der Guests ab #29 |

Für einen unprivilegierten Development-Run kann der Guest-Port mit
`VZCTL_DNS_GUEST_PORT=15353` angehoben werden. Der #29-Produktionspfad setzt
Port 53 voraus.

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

## Health und Events

`daemon.health` enthält `dns_ok` und ein `dns`-Objekt mit Listenern,
Record-/Zone-Zahl, TTL, Upstream und `last_error`. `ok=false` markiert einen
degradierten Supervisor, etwa wenn `.0:53` nicht gebunden werden konnte.

Jeder erfolgreiche Reload emittiert `dns.reloaded`; ein Snapshot- oder
Bind-Fehler emittiert `dns.reload_failed`. Stirbt der Supervisor, verschwinden
alle DNS-Sockets sofort. Das ist die akzeptierte Alpha-Semantik: UDS-Health und
Event-Stream sind dann ebenfalls nicht erreichbar, während VM-Helper
weiterlaufen.

## Grenzen v1

- UDP only; TCP-Fallback und DNSSEC-Validierung sind nicht implementiert.
- Der Forwarder unterstützt IPv4-Upstreams.
- `/etc/resolver` (#27), `vzctl dns query` (#28) und Guest-cloud-init (#29)
  bleiben eigene Slices.
