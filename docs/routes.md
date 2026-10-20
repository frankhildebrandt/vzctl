# Router-VM und `route apply`

Issue [#32](https://github.com/frankhildebrandt/vzctl/issues/32) ergänzt den
Network-Sollzustand aus #31 um Router-VMs.

## Router anlegen

```bash
vzctl vm create router --from ubuntu-base --disk 20 --role router
vzctl net attach router --network dmz --ip 10.80.0.2
vzctl net attach router --network lan --ip 10.90.0.2
```

`--role router` schreibt `roles: ["router"]` in das VM-Manifest und ergänzt
das NoCloud-Template um einen geschlossenen Bootdefault
`net.ipv4.ip_forward=0`. Der Bootstrap aktiviert Forwarding erst nach dem
Anlegen der eigenen Deny-Tabelle. Eine Router-VM braucht mindestens zwei Attachments.
Für jedes Netz ist ausschließlich die aus dem CIDR abgeleitete `.2` zulässig
(Ausnahme: Docker+Router-VMs — Parent-vmnet behält die Guest-IP, nur das
`backend: docker`-Netz braucht `.2` als `bip`).
Host-Gateway und DNS bleiben `.0`; normale Gäste beginnen bei `.10`.

Peer-Router bekommen für `backend: docker`-CIDRs automatisch Static Routes
(`ip route replace <cidr> via <docker-parent-ip>`).

## Konfiguration anwenden

```bash
vzctl route apply --router router
vzctl route plan --config examples/edge-dmz/hypernetwork.config.yaml
vzctl route apply --config examples/edge-dmz/hypernetwork.config.yaml
vzctl route status --router router --format json
```

Der Supervisor liest Rollen und Attachments, validiert die `.2`-Adressen und
sendet den Plan an den laufenden VM-Helper. Nur der Helper öffnet die
virtio-vsock-Verbindung und pusht die Konfiguration per Guest-Agent `exec`.
SSH ist weder Happy Path noch Fallback dieses Befehls.

Ohne `--config` wird eine `hypernetwork.config.yaml` im aktuellen Verzeichnis
verwendet. Fehlt auch diese, gilt wie in #32 eine leere Allow-Liste bei
Default-Deny. `route plan` liest denselben Sollzustand, verändert den Gast aber
nicht und liefert `policy_changes[]` mit `add`, `update` oder `remove`.

## Forward-Policies

```yaml
spec:
  policies:
    - name: dmz-default
      network: dmz
      forward: deny-all
      allow:
        - { to: lan, proto: tcp, ports: [5432] }
        - { to: dmz, proto: icmp }
```

`network` ist das Quellnetz. `to` ist ein weiteres Attachment desselben
Routers **oder** `internet` (Egress über ein `natEgress: true`-Attachment des
Routers; nftables + MASQUERADE). Optional pinnt `via` die Policy auf eine
Router-VM (Config-Key), analog zu `routes.*.via` — nötig, wenn mehrere Router
am Quellnetz hängen. v1 unterstützt `tcp` und `udp` mit mindestens
einem Port sowie `icmp` ohne Ports. Nur `forward: deny-all` ist zulässig.
Namen, Netze, Protokolle und Ports werden vor dem Guest-Apply validiert.
Policy-Namen sind auf Buchstaben, Ziffern, Punkt, Bindestrich und Unterstrich
begrenzt. Bei mehreren laufenden Routern wird eine Policy über `via` oder
(eindeutig) über ihr Quellnetz genau einem Router zugeordnet; keine oder
mehrdeutige Zuordnungen sind ungültig (`set policies.*.via`).

Der Apply speichert bzw. setzt:

- `/etc/sysctl.d/90-vzctl-router.conf`;
- `/etc/vzctl/routes.json`;
- `/etc/vzctl/vzctl.nft`;
- den Laufzeitwert `net.ipv4.ip_forward=1`;
- die vollständige nftables-Tabelle `inet vzctl`.

Identische Inhalte und bereits gesetzte Laufzeitwerte ergeben
`changed: false`. Damit ist ein wiederholtes `route apply` idempotent.

## Wiederherstellung nach Gast-Neustart

`route apply` installiert und aktiviert einen eigenen Boot-Dienst:
`vzctl-router.service` auf systemd oder `/etc/init.d/vzctl-router` im
OpenRC-Runlevel `default`. Der Dienst ruft
`/usr/local/lib/vzctl/router-apply --restore` auf und liest den letzten
gespeicherten Plan aus `/etc/vzctl/`. Er läuft nach Netzwerkbereitschaft und,
falls vorhanden, nach dem Boot-Lauf des Distributionsdienstes `nftables`.

Zusätzlich installiert Apply auch für vorhandene VMs Start-Guards für Docker
und den Guest-Agent. Unter systemd sind dies privilegierte `ExecStartPre`-
Aufrufe, unter OpenRC eine eigene Bootstrap-Abhängigkeit. Der persistente
Sysctl bleibt beim Boot zunächst auf `0`; Bootstrap und Restore aktivieren
Forwarding erst mit vorhandenen eigenen Regeln. Eigene bereits aktive Regeln
werden bei einem bloßen Dienstneustart erhalten.

Die Wiederherstellung ersetzt ausschließlich die Tabelle `inet vzctl`.
Löschen der alten eigenen Tabelle und Laden der neuen Regeln erfolgen in
einer nftables-Transaktion; ein fehlerhafter Regelsatz lässt die bisherige
Tabelle bestehen. Es gibt kein globales `flush ruleset`. Andere Tabellen
und `/etc/nftables.conf` bleiben unverändert; der Distributionsdienst
`nftables` wird weder aktiviert noch gestartet. Der neue Dienst wird beim
Apply nur für spätere Boots aktiviert, nicht zusätzlich live gestartet.

Ein enger Migrationsfall entfernt außerdem eine alte, von vzctl selbst
erzeugte globale `ip/filter/FORWARD`-Drop-Chain. Voraussetzung sind der genaue
alte Befehl in einer geschützten CloudInit-Provenienzdatei sowie eine leere
Standard-Chain. Entfernung und neue eigene Regeln bilden eine gemeinsame
Transaktion. Bei fremden Regeln oder unklarer Herkunft wird vor persistenten
Änderungen abgebrochen. Bei einem späteren nft-Transaktionsfehler bleiben
eigene konservative Boot-Guards bestehen; laufende Firewall und gespeicherte
Routen bleiben unverändert.

Eine vorhandene fremde Unit unter dem vzctl-Dienstnamen, eine verlinkte Unit,
ein nicht unterstütztes Init-System oder ein Enable-Fehler bricht den Apply
sichtbar ab. Restore lehnt fehlende oder verlinkte gespeicherte Dateien ab.
Ein Enable-Fehler kann nach erfolgreicher Anwendung der Laufzeitregeln
auftreten; er bedeutet dann ausdrücklich, dass die Boot-Persistenz noch
nicht hergestellt ist.

Der Helper ist Teil des Guest-Utils-Fingerprints. Bestehende Gäste erhalten
ihn über den normalen Guest-Utils-Rollout; ein anschließendes `route apply`
(auch innerhalb von Stack-Apply) richtet den Boot-Dienst ein. Allein der
Dateiaustausch aktiviert noch keinen Dienst. Ein späterer manueller globaler
Firewall-Flush durch fremde Werkzeuge ist außerhalb dieser Boot-Garantie;
danach muss der gespeicherte vzctl-Plan erneut angewendet werden.

Die Forward-Chain hat immer `policy drop`. Zusätzlich erlaubt sie
`established,related` Rückverkehr und ausschließlich die gerenderten
Policy-Regeln. `route status --format json` prüft Tabelle und Statusdatei im
Gast und liefert `active`, `forward_policy`, `policies[]` und `rules[]`.

Das konfigurierbare Default-Netz bleibt ein separates `shared`-vmnet mit
vollem NAT-Egress. Die Router-Regeln öffnen nur Cross-Net-Forwarding. Host-`pf`
bleibt außerhalb des Default-Pfads.
