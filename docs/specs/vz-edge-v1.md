# vz-edge Protocol v1

Status: Alpha  
Architecture: [ADR 0002](../adr/0002-process-ownership.md)

## Scope

`vz-edge` is the long-lived host dataplane process. It owns DNS listeners,
loopback port forwards, guest-facing ingress listeners and the Caddy, Dex and
`oidc-simple` child processes. The control-plane supervisor owns desired state,
SQLite and apply journals.

The split keeps the active host dataplane alive while `vz-supervisor` restarts.
`vz-edge` does not own `vmnet_network_ref`; those remain exclusively in
`vz-net`.

## Transport and lifecycle

- UDS: `$VZCTL_STATE_DIR/edge.sock`, mode `0600`.
- State directory and peer UID provide authentication.
- Framing: newline-delimited JSON-RPC 2.0.
- LaunchAgent: `com.vzctl.edge`, `KeepAlive=true`.
- Start order: `vz-net` → `vz-edge` → `vz-supervisor`.
- Runtime cache: `$VZCTL_STATE_DIR/runtime/edge/manifest.json`, mode `0600`.

The manifest is a recovery cache, not a second desired-state source. On an edge
restart it restores the last successfully applied generation before the control
plane reconnects. A corrupt manifest is never interpreted as an empty desired
state; health remains degraded until a fresh reconcile succeeds.

## Methods

### `edge.health` / `edge.status`

Returns `ok`, version, applied generation/digest, DNS health, active port
forwards, ingress-listener count, supervised child status and `last_error`.

### `edge.reconcile`

```json
{
  "generation": 42,
  "digest": "sha256-hex",
  "desired": {
    "network_snapshot": {"networks": [], "attachments": []},
    "host_services": [],
    "port_forwards": [],
    "ingress": [],
    "oidc": []
  }
}
```

- The snapshot is global and replaces the prior runtime generation.
- Same generation + digest is idempotent.
- Same generation + another digest or an older generation returns `-32042`.
- Parse, identity and Caddy config validation happen before replacement.
- The manifest is replaced only after a successful apply.
- On failure `vz-edge` attempts to restore the previous in-memory generation.

### `dns.lookup`

Params: `{ "name": "web.svc.edge-dmz.vz.test" }`.

Returns host- and guest-horizon addresses from the live zone.

## Child-process policy

Only these identities are accepted:

| Kind | Process name | Binary basename | Arguments |
|---|---|---|---|
| Caddy | `caddy-{project}` | `caddy` | fixed `run --config … --adapter caddyfile` |
| Dex | `dex-{project}` | `dex` | fixed `serve {config}` |
| Dev IdP | `oidc-simple-{project}` | `vzctl-oidc-simple` | fixed `--config {config}` |

Unexpected exits use bounded exponential restart backoff (1–30 seconds).
Intentional removal never restarts a child. Caddy config is validated before
the running instance is replaced. Caddy, Dex and oidc-simple must accept a TCP
connection on their configured loopback listener before the generation commits.
Configs, PID files and logs are scoped by project below `runtime/edge/`.

## Failure semantics

| Failure | Effect |
|---|---|
| `vz-supervisor` crash | Applied DNS, listeners and children remain active |
| Child crash | Only that child is restarted |
| `vz-edge` crash | launchd restarts it; last-good manifest is restored |
| `vz-net` crash | Edge remains alive, but guest bridge paths may be unavailable |
| Invalid new config | Reconcile fails; last-good manifest is retained |
