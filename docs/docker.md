# Docker Context (Alpha)

Stack-VMs mit `roles: [docker]` bekommen eine Docker-Engine und einen SSH-basierten
Host-Context. TCP `2375` ohne SSH ist Non-Goal.

## Flow

1. `vzctl up` / `apply` erzeugt bei `roles: [docker]` ein ed25519-Keypair unter
   `~/Library/Application Support/vzctl/projects/{project}/docker/`.
2. NoCloud-Seed injiziert User `vzctl` (Gruppe `docker`) + `authorized_keys`.
3. Attachment-Label `vzctl.dev/dns-services=docker` → DNS
   `docker.svc.{project}.vz.test`.
4. Apply-Step `ensure_docker_context` legt Context `vzctl-{project}` an:
   `ssh://vzctl@docker.svc.{project}.vz.test`.
5. `down --purge` entfernt den Context.

## CLI

```bash
vzctl docker -- ps
vzctl docker --project edge-dmz -- compose version
```

Der Wrapper setzt den Context und reicht Args an das lokale `docker`-Binary durch.
`DOCKER_SSH_COMMAND` nutzt die vzctl-SSH-Config (`IdentityFile`, `accept-new`).

## cloudInit

`spec.vms.*.cloudInit` wird mit dem System-Seed gemerged (System-Skalare gewinnen;
Listen werden angehängt). Beispiel: [`examples/edge-dmz/cloud-init/docker.yaml`](../examples/edge-dmz/cloud-init/docker.yaml).

## doctor

`doctor` prüft optional vorhandene Contexts (WARN bei Unreachable / fehlendem
`docker`-CLI).
