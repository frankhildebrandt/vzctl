# Network CRUD und Default-Netz v1

Issue [#31](https://github.com/frankhildebrandt/vzctl/issues/31) implementiert
Desired-State-`shared`-vmnet-Netze und persistente VM-Attachments (Runtime in
`vz-net`):

```bash
vzctl net create dmz --cidr 10.80.0.0/24 --mode shared \
  --label tier=edge --project demo --stack dev
vzctl net attach web --network dmz --ip 10.80.0.10
vzctl net list --format json
vzctl net detach web --network dmz
vzctl net delete dmz
```

`--label key=value` ist wiederholbar. `--project` und `--stack` werden am Netz
gespeichert und von Attachments geerbt, falls sie dort nicht überschrieben
werden. Bridged Mode ist in v0.1 unsupported.

## Desired State und Lifecycle

- SQLite (Control-Plane) hält Netze, Labels/Metadaten und Attachments.
- Beim CP-Start wird für jede vmnet-Network-Row `net.acquire` gegen **`vz-net`**
  aufgerufen (idempotent); DHCP und vmnet-DNS-Proxy bleiben aus.
- Live-Refs hält nur `vz-net`. Helper beziehen serialisierte Attachment-Handles
  über CP `helper.networks` → `net.serialize`.
- `net attach` und `net detach` scheitern bei einer laufenden/startenden VM.
- `net delete` scheitert, solange eine VM attached ist; danach ruft CP
  `net.release` auf.
- Sauberer CP-Stop releast **keine** vmnet-Refs. Sauberer `vz-net`-Stop
  (SIGTERM) gibt alle CIDRs frei; `stop_interface` allein reicht laut G0 nicht.

IP-Konvention: Host-Gateway und DNS liegen auf der Netzadresse `.0`, Router auf
`.2`, Gäste beginnen bei `.10`. Eine Attachment-IP muss `.2` oder im
Guest-Bereich liegen; IPs sind pro Netz eindeutig.

## Default-Netzwerk

Issue [#51](https://github.com/frankhildebrandt/vzctl/issues/51) ergänzt den
Happy Path für VMs ohne explizites Attachment:

```bash
vzctl net default set lan --cidr 10.70.0.0/24
vzctl net default show
vzctl vm create web --from ubuntu-base --data-disk 4
# web erhält z. B. 10.70.0.10/24, Gateway/DNS 10.70.0.0
```

Die Konfiguration aus Name und CIDR liegt als Singleton im SQLite Desired
State. `set` ist idempotent. Fehlt die zugehörige Network-Row später, erzeugt
der nächste VM-Create-Pfad sie erneut als `mode=shared`.

Der Supervisor vergibt unter seinem Registry-Lock die erste freie Guest-IP ab
Offset `.10`. `vm create --network <name>` wählt stattdessen ein vorhandenes
Netz. Ein bereits per `net attach` gesetztes explizites Attachment gewinnt
ebenfalls. Automatische Default-Attachments sind intern markiert; ein späteres
explizites Attachment ersetzt nur dieses automatische Attachment. Explizite
Multi-NIC-Attachments, etwa für Router, bleiben erhalten.

`shared` lässt NAT44 standardmäßig aktiv (`natEgress: true`) und bietet damit
Zugriff auf Host und Internet. Mit `natEgress: false` wird das Netz als
host-only (`VMNET_HOST_MODE`) angelegt: ICMP zum Host-Gateway `.0` bleibt
erreichbar, **Gast-DNS UDP `.0:53` ist auf Host-Mode derzeit unzuverlässig**
(Shared-Mode-Netze sind davon nicht betroffen). Internet-NAT entfällt; Gäste
nutzen dann Router `.2` als Default-Gateway und brauchen Policy `to: internet`
auf dem Router. Für Stacks, die Gast-DNS brauchen (cloud-init/`apt`), `lan`
daher mit `natEgress: true` belassen und Router-Zuordnung über `policies.*.via`
steuern.
DHCP und vmnet-DNS-Proxy bleiben deaktiviert; die VM erhält ihre
statische Adresse, Default-Route `via .0 on-link` (bzw. `via .2` ohne NAT) und
ausschließlich `.0` als DNS über den pro Clone erzeugten NoCloud-Seed. Bei einem
projektgebundenen Netz wird zusätzlich `{project}.vz.test` als Search-Domain
gesetzt. Das gilt identisch für explizite und automatische Default-Attachments
sowie für die Primär-NIC einer Router-VM. Router `.2` bleibt der Next Hop für
explizite Cross-Net-Routen.
Cross-Net-Traffic wird dadurch nicht freigeschaltet: Dafür bleiben Router plus
[#33](https://github.com/frankhildebrandt/vzctl/issues/33) zuständig.

## Ownership

Live `vmnet_network_ref` + Host-Bridge liegen in **`vz-net`** (LaunchAgent
`com.vzctl.net`, Socket `$VZCTL_STATE_DIR/net.sock`). Der Control-Plane-Supervisor
hält Desired State in SQLite und orchestriert über
[`docs/specs/vz-net-v1.md`](specs/vz-net-v1.md) (`net.acquire` /
`net.release` / `net.serialize`). DNS-Listener bleiben im Supervisor und binden
erst nach erfolgreichem Acquire auf Bridge-`.0`.

## Unclean Exit

Nach `kill -9` auf **`vz-net`** kann Apple die CIDR-Reservation trotz Prozessende
blockieren. Control-Plane-Crashes allein orphanen CIDRs **nicht** mehr — die Refs
leben in `vz-net` weiter; nach CP-Restart ist `net.acquire` idempotent.

Beim nächsten CP-Start bleibt die Desired-State-Row erhalten; schlägt Acquire fehl,
wird sie als `runtime_state=orphaned` mit `last_error` gelistet. `vzctl doctor`
meldet dann WARN. Für Alpha gilt: betroffene CIDR bis zum Host-Reboot nicht
wiederverwenden oder ein neues Netz mit frischer CIDR anlegen.

Der gemessene Hintergrund steht in
[`docs/spikes/g0-network.md`](spikes/g0-network.md). Spike für den Split:
`scripts/spike-vz-net-cp-crash.sh`. Router-Template,
deklarative Forward-Policies sowie `route apply|plan|status` sind in
[`routes.md`](routes.md) beschrieben. Autoritative Records, Dual-UDP-Listener
und Forwarding sind in [`dns.md`](dns.md) spezifiziert.
