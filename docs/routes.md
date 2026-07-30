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
Für jedes Netz ist ausschließlich die aus dem CIDR abgeleitete `.2` zulässig.
Host-Gateway und DNS bleiben `.0`; normale Gäste beginnen bei `.10`.

## Konfiguration anwenden

```bash
vzctl route apply
vzctl route apply --router router --format json
```

Der Supervisor liest Rollen und Attachments, validiert die `.2`-Adressen und
sendet den Plan an den laufenden VM-Helper. Nur der Helper öffnet die
virtio-vsock-Verbindung und pusht die Konfiguration per Guest-Agent `exec`.
SSH ist weder Happy Path noch Fallback dieses Befehls.

Der Apply schreibt atomar:

- `/etc/sysctl.d/90-vzctl-router.conf`;
- `/etc/vzctl/routes.json`;
- den Laufzeitwert `net.ipv4.ip_forward=1`;
- eine leere Basis-Forward-Policy mit Default `DROP`.

Identische Inhalte und bereits gesetzte Laufzeitwerte ergeben
`changed: false`. Damit ist ein wiederholtes `route apply` idempotent.

Die Default-DROP-Regel verhindert Cross-Net-Traffic trotz aktivem
IP-Forwarding. #33 ergänzt die expliziten Allow-Regeln; Host-`pf` bleibt
außerhalb des Default-Pfads.
