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

IP-Konvention: Host-Gateway und DNS liegen auf der Netzadresse `.0`; der
geschützte macOS-Host-/Ingress-Alias liegt auf `.1`, Router auf `.2`, Gäste
beginnen bei `.10`. Auf `.1` veröffentlicht vzctl ausschließlich konfigurierte
Ingress-Ports. Eine Attachment-IP muss `.2` oder im
Guest-Bereich liegen; IPs sind pro Netz eindeutig.

`.0:53` bleibt der logische Guest-DNS-Endpunkt. PF leitet ihn intern auf einen
exklusiven hohen `vz-edge`-Port um, damit `mDNSResponder` nicht versehentlich
die Host-Antwort ausliefert. Die Guest-Konfiguration ändert sich dadurch nicht.

## Default-Netzwerk

Issue [#51](https://github.com/frankhildebrandt/vzctl/issues/51) ergänzt den
Happy Path für VMs ohne explizites Attachment:

```bash
vzctl net default set lan --cidr 10.70.0.0/24
vzctl net default show
vzctl vm create web --from ubuntu-base --disk 20
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
erreichbar; Guest-DNS `.0:53` läuft über denselben PF-Redirect wie in
Shared-Mode-Netzen. Internet-NAT entfällt; Gäste
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
Auf dem eigenen Listener ist deshalb `{vm}` als Kurzname gültig; vollständig
heißen VM und Container `{name}.{network}.{project}.vz.test`, inklusive
Wildcard-A und IPv4-PTR. Docker-Container verwenden als Kurzname-Kontext die
primäre vmnet-NIC ihrer Docker-VM. `svc` bleibt als VM-, Container- und
Netzwerkname reserviert. IPv4-mDNS auf `224.0.0.251:5353` bleibt innerhalb des
jeweiligen Layer-2-Netzes erlaubt, wird aber nicht zwischen vmnet/Docker-Netzen
reflektiert.
Cross-Net-Traffic wird dadurch nicht freigeschaltet: Dafür bleiben Router plus
[#33](https://github.com/frankhildebrandt/vzctl/issues/33) zuständig.

## Host-Netzwechsel und Sleep/Wake

Der Supervisor beobachtet `NWPathMonitor` sowie macOS Sleep/Wake. Jede echte
Änderung erhöht eine monotone `network_epoch`; Flapping wird zwei Sekunden
entprellt und Reconcile läuft seriell. Bei `path=unsatisfied` gibt es keine
Repair-Schleife. Nach einem nutzbaren Pfad gelten begrenzte Backoffs bis
30 Sekunden.

Offline/Captive/Degraded werden danach passiv erneut geprüft, weil eine
Portal-Freigabe nicht zwingend einen neuen `NWPath` erzeugt. Diese Proben sind
nicht-destruktiv. Der opt-in VM-Fallback darf höchstens einmal pro Epoch laufen.

Recovery prüft Host-/VPN-Routen, `net.verify`, wendet das `vz-edge`-Manifest
idempotent neu an und trennt interne Gesundheit, Host-Portal/Egress und
Guest-Egress. Live-Refs, Netzwerk-IDs, IPs und Serialisierungen bleiben im
Default-Modus unverändert. CIDR-Konflikte werden gemeldet, niemals umnummeriert.
Locale, Zeitzone, Land und Wall-Clock sind keine Netz-Inputs. Helper führen
ihre Wake-Time-Sync- und VirtioFS-Remount-Logik unabhängig weiter aus.

Bestehende externe TCP-Verbindungen dürfen bei neuer öffentlicher IP abbrechen;
interne Verbindungen dürfen durch einen wachen LAN/WLAN-Wechsel nicht
unterbrochen werden. Captive Portal pausiert die Egress-Frist bis zur Freigabe.

## Ownership

Live `vmnet_network_ref` + Host-Bridge liegen in **`vz-net`** (LaunchAgent
`com.vzctl.net`, Socket `$VZCTL_STATE_DIR/net.sock`). Der Control-Plane-Supervisor
hält Desired State in SQLite und orchestriert über
[`docs/specs/vz-net-v1.md`](specs/vz-net-v1.md) (`net.acquire` /
`net.release` / `net.serialize`). DNS-Listener liegen in `vz-edge` und binden
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
