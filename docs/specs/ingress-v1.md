# Ingress v1 (Caddy)

Status: v0.2<br>
Issues: [#44](https://github.com/frankhildebrandt/vzctl/issues/44),
[#43](https://github.com/frankhildebrandt/vzctl/issues/43)

## Scope

Embedded Caddy reverse proxy on Host loopback. Config comes from
`spec.ingress` in `hypernetwork/v1`. TLS leafs are minted by the Local CA
([certs-v1.md](certs-v1.md)), not Caddy `tls internal`.

## Bind

- Host: `127.0.0.1:80` / `:443` (ports configurable via `httpPort` / `httpsPort`)
- **One Caddy process** serves every project that still uses those public ports.
  `vz-edge` merges site blocks into one Caddyfile (`caddy-shared`) so SNI on
  `:443` can distinguish `*.svc.{project}.vz.test`. Separate `caddy-{project}`
  children exist only when `httpPort`/`httpsPort` are unprivileged (`≥ 1024`)
  and therefore skip the `+18000` loopback mapping.
- Caddy `reverse_proxy` upstreams use a **vmnet** attachment IP. Docker-bridge
  IPs (`backend: docker`) are skipped unless the VM has no vmnet address — the
  Mac cannot reach docker0.
- Guests reach the same Caddy via Split-Horizon DNS + Host Gateway Proxy:
  - Host DNS (`127.0.0.1:15353`): Ingress hosts → `127.0.0.1`
  - Guest DNS (bridge `.0:53`): Ingress hosts → genau die vmnet host-service
    `.1` des Listener-Netzes
    (not `.0` — guest TCP to `.0` is blackholed on macOS vmnet; docker-backend
    CIDRs are omitted)
    - Dies gilt unabhängig vom Backend-Port, auch für Docker-Published-Ports
      und Kubernetes-NodePorts. Caddy behält die numerische Backend-IP und den
      Backend-Port; der öffentliche DNS-Name wird nie als Upstream verwendet.
      Ohne aktives Gateway entsteht kein Guest-Override auf VM-IP oder Loopback.
  - Supervisor `HostGatewayIngressProxy`: public `:80/:443` on vmnet `.1` **and**
    `ingress.bind` (default `127.0.0.1`) → Caddy on unprivileged loopback ports
    (`http_port+18000` / `https_port+18000`, e.g. `18080`/`18443`)
  - Privileged TCP: `vz-dns-bind` aliases `.1` on the bridge if needed, listens,
    and streams accepted client FDs to the supervisor. Guest-UDP `.0:53` wird im
    PF-Anchor auf den exklusiven `vz-edge`-Port `.0:15054` umgeleitet, damit der
    macOS-Wildcard-DNS keine Host-Antwort übernehmen kann.
  - Der Helper legt `.1` für jedes aktive vmnet an. Ein PF-Anchor
    `com.apple/vzctl` erlaubt auf `.1` nur die konfigurierten Ingress-Ports;
    andere Host-Dienste sowie UDP/ICMP bleiben gesperrt.
  - Projektfremde Ingress-Namen liefern am Guest-Listener `NXDOMAIN`.
    `backend: docker` nutzt die `.1` der primären vmnet-NIC der owning Docker-VM.
  - `ingress.ensure` übergibt je vmnet `gateway_bindings` mit kanonischem CIDR
    und erlaubten Quell-CIDRs; Docker-CIDRs werden nur der Primary-NIC zugeordnet.
  - Caddy itself binds only `ingress.bind` on those unprivileged ports so the
    user-level process does not need root for `:80/:443`
- Direct guest→VM traffic uses `{vm}.{net}.{project}.vz.test`, not `*.svc`
- `svc` is the reserved ingress namespace and is invalid as a VM, container or
  network name.

## Routes

```yaml
ingress:
  enabled: true
  bind: "127.0.0.1"
  hostAliases: true
  redirectHttp: true
  routes:
    - host: web.svc.edge-dmz.vz.test
      to: web:80
    - host: auth.svc.edge-dmz.vz.test
      to: oidc:5556
```

- `to: vm:port` → Guest IP from attachments
- `to: oidc:port` → `127.0.0.1:<port>` (Dex)
- Route `host` must be a `*.svc.{domain}` / service name — **never** `*.localhost`

## Host aliases

When `hostAliases: true`, Caddy also serves `{short}.localhost` for the same
upstream (e.g. `web.localhost`). Guests must not use `*.localhost`.

## Lifecycle

The control plane persists each project's ingress intent and reconciles a
**global** generation. `vz-edge` concatenates those Caddyfiles (one global
block, then every site block) when backend ports match. Caddyfile validation
happens before replacement; a failed generation does not replace the
last-good edge manifest. WebSocket / gRPC pass-through uses Caddy defaults.
