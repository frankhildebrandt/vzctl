# Docker Context (Alpha)

Stack-VMs mit `roles: [docker]` bekommen eine Docker-Engine und einen SSH-basierten
Host-Context. TCP `2375` ohne SSH ist Non-Goal.

Bei Create wächst die Root-Disk auf mindestens **8 GiB** (sealed Ubuntu-Roots sind
~3.5 GiB und reichen nicht für `docker.io`). Cloud-Init mountet zuerst die
Data-Disk unter `/var/lib/docker` und legt Apt-Caches dorthin. User `vzctl`
(SSH-Context) und `vzctl-agent` (`vm exec`) landen in Gruppe `docker`.

## Flow

1. `vzctl up` / `apply` erzeugt bei `roles: [docker]` ein ed25519-Keypair unter
   `~/Library/Application Support/vzctl/projects/{project}/docker/`.
2. NoCloud-Seed injiziert User `vzctl` (Gruppe `docker`) + `authorized_keys`.
3. Attachment-Label `vzctl.dev/dns-services=docker` → DNS
   `docker.svc.{project}.vz.test` (nur auf vmnet-Attachments).
4. Apply-Step `ensure_docker_context` legt Context `vzctl-{project}` an:
   `ssh://vzctl@docker.svc.{project}.vz.test`.
   SSH-Pfade mit Leerzeichen werden gequotet; `~/.ssh/config` bekommt ein
   `Include` auf die vzctl-`ssh_config` (Docker Desktop ignoriert oft
   `DOCKER_SSH_COMMAND`). Bei VM-Recreate werden stale Host-Keys entfernt.
5. Apply-Step `ensure_containers` (danach): pro Docker-VM `composeFiles` via
   `docker compose up -d` und deklarative `containers` ensure-only
   (Labels `vzctl.dev/managed`, `vzctl.dev/vm`, `vzctl.dev/hash`).
6. `down --purge` entfernt den Context.

## Deklarative Container

Unter `spec.vms.<docker-vm>`:

```yaml
vms:
  docker:
    roles: [docker]
    composeFiles:
      - compose.yaml
      - apps/api/compose.yaml
    containers:
      redis:
        image: redis:7-alpine
        ports: ["6379:6379"]
        env: { REDIS_PASSWORD: dev }
        volumes: ["./data/redis:/data"]
        restart: unless-stopped
```

- Mehrere Compose-Files sind erlaubt (je eigenes Compose-Projekt `{vm}-{stem}`).
- Ensure-only: fehlende/geänderte managed Container anlegen/recreaten; manuell
  gestartete Container bleiben. Kein Prune.
- Bind-Mounts nutzen den Project-Mount (Host-Pfad = Guest-Pfad).

## `backend: docker` (Hypernetwork)

Ein Netz mit `backend: docker` ist kein vmnet, sondern das Docker-Bridge-Subnetz
der Owner-VM (`roles: [docker, router]`, Attachment-IP `.2` = `bip`):

```yaml
networks:
  containers:
    cidr: 10.95.0.0/24
    mode: shared
    natEgress: false
    backend: docker
vms:
  docker:
    roles: [docker, router]
    networks:
      - { name: lan, ip: 10.90.0.10 }
      - { name: containers, ip: 10.95.0.2 }
```

vzctl schreibt `/etc/docker/daemon.json` mit `"bip": "<.2>/<prefix>"` und
`"iptables": false`. Forward/Policies laufen über die Router-nftables der
Docker-VM; Peer-Router bekommen Static Routes zum Container-CIDR.

Image-Pulls nutzen die Parent-NIC (z. B. `lan` → Stack-Router → `natEgress`).

## Project path (virtiofs)

Apply mountet den Stack-Ordner (Verzeichnis von `hypernetwork.config.yaml`)
**1:1** in jede VM mit `roles: [docker]` — gleicher absoluter Host-Pfad im Guest
(Share-Tag `project`). Damit funktionieren Container-Binds unter dem Projektroot:

```bash
# Host: /Users/me/proj/edge-net/app → gleicher Pfad in der Docker-VM
vzctl docker --project edge-dmz -- run --rm -v /Users/me/proj/edge-net/app:/app alpine ls /app
```

Volume-Name `project` ist dafür systemseitig belegt; eigene Volumes sollten
einen anderen Namen nutzen. Der Guest-Bind liegt in der Init-Mount-Namespace
(sichtbar für die Docker-Engine).

## CLI

```bash
vzctl docker --project edge-dmz ps --all
vzctl docker --project edge-dmz inspect <id>
vzctl docker --project edge-dmz start|stop|restart <id>
vzctl docker --project edge-dmz run --image nginx:alpine --name web -p 8080:80
# `--project` / `--format` dürfen auch nach dem Verb stehen:
vzctl docker run --project edge-dmz --image nginx:alpine --format json
vzctl docker -- ps
vzctl docker --project edge-dmz -- compose version
```

Strukturierte Verben (`ps`, `inspect`, `start`, `stop`, `restart`, `run`) liefern
mit `--format json` ein Envelope für CLI/UI. `run` ist immer detached (`-d`).

Passthrough (`--` oder unbekannte Args) setzt den Context und reicht Args an das
lokale `docker`-Binary durch. `DOCKER_SSH_COMMAND` nutzt die vzctl-SSH-Config
(`IdentityFile`, `accept-new`).

## cloudInit

`spec.vms.*.cloudInit` wird mit dem System-Seed gemerged (System-Skalare gewinnen;
Listen werden angehängt). Beispiel: [`examples/edge-dmz/cloud-init/docker.yaml`](../examples/edge-dmz/cloud-init/docker.yaml).

## doctor

`doctor` prüft optional vorhandene Contexts (WARN bei Unreachable / fehlendem
`docker`-CLI).
