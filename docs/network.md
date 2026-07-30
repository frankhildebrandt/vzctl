# Network CRUD v1

Issue [#31](https://github.com/frankhildebrandt/vzctl/issues/31) implementiert
Supervisor-owned `shared`-vmnet-Netze und persistente VM-Attachments:

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

- SQLite hält Netze, Labels/Metadaten und Attachments.
- Der Supervisor rekonstruiert beim Start für jede Network-Row einen
  `vmnet_network_ref`; DHCP und vmnet-DNS-Proxy bleiben aus.
- Nur der Supervisor hält diese Refs. Der spätere Helper-Startpfad darf daraus
  ausschließlich serialisierte Attachment-Handles beziehen; die persistente
  Attachment-Row ist bereits dessen Sollzustand.
- `net attach` und `net detach` scheitern bei einer laufenden/startenden VM.
- `net delete` scheitert, solange eine VM attached ist.
- Delete und sauberer Supervisor-Stop droppen den letzten starken Ref. Das ist
  zwingend: `stop_interface` allein gibt die CIDR laut G0 nicht frei.

IP-Konvention: Host-Gateway und DNS liegen auf der Netzadresse `.0`, Router auf
`.2`, Gäste beginnen bei `.10`. Eine Attachment-IP muss im CIDR und im
Guest-Bereich liegen; IPs sind pro Netz eindeutig.

## Unclean Exit

Nach `kill -9` kann vmnet die CIDR-Reservation trotz Prozessende blockieren.
Beim nächsten Start bleibt die Desired-State-Row erhalten, wird aber als
`runtime_state=orphaned` mit `last_error` gelistet. `vzctl doctor` meldet dann
WARN. Für Alpha gilt: betroffene CIDR bis zum Host-Reboot nicht wiederverwenden
oder ein neues Netz mit frischer CIDR anlegen.

Der gemessene Hintergrund steht in
[`docs/spikes/g0-network.md`](spikes/g0-network.md). Router-Template
([#32](https://github.com/frankhildebrandt/vzctl/issues/32)) und Policies
([#33](https://github.com/frankhildebrandt/vzctl/issues/33)) sind nicht Teil
dieses Slices.
