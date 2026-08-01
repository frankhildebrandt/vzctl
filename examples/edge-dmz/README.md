# edge-dmz

Referenz-Environment für einen isolierten DMZ-Stack mit zwei Netzen:

- `router` hängt mit der reservierten Router-Adresse `.2` an `dmz` und `lan`;
- `web` und `docker` sind APFS Linked Clones im DMZ-Netz;
- vzctl-DNS bedient Guests über die Bridge-Adresse `.0` und den Host über
  `127.0.0.1:15353`;
- Forwarding ist standardmäßig gesperrt. Die Policy erlaubt aus der DMZ nur
  TCP/5432 ins LAN sowie ICMP innerhalb der DMZ.

Die Cloud-Init-Dateien enthalten keine Zugangsdaten. VM-Identität,
SSH-Host-Keys, Agent-Token, statische Adressen und DNS werden pro Clone von
`vzctl` erzeugt. Die Docker-VM bekommt zusätzlich einen managed SSH-User
`vzctl` für den Host-Docker-Context.

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
vzctl docker -- ps
vzctl port list
vzctl down -C examples/edge-dmz
```

`down` stoppt die VMs und behält verwaltete Ressourcen. Erst
`down --purge` entfernt Stack-Ressourcen inkl. Docker-Context und Port-Forwards;
das versiegelte Base-Image bleibt unverändert.

Siehe auch [docs/docker.md](../../docs/docker.md), [docs/ports.md](../../docs/ports.md),
[certs-v1](../../docs/specs/certs-v1.md), [ingress-v1](../../docs/specs/ingress-v1.md) und
[oidc-v1](../../docs/specs/oidc-v1.md).

## v0.2 Ingress / OIDC

Das Beispiel aktiviert Local CA, Caddy Ingress und `oidc-simple` (Dev-Picker):

- `https://web.svc.edge-dmz.vz.test` → VM `web:80`
- `https://auth.svc.edge-dmz.vz.test` → `vzctl-oidc-simple` auf `127.0.0.1:5556`
- Login: Username aus der Liste wählen (keine Passwörter)
- Host-Alias `web.localhost` wenn `hostAliases: true`
- `web.requires: [oidc]` → Client-Autoconfig unter Application Support

Vor `up` Binaries holen / bauen:

```bash
./scripts/fetch-caddy.sh
cargo build -p vzctl-oidc-simple --release   # oder: make install
vzctl certs ca init
```

