# Router-VM und `route apply`

Issue [#32](https://github.com/frankhildebrandt/vzctl/issues/32) ergänzt den
Network-Sollzustand aus #31 um Router-VMs.

## Router anlegen

```bash
vzctl vm create router --from ubuntu-base --data-disk 4 --role router
vzctl net attach router --network dmz --ip 10.80.0.2
vzctl net attach router --network lan --ip 10.90.0.2
```

`--role router` schreibt `roles: ["router"]` in das VM-Manifest und ergänzt
das NoCloud-Template um den persistenten Sysctl
`net.ipv4.ip_forward=1`. Eine Router-VM braucht mindestens zwei Attachments.
Für jedes Netz ist ausschließlich die aus dem CIDR abgeleitete `.2` zulässig
(Ausnahme: Docker+Router-VMs — Parent-vmnet behält die Guest-IP, nur das
`backend: docker`-Netz braucht `.2` als `bip`).
Host-Gateway und DNS bleiben `.0`; normale Gäste beginnen bei `.10`.

Peer-Router bekommen für `backend: docker`-CIDRs automatisch Static Routes
(`ip route replace <cidr> via <docker-parent-ip>`).

## Konfiguration anwenden

```bash
vzctl route apply
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
Routers; nftables + MASQUERADE). v1 unterstützt `tcp` und `udp` mit mindestens
einem Port sowie `icmp` ohne Ports. Nur `forward: deny-all` ist zulässig.
Namen, Netze, Protokolle und Ports werden vor dem Guest-Apply validiert.
Policy-Namen sind auf Buchstaben, Ziffern, Punkt, Bindestrich und Unterstrich
begrenzt. Bei mehreren laufenden Routern wird eine Policy über ihr Quellnetz
genau einem Router zugeordnet; keine oder mehrdeutige Zuordnungen sind ungültig.

Der Apply schreibt atomar:

- `/etc/sysctl.d/90-vzctl-router.conf`;
- `/etc/vzctl/routes.json`;
- `/etc/vzctl/vzctl.nft`;
- den Laufzeitwert `net.ipv4.ip_forward=1`;
- die vollständige nftables-Tabelle `inet vzctl`.

Identische Inhalte und bereits gesetzte Laufzeitwerte ergeben
`changed: false`. Damit ist ein wiederholtes `route apply` idempotent.

Die Forward-Chain hat immer `policy drop`. Zusätzlich erlaubt sie
`established,related` Rückverkehr und ausschließlich die gerenderten
Policy-Regeln. `route status --format json` prüft Tabelle und Statusdatei im
Gast und liefert `active`, `forward_policy`, `policies[]` und `rules[]`.

Das konfigurierbare Default-Netz bleibt ein separates `shared`-vmnet mit
vollem NAT-Egress. Die Router-Regeln öffnen nur Cross-Net-Forwarding. Host-`pf`
bleibt außerhalb des Default-Pfads.
