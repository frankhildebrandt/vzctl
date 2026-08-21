# hypernetwork.config.yaml

`apiVersion: hypernetwork/v1`, `kind: Environment`. Unknown fields are rejected.
Export the schema with `vzctl validate --schema`.

## Required skeleton

```yaml
apiVersion: hypernetwork/v1
kind: Environment
metadata:
  name: demo                    # 1–63 [A-Za-z0-9][A-Za-z0-9._-]*
spec:
  project: demo                 # same charset as name
  domain: demo.vz.test          # must end with .vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: 127.0.0.1:15353
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base:
      from: ubuntu-latest       # pull alias
      role: base
      tag: v1                   # 1–64 [A-Za-z0-9][A-Za-z0-9._-]*
  networks:
    lan:
      cidr: 10.90.0.0/24
      mode: shared              # shared | host
      dhcp: false               # keep false (default)
      natEgress: true           # false = host-only; internet only via router
      backend: vmnet            # vmnet (default) | docker
  routes: []                    # must be a list, not {}
  policies: []                  # must be a list, not {}
  vms:
    web:
      from: ubuntu-base         # image key under spec.images
      disk: 8G                  # root capacity (also accepts 8Gi, 8192MiB)
      networks:
        - { name: lan, ip: 10.90.0.10 }
```

`dns`, `images` (≥1), `networks` (≥1), `routes`, `policies`, and `vms` (≥1)
are required. Empty `routes`/`policies` must be `[]`.

## Images

| Field | Notes |
|---|---|
| `from` | Pull alias: `ubuntu-latest`, `ubuntu-26.04`, `ubuntu-24.04`, `ubuntu-22.04`, `ubuntu-20.04`, `debian-latest`, `debian-13`, `debian-12`, `debian-11`, `alpine-latest`, `arch-latest`, `fedora-latest`, `rocky-latest`, `alma-latest`, `opensuse-latest`, `fedora-coreos-latest` (`coreos-latest`), `flatcar-latest`, `photon-latest`, `opensuse-microos-latest`, `talos-latest` |
| `role` | only `base` |
| `tag` | pins `sealed/<canonical>@<tag>.raw`; apply skips bake/seal when that tag is already sealed |

VMs share the sealed base as an APFS linked clone. Identity (machine-id, MAC,
SSH host keys, cloud-init instance-id) is reset per clone. Never open the
base writable.

## VMs

| Field | Required | Notes |
|---|---|---|
| `from` | yes | image key |
| `disk` | yes | usable root size; `dataDisk` is a legacy alias |
| `networks[]` | yes | each `{ name, ip }`; name must exist |
| `clone` | no | only `linked` |
| `cpus` | no | positive int; default 2 |
| `memory` | no | ≥256 MiB; `2048`, `2048MiB`, `2Gi` |
| `cloudInit` | no | relative path; merged with system NoCloud |
| `dependsOn` | no | known VMs; must be a DAG |
| `roles` | no | `router` and/or `docker` only |
| `requires` | no | e.g. `[oidc]` when ingress/OIDC protect the VM |
| `ports` | no | `"8080:80"` or `"127.0.0.1:8080:80"` |
| `mounts` | no | `{ source: <volume>, target: /abs, readOnly?: false }` |
| `composeFiles` | docker role | compose files relative to the YAML |
| `containers` | docker role | ensure-only map; see below |

Stack-level `spec.ports`: `"8080:web:80"` or `"127.0.0.1:8080:web:80"`.

## Networks, routes, policies

- CIDR must be canonical IPv4. Overlaps with host/VPN routes are runtime
  diagnostics, not silent YAML rewrites.
- `backend: docker` is a logical subnet on a Docker+router VM (`docker0` bip
  `.2`, no vmnet): `natEgress: false`, no DHCP, exactly one attached VM with
  `roles: [docker, router]` at `.2`, and that VM still needs a vmnet NIC.
- `routes[]`: `{ name, from, to, via }` — `via` is a router VM attached to both
  nets.
- `policies[]`: `{ name, network, forward: deny-all, allow: [{ to, proto, ports }], via? }`.
  `to` is a net name or `internet`. `to: internet` needs a router on the source
  net. `proto`: `tcp` | `udp` | `icmp`. Set `via` when several routers match.

## Volumes / virtiofs

```yaml
spec:
  volumes:
    app: ./share                # relative to the YAML, or absolute
  vms:
    web:
      mounts:
        - { source: app, target: /srv/app }
```

Volume names: 1–36 `[A-Za-z0-9][A-Za-z0-9_-]*`. Tag `vzctl` is reserved.
Targets are absolute and unique per VM.

## Docker containers (docker-role VM)

```yaml
vms:
  docker:
    roles: [docker]
    composeFiles: [compose.yaml]
    containers:
      redis:
        image: redis:7-alpine
        ports: ["6379:6379"]    # Docker -p, not host forwards
        env: { REDIS_PASSWORD: dev }
        volumes: ["./data/redis:/data"]
        restart: unless-stopped
        command: ["redis-server", "--appendonly", "yes"]
```

`containers` is ensure-only (label `vzctl.dev/managed`). Missing/changed
containers are recreated; manually started ones are not pruned.

Docker-role also creates SSH user `vzctl` and DNS name
`docker.svc.{project}.vz.test`.

## Ingress / CA / OIDC (optional)

```yaml
certs: { enabled: true, onRotate: reinject }   # reinject | reboot
ingress:
  enabled: true
  bind: "127.0.0.1"
  hostAliases: true
  redirectHttp: true
  routes:
    - { host: web.svc.demo.vz.test, to: "web:80", requires: [oidc] }
    - { host: auth.svc.demo.vz.test, to: "oidc:5556" }
oidc:
  enabled: true
  mode: oidc-simple
  issuer: https://auth.svc.demo.vz.test
  listen: "127.0.0.1:5556"
  clients: auto
  users:
    - { username: alice, email: alice@dev.local, role: admin }
```

- `ingress.routes[].to`: `vm:port` or `oidc:<port>`
- `oidc.issuer` host must be `auth.svc.{domain}`
- `*.localhost` is not a canonical route host (only a host alias)
- `oidc-simple`: `users` required; no `passwordFile` / `uplink`
- Dex (`mode: embedded`): secrets only as file refs, never inline

## Resilience (optional)

```yaml
resilience:
  network:
    egressProbe: { enabled: true, url: https://captive.apple.com/ }
    restartVMsOnStuckEgress: false
```

## Observability (optional)

```yaml
observability:
  probes:
    - { name: router-ssh, from: web, target: "router.lan.lab.vz.test:22", expect: tcp }
    - { name: host-ingress, from: host, target: "https://web.svc.lab.vz.test/", expect: http_2xx }
```

`from` is `host` or a VM key. `expect` is `tcp`, `http_2xx`, or `dns`.
`validate` rejects unknown VM refs and credentials in targets.
`stack status` runs these probes.
