---
title: Docker
description: Docker-Role, SSH-Context und deklarative Container.
---

VMs mit `roles: [docker]` bekommen eine Docker-Engine und einen SSH-basierten
Host-Context. Offenes TCP `2375` ist Non-Goal.

## Flow

1. Apply legt ein ed25519-Keypair unter
   `~/Library/Application Support/vzctl/projects/{project}/docker/` an.
2. Cloud-Init injiziert User `vzctl` (Gruppe `docker`) inkl. `authorized_keys`.
3. Label `vzctl.dev/dns-services=docker` → `docker.svc.{project}.vz.test`.
4. Context `vzctl-{project}`: `ssh://vzctl@docker.svc.{project}.vz.test`.
5. Step `ensure_containers`: `composeFiles` und deklarative `containers`.
6. Laufende Stack-Container erscheinen als
   `{container}.{docker-net}.{project}.vz.test`.

```bash
vzctl docker -- ps
vzctl docker -- inspect <id>
```

## Deklarative Spec

```yaml
vms:
  docker:
    roles: [docker]
    composeFiles:
      - compose.yaml
    containers:
      redis:
        image: redis:7-alpine
        ports: ["6379:6379"]
```

Ensure-only: managed Container anlegen/recreaten; manuelle Container bleiben unberührt.

## `backend: docker`

Ein Netz mit `backend: docker` exponiert die Docker-Bridge als Hypernetwork-CIDR.
Owner-VM braucht `roles: [docker, router]`, Attachment-IP `.2` (= `bip`), kein vmnet.
Peer-Router bekommen Static Routes.
