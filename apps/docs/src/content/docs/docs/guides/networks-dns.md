---
title: Netze & DNS
description: vmnet-Attachments, IP-Konvention und vz-edge DNS.
---

## Netze

Desired-State-Netze sind typischerweise `mode: shared` (vmnet). Bridged Mode ist unsupported.

IP-Konvention auf Custom-vmnet:

| Adresse | Rolle |
| --- | --- |
| `.0` | Host-Gateway / Guest-DNS |
| `.1` | geschützter Host-/Ingress-Alias |
| `.2` | Router-VM |
| `.10+` | Guests |

Cross-Net-Traffic läuft nur über eine Router-VM und Policies.

CLI (imperativ, parallel zum deklarativen Stack):

```bash
vzctl net create dmz --cidr 10.80.0.0/24 --mode shared --project demo
vzctl net attach web --network dmz --ip 10.80.0.10
vzctl net list
```

Live-Refs hält `vz-net`. Helper bekommen Attachment-Handles serialisiert vom Control Plane.

## DNS

`vz-edge` liefert die Zone:

```text
{vm}.{net}.{project}.vz.test
```

| Listener | Default | Zweck |
| --- | --- | --- |
| Host | `127.0.0.1:15353` | macOS via `/etc/resolver` |
| Guest | Bridge `.0:53` | Nameserver in VMs |

Port 53 und Ingress 80/443 brauchen den Root-LaunchDaemon `vz-dns-bind`
(SCM_RIGHTS):

```bash
sudo vzctl dns install-bind-helper
vzctl dns install-resolver
vzctl dns status
vzctl dns query web.lan.demo.vz.test
```

Ohne Bind-Helper: Guest-DNS/`dns_ok=false` und typisch `Permission denied` auf Port 53.

Services (Ingress/Docker) erscheinen unter `*.svc.{project}.vz.test`.
