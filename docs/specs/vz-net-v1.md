# vz-net Protocol v1

Status: Alpha<br>
Architecture: [ADR 0002](../adr/0002-process-ownership.md)

## Scope

Unix-socket JSON-RPC between the control-plane supervisor (`vz-supervisor`) and
the HyperNetwork Supervisor (`vz-net`). `vz-net` owns every live
`vmnet_network_ref` and host bridge interface. Desired-state metadata stays in
the control-plane SQLite database.

## Transport

- Default socket: `$VZCTL_STATE_DIR/net.sock` (mode `0600`).
- Framing: one JSON object per line, newline-terminated (same as `vz.sock`).
- No auth beyond filesystem permissions on the state directory (`0700`) and socket.

## Process model

| Binary | LaunchAgent | Role |
|---|---|---|
| `vz-net` | `com.vzctl.net` | Hold refs + host bridges; KeepAlive |
| `vz-supervisor` | `com.vzctl.supervisor` | Desired state, DNS, apply, helper RPC |

Install starts `com.vzctl.net` before `com.vzctl.supervisor`.

## Methods

### `health`

Params: none or `{}`.

Result:

```json
{ "ok": true, "version": "0.0.1-alpha", "networks": 2 }
```

### `net.acquire`

Reserve (or re-attach to) a named shared/host vmnet network and ensure the host
bridge is up so gateway `.0` is bindable.

Params:

```json
{
  "name": "dmz",
  "cidr": "10.80.0.0/24",
  "mode": "shared",
  "nat_egress": true
}
```

- `mode` must be `"shared"` in v1.
- `nat_egress: true` → `VMNET_SHARED_MODE`; `false` → `VMNET_HOST_MODE`.
- **Idempotent:** same `name` + same `cidr`/`mode`/`nat_egress` returns success
  without creating a second reservation.
- Conflict if `name` exists with different config, or another name already holds
  the same CIDR.

Result:

```json
{
  "name": "dmz",
  "cidr": "10.80.0.0/24",
  "mode": "shared",
  "nat_egress": true,
  "gateway": "10.80.0.0"
}
```

Errors:

| Code | When |
|---|---|
| `-32602` | Invalid params / unsupported mode |
| `-32031` | Conflict (name or CIDR) |
| `-32032` | Runtime reserve failed (includes Apple status). Only when the failure is consistent with an unclean prior exit should the message mention orphaned-until-reboot. |

### `net.release`

Drop the named reservation (stop interface + CFRelease).

Params: `{ "name": "dmz" }`

Result: `{ "released": true, "name": "dmz" }`

`not found` is an error (`-32031`).

### `net.list`

Params: none or `{}`.

Result:

```json
{
  "networks": [
    {
      "name": "dmz",
      "cidr": "10.80.0.0/24",
      "mode": "shared",
      "nat_egress": true,
      "gateway": "10.80.0.0"
    }
  ]
}
```

### `net.serialize`

Copy-serialize a live network for helper attach
(`vmnet_network_copy_serialization`).

Params: `{ "name": "dmz" }`

Result:

```json
{
  "name": "dmz",
  "serialization": "<base64>"
}
```

The blob is only valid while `vz-net` still holds the original ref.

## Lifecycle rules

1. Control-plane **must not** release refs on its own shutdown or crash.
2. Control-plane `net.delete` / stack teardown calls `net.release`.
3. Clean `vz-net` SIGTERM releases every held ref (CIDRs become free).
4. Unclean `vz-net` exit may orphan CIDRs until host reboot (G0); fresh CIDRs
   still work.

## Out of scope

- Helper talking to `net.sock` directly
- DNS zone build / UDP listeners
- Desired-state persistence
- Bridged mode
