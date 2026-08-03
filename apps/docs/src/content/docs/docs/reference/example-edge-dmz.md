---
title: Beispiel edge-dmz
description: Referenz-Environment mit DMZ, LAN, Router, Web und Docker.
---

Pfad im Repo: `examples/edge-dmz`.

## Aufbau

- `router` an `dmz` und `lan` mit Router-Adresse `.2`
- `web` und `docker` als Linked Clones im DMZ-Netz
- DNS: Guests über Bridge `.0`, Host über `127.0.0.1:15353`
- Forwarding default deny; Policy erlaubt u. a. TCP/5432 DMZ→LAN
- v0.2: Local CA, Caddy Ingress, `oidc-simple`

## Befehle

```bash
vzctl validate -C examples/edge-dmz
vzctl image pull ubuntu-latest
vzctl up -C examples/edge-dmz
vzctl docker -- ps
vzctl port list
vzctl down -C examples/edge-dmz
```

Ingress-Beispiel nach Vendor-Setup:

- `https://web.svc.edge-dmz.vz.test` → VM `web:80`
- `https://auth.svc.edge-dmz.vz.test` → oidc-simple

Vorher:

```bash
make vendor && make install-vendor
vzctl certs ca init
```

Cloud-Init im Beispiel enthält keine Zugangsdaten — Identity und Keys setzt vzctl pro Clone.
