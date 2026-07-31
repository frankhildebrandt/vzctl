# edge-dmz

Referenz-Environment für einen isolierten DMZ-Stack mit zwei Netzen:

- `router` hängt mit der reservierten Router-Adresse `.2` an `dmz` und `lan`;
- `web` und `docker` sind APFS Linked Clones im DMZ-Netz;
- vzctl-DNS bedient Guests über die Bridge-Adresse `.0` und den Host über
  `127.0.0.1:15353`;
- Forwarding ist standardmäßig gesperrt. Die Policy erlaubt aus der DMZ nur
  TCP/5432 ins LAN sowie ICMP innerhalb der DMZ.

Die Cloud-Init-Dateien enthalten keine Zugangsdaten. VM-Identität,
SSH-Schlüssel, Agent-Token, statische Adressen und DNS werden pro Clone von
`vzctl` erzeugt.

## Prüfen und vergleichen

Vom Repository-Root:

```bash
cargo run -q -p vzctl -- validate -C examples/edge-dmz
cargo run -q -p vzctl -- plan -C examples/edge-dmz
cargo run -q -p vzctl -- diff -C examples/edge-dmz
```

`validate` arbeitet vollständig offline. `plan` und `diff` lesen den
Actual State über `stack.inspect`, verändern aber weder Lease, Journal noch
Runtime-Ressourcen. Dafür muss der Supervisor laufen.

## Starten und stoppen

Nach Installation von `vzctl` und Supervisor:

```bash
vzctl image pull ubuntu-latest
vzctl up -C examples/edge-dmz
vzctl down -C examples/edge-dmz
```

`down` stoppt die VMs und behält verwaltete Ressourcen. Erst
`down --purge` entfernt Stack-Ressourcen; das versiegelte Base-Image bleibt
unverändert.
