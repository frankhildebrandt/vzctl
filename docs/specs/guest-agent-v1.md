# Guest-Agent Protocol v1

Status: Alpha, wire contract frozen for protocol version `1`<br>
Issue: [#13](https://github.com/frankhildebrandt/vzctl/issues/13)<br>
Architecture: [ADR 0002](../adr/0002-process-ownership.md)

## Scope

This document specifies the control-plane protocol between one macOS VM helper
and the `vzctl-agent` inside its guest. It covers transport, framing,
authentication, commands, deadlines, cancellation and stable errors.

The agent binary and image integration are #14. Helper↔agent E2E wiring is #15.
Clock correction and helper wake wiring are #16.

## Ownership and transport

- The per-VM **helper owns the vsock client** and the
  `VZVirtioSocketDevice` associated with its `VZVirtualMachine`, as required by
  ADR 0002.
- The guest agent listens on virtio-vsock. The Alpha default port is `21950`;
  an override, if used, is part of the per-VM configuration and must match on
  both sides.
- This protocol is never exposed on TCP, UDP or a Unix socket. SSH is only a
  diagnostic fallback outside this protocol.
- One connection may carry multiple requests. Request IDs make responses
  correlatable; responses may arrive out of order.
- The supervisor never opens an agent connection and never receives the auth
  token. It sees agent state only through sanitized helper reports; see
  [Supervisor visibility](#supervisor-visibility).

## Framing

Each message is exactly one frame:

```text
+----------------------+------------------------------+
| length: u32 little   | payload: length bytes        |
| endian, 4 bytes      | UTF-8 encoded JSON object    |
+----------------------+------------------------------+
```

- `length` is the payload byte count and does not include the four-byte prefix.
- The receiver reads exactly four bytes, decodes an unsigned 32-bit
  little-endian integer, then reads exactly `length` bytes.
- The payload must be valid UTF-8 and exactly one JSON object. A UTF-8 BOM,
  trailing bytes and multiple JSON values are invalid.
- Empty frames and frames larger than `1,048,576` bytes are invalid.
- A short prefix/payload at EOF is a protocol error. If an `id` can be decoded,
  the receiver returns `proto`; otherwise it closes the connection.
- There is no delimiter, compression or binary attachment in v1. Binary input
  is base64 in JSON and counts toward the frame limit.

## Message envelope

Every request after connection setup has this shape:

```json
{
  "v": 1,
  "id": "01K1B8JX6B6R1R4CFA9G2X3TQY",
  "method": "ping",
  "params": {}
}
```

- `v` must be integer `1`.
- `id` is a non-empty string of at most 128 UTF-8 bytes and must be unique
  among in-flight requests on the connection.
- `method` is a case-sensitive ASCII string.
- `params` is a JSON object. Unknown fields are ignored unless they change
  security semantics; wrong types produce `proto`.

A successful response is:

```json
{
  "v": 1,
  "id": "01K1B8JX6B6R1R4CFA9G2X3TQY",
  "ok": true,
  "result": {"pong": true}
}
```

An error response is:

```json
{
  "v": 1,
  "id": "01K1B8JX6B6R1R4CFA9G2X3TQY",
  "ok": false,
  "error": {
    "code": "unsupported",
    "message": "method is not supported",
    "details": {"method": "logs"}
  }
}
```

Exactly one of `result` or `error` is present. `message` is diagnostic and not
stable API; clients branch only on `error.code`. `details` is optional.

## Version handshake and authentication

The first frame on every new connection must be `hello`; no other command is
accepted before it succeeds:

```json
{
  "v": 1,
  "id": "hello-1",
  "method": "hello",
  "params": {
    "token": "base64url-without-padding",
    "helper_version": "0.1.0"
  }
}
```

Success:

```json
{
  "v": 1,
  "id": "hello-1",
  "ok": true,
  "result": {
    "v": 1,
    "agent_version": "0.1.0",
    "capabilities": ["ping", "version", "exec", "report_ip", "health", "time_hint", "network_probe"]
  }
}
```

Rules:

1. The token is unique per VM and contains at least 256 random bits. The
   canonical encoding is unpadded base64url.
2. Provision it through the VM's local NoCloud seed, preferably `user-data`
   `write_files`, into a file readable only by the agent service account. It
   must never be committed to Git, embedded in a sealed base or logged.
3. The host-side copy is stored in the per-VM private state/bundle with mode
   `0600` or stricter. It is transmitted to the agent only in `hello` over this
   VM's vsock connection.
4. The agent compares tokens in constant time. On mismatch it sends `auth`
   when possible and immediately closes the connection. It applies a bounded
   retry delay; the helper does not retry in a tight loop.
5. Unsupported `v` returns `proto` with `details.supported_versions: [1]` and
   closes the connection.
6. Until `hello` succeeds, all non-`hello` requests return `auth` and the
   connection closes.

Token rotation is stop-and-reprovision in Alpha: stop the VM, generate a new
token, replace both private host state and the NoCloud guest file, then boot.
Any mismatch aborts all pending agent work and marks the helper state
`auth_failed`; there is no silent fallback to the old token. Live rotation and
a `rotate_token` method are not part of v1.

## Alpha methods

`hello` and `cancel` are protocol-control methods. The command methods are:

| Method | Params | Result | Default / maximum |
|---|---|---|---|
| `ping` | optional `nonce` string | `pong: true`, optional echoed `nonce` | 1 s / 5 s |
| `version` | `{}` | `agent_version`, `v`, `capabilities` | 1 s / 5 s |
| `exec` | `cmd`, optional `cwd`, `env`, `stdin_b64`, `timeout_ms`, `tty`, `cols`, `rows` | one-shot: `exit`/`stdout`/`stderr`/`truncated`; tty: `upgraded: true` then mux | 30 s / 600 s (one-shot); tty until exit/disconnect |
| `report_ip` | `{}` | `interfaces` array | 2 s / 10 s |
| `health` | `{}` | `status` (`ok`\|`degraded`\|`down`), `uptime_ms`, `queue_depth`, `p99_exec_ms`, `checks`, optional `last_error` | 2 s / 10 s |
| `time_hint` | `host_unix_ms`, `reason` | `observed_guest_unix_ms`, `offset_ms`, `action` | 2 s / 5 s |
| `fs.mount` | `name`, `target`, optional `read_only` | `mounted: true`, `name`, `target` | 10 s / 30 s |
| `fs.unmount` | `name` and/or `target` | `mounted: false`, `name`, `target` | 10 s / 30 s |
| `ca_inject` | `pem`, `fingerprint`, optional `name` | `installed: true`, `fingerprint` | 15 s / 60 s |
| `network_probe` | `url` **or** `target` (+ optional `via`, `connect_ip`, `timeout_ms`) | URL: `classification`/`phase`/…; target: `resolved_ips`, `chosen_ip`, `connect_ms`, `error_stage`, `dns`/`ip` legs | 5 s / 30 s |
| `stats` | `{}` | `cpu.percent`, `memory.{used_mib,total_mib,percent}`, `disk.{read_iops,write_iops}`, `load1`, `mem_used_pct`, optional `top_process` | 2 s / 10 s |
| `services.list` | `{}` | `services[]` with `name`, `kind`, `url`, `pid` | 2 s / 10 s |
| `services.http` | `name`, `path`, optional `method`, `headers`, `body_b64` | `status`, `headers`, `body_b64`, `truncated` | 15 s / 30 s |
| `services.stream` | same as `services.http` | `upgraded: true` then mux stdout of the upstream body | until disconnect |

Capability `fs_mount` advertises the virtiofs bind helpers. The agent invokes
`/usr/local/lib/vzctl/virtiofs-bind` via `sudo -n` (installed by system
NoCloud); it does **not** hold `CAP_SYS_ADMIN` itself. See
[virtiofs-v1.md](virtiofs-v1.md).

Capability `ca_inject` installs the Local CA PEM into the guest system store
(see [certs-v1.md](certs-v1.md)): write
`/usr/local/share/ca-certificates/{name}.crt` (default name `vzctl-local`) and
run the restricted `/usr/local/lib/vzctl/ca-inject` helper via `sudo -n`. The
helper independently validates the DER sha256 fingerprint, performs an atomic
install, runs `update-ca-certificates` and verifies the result against the
system CA bundle. `fingerprint` is the sha256 hex of the DER the host expects;
mismatch before or after install is an error.

All time values are integer milliseconds. A caller may choose a shorter
deadline. Values above the maximum return `proto`; zero and negative values are
invalid.

### `stats`

Capability `stats` samples guest CPU, memory and disk IOPS from `/proc`.
The agent keeps the previous `/proc/stat` and `/proc/diskstats` snapshot;
`cpu.percent` and `disk.*_iops` are deltas and `null` on the first call.
Memory comes from `MemTotal`/`MemAvailable`. IOPS sum whole disks (`vda`,
`nvme0n1`) and skip partitions plus loop/ram/zram devices.

```json
{
  "cpu": { "percent": 12.4 },
  "memory": { "used_mib": 512, "total_mib": 1024, "percent": 50.0 },
  "disk": { "read_iops": 3.2, "write_iops": 1.1 }
}
```

Older agents without the capability return `unsupported`. Hosts should show
n/a until `vzctl vm agent upgrade`.

### Guest publish (`guest_publish`)

Capability `guest_publish` lets guest processes advertise a **loopback HTTP
API** under a DNS-label name. The agent listens on Unix socket
`/run/vzctl/guest.sock` (mode `0660`):

- `PUT /v1/services/{name}` body `{kind, url, pid?}` — `url` must be
  `http(s)://127.0.0.1|localhost|::1:<port>`
- `DELETE /v1/services/{name}`
- `GET /v1/services`

`iwatch --listen --name app` registers `kind: iwatch`. Dead PIDs are reaped on
list. Hosts proxy through vsock `services.list`, `services.http` (256 KiB body
cap) and `services.stream` (mux stdout after `{upgraded:true}`, like
`exec_tty`). Paths must be root-relative (`/api/...`) and stay on the
registered origin.

### `network_probe`

Die optionale Capability trennt DNS-, TCP-, TLS- und HTTP-Phase.

Zwei Parameter-Modi (genau einer Pflicht):

- **HTTP** (`url`): klassifiziert `online`, `captive` oder `offline`. Redirects
  werden nicht verfolgt. Die URL muss HTTP(S) sein und darf keine Credentials
  enthalten. Logs und Fehler enthalten weder URL/Query noch öffentliche IP oder
  Secrets.
- **Connect** (`target` = `host:port`): Guest-DNS plus TCP-Connect. `via` ist
  `dns`, `ip` oder `both` (Default `both`). Bei `via=ip`/`both` darf der Host
  `connect_ip` setzen (vom Host-Resolver aufgelöst), damit Guest-DNS-Ausfälle
  den TCP-Pfad nicht blockieren. Antwort enthält `resolved_ips`, `chosen_ip`,
  `connect_ms`, `error_stage` (`dns`|`tcp`|`timeout`) und bei `both` die Legs
  `dns`/`ip` mit `ok`.

Fehlt die Capability bei einem alten Agent, gilt das Ergebnis als `unknown`
und degradiert das interne Netz nicht.

### `exec`

Request:

```json
{
  "v": 1,
  "id": "exec-42",
  "method": "exec",
  "params": {
    "cmd": ["uname", "-a"],
    "cwd": "/tmp",
    "env": {"LANG": "C.UTF-8"},
    "timeout_ms": 5000
  }
}
```

Success:

```json
{
  "v": 1,
  "id": "exec-42",
  "ok": true,
  "result": {
    "exit": 0,
    "stdout": "Linux guest 6.8.0 ...\n",
    "stderr": "",
    "truncated": false
  }
}
```

`cmd` is a non-empty array of strings and maps directly to executable plus
argv. The agent must not join it into a shell command. `env` augments a small,
sanitized service environment; it does not replace protected agent variables.
`stdin_b64`, when present, is capped at 256 KiB decoded.

Exit zero is success. A non-zero exit, signal, launch failure or invalid `cwd`
returns `exec_failed`; its details contain `exit` (integer or `null`), optional
`signal`, `stdout`, `stderr` and `truncated`.

Stdout and stderr are captured separately and capped at 256 KiB each. Once a
stream reaches its cap the agent continues draining it, discards excess bytes
and sets `truncated: true`, preventing a child-process pipe deadlock.

### `exec` with `tty: true` (capability `exec_tty`)

Interactive sessions negotiate a connection upgrade. The helper must open a
**fresh** vsock connection, complete `hello`, then send `exec` with:

- `tty: true` (required);
- `cmd`, optional `cwd` / `env` as in one-shot `exec`;
- optional `cols` / `rows` (positive integers; default 80×24);
- **no** `stdin_b64` (invalid with `tty`).

Agents advertise capability `exec_tty`. Without it, `tty: true` returns
`unsupported`. When `env.TERM` is omitted, the agent defaults
`TERM=xterm-256color` so ncurses tools can initialize.

Success response:

```json
{
  "v": 1,
  "id": "exec-tty-1",
  "ok": true,
  "result": { "upgraded": true }
}
```

After this response the connection carries **only** mux frames (no further
JSON-RPC). Closing the connection ends the session: the agent sends `SIGHUP`
to the PTY process group and reaps the child.

Mux frame (little-endian), max payload `1,048,576` bytes:

```text
+--------+------------------+----------------------+
| type   | length: u32 LE   | payload              |
| 1 byte | 4 bytes          | length bytes         |
+--------+------------------+----------------------+
```

| type | name | direction | payload |
|---|---|---|---|
| `0x01` | stdin | helper → agent | raw bytes |
| `0x02` | stdout | agent → helper | raw PTY master bytes |
| `0x04` | resize | helper → agent | `u16 cols`, `u16 rows` (LE) |
| `0x05` | exit | agent → helper | `i32` exit status (LE); then EOF |
| `0x06` | stdin_eof | helper → agent | empty |

Unknown types are a `proto` failure and close the connection. One-shot `exec`
without `tty` is unchanged.

### `report_ip`

```json
{
  "v": 1,
  "id": "ip-1",
  "ok": true,
  "result": {
    "interfaces": [
      {
        "name": "enp0s1",
        "mac": "02:00:00:00:00:10",
        "addresses": ["10.90.1.10/24"]
      }
    ]
  }
}
```

Only active, non-loopback interface addresses are reported. Link-local
addresses may be included and must be identifiable by their CIDR. Attachment
matching is a helper/E2E concern in #15.

G0 reserves each subnet's `.0` for the host bridge/gateway/DNS UDP listener,
`.2` for the router and `.10+` for guests. This is context for validation only:
`report_ip` does not configure networking or implement DNS/vmnet, and `.0`
must not be accepted as a guest address.

### `health`

`status` is `ok`, `degraded`, or `down`. `down` is reserved for missing/untrusted
token or handshake failure. `degraded` is set when in-flight RPCs (`queue_depth`)
are at least 2 or p99 exec latency exceeds 5s. `checks` is an object whose values
contain at least `ok: boolean` and may include a diagnostic `message`. Health must
not expose secrets, arbitrary files or process environments.

### `time_hint`

```json
{
  "v": 1,
  "id": "time-1",
  "method": "time_hint",
  "params": {
    "host_unix_ms": 1785387600000,
    "reason": "wake"
  }
}
```

`reason` is one of `handshake`, `wake` or `manual`. The agent samples its wall
clock before any correction and responds:

```json
{
  "v": 1,
  "id": "time-1",
  "ok": true,
  "result": {
    "observed_guest_unix_ms": 1785387599700,
    "offset_ms": 300,
    "action": "none"
  }
}
```

`offset_ms` is `host_unix_ms - observed_guest_unix_ms`. `action` has these
stable values:

- `none`: absolute offset is at or below the configured threshold;
- `stepped`: the agent set `CLOCK_REALTIME` to `host_unix_ms`;
- `skipped`: correction was required but the agent runs in dry-run mode.

The default threshold is 1 second and is configurable with
`--time-hint-threshold`. `--time-hint-dry-run` never changes the clock. A
failed real clock step returns `internal`; it must not be reported as
`skipped`.

The Linux service runs as the dedicated `vzctl-agent` user and receives only
`CAP_SYS_TIME`. The agent calls `clock_settime` directly: it does not invoke a
shell, `chrony`, `timedatectl` or `date -s`, and it does not receive root or
unrelated capabilities. `ProtectClock` is disabled only for this service
because systemd would otherwise block the narrowly scoped capability.

## Deadlines and cancellation

- Connect timeout: 5 seconds. Handshake timeout: 2 seconds.
- A method deadline starts after the complete request frame has been written.
  The helper owns the authoritative wall-clock deadline; `exec.timeout_ms` is
  also enforced inside the guest.
- At deadline, the helper sends `cancel` when the connection is usable and
  completes the original request locally as `timeout`. It does not wait
  indefinitely for cancellation acknowledgement.
- `cancel` params are `{"id":"<target-request-id>"}`. Success is
  `{"cancelled":true}`; an already completed or unknown ID returns
  `{"cancelled":false}`.
- On accepted cancellation the target operation is terminated, its child
  process group is stopped for `exec`, pipes are drained, and the target
  request returns `timeout` with `details.reason: "cancelled"`.
- Closing the connection cancels all its in-flight work. No operation survives
  reconnect and no response is replayed. Callers may retry only operations they
  know are safe to repeat.
- IDs may be reused only after the prior response or connection close.

Cancellation example:

```json
{"v":1,"id":"cancel-1","method":"cancel","params":{"id":"exec-42"}}
```

```json
{"v":1,"id":"cancel-1","ok":true,"result":{"cancelled":true}}
```

```json
{"v":1,"id":"exec-42","ok":false,"error":{"code":"timeout","message":"request cancelled","details":{"reason":"cancelled"}}}
```

## Stable error codes

These strings are stable for all protocol-v1 implementations:

| Code | Meaning | Retry guidance |
|---|---|---|
| `auth` | Missing/invalid token or command before handshake | Do not retry until token/config is repaired or rotated |
| `timeout` | Deadline exceeded or accepted cancellation | Retry only if the operation is safe to repeat |
| `exec_failed` | Process launch, non-zero exit or signal | Do not retry blindly; inspect structured details |
| `unsupported` | Valid request for an unknown/unavailable method | Do not retry without capability/version change |
| `proto` | Invalid framing, JSON, envelope, version or parameters | Fix client/request; connection may be closed |
| `internal` | Unexpected agent failure not attributable to the request | Bounded retry; surface diagnostics |

Implementations may add fields to `details`, but must not invent new v1 error
codes. A method-specific failure must map to the closest code above.

## Supervisor visibility

The helper reduces agent observations to a report such as:

```json
{
  "vm_id": "demo/web",
  "agent": {
    "state": "ready",
    "protocol": 1,
    "version": "0.1.0",
    "last_seen_at": "2026-07-30T07:00:00Z",
    "health": "ok"
  }
}
```

Allowed states are `connecting`, `ready`, `degraded`, `auth_failed` and
`unavailable`. IP data may be forwarded separately after `report_ip`. Tokens,
raw handshake messages, command argv, stdout and stderr are never forwarded in
state/heartbeat reports.

The supervisor treats this as indirect, ephemeral helper-owned state. A helper
disconnect makes the state unavailable/stale; the supervisor must not infer
that the guest stopped. Alpha persistence and E2E wiring remain #15.

## Guest utils rollout

The host control plane may push a **guest utils bundle** to running VMs without
rebaking the sealed base image. The bundle contains:

- `/usr/local/sbin/vzctl-agent` (cross-built ARM64 binary)
- `/usr/local/bin/iwatch` (GitHub Release `linux_arm64`, pin `guest-agent/IWATCH_VERSION`)
- `/usr/local/lib/vzctl/{virtiofs-bind,router-apply,ca-inject}`
- `/etc/sudoers.d/vzctl-{agent,virtiofs,router,ca}`
- `/etc/systemd/system/vzctl-agent.service` (and OpenRC unit when present)
- `/usr/lib/vzctl-agent/image-metadata.json`

Rollout is triggered by `vzctl apply` (`ensure_guest_utils`, after agents are
ready) and manually via `vzctl vm agent upgrade`. The host caches the bundle
under `$VZCTL_STATE_DIR/guest-utils/{bundle_id}/` where `bundle_id` is
`{agent_version}-{content_sha256_prefix}`.

After a successful rollout the guest records:

```json
{
  "bundle_id": "0.1.3-deadbeef",
  "agent_version": "0.1.3",
  "iwatch_version": "v0.1.0",
  "content_sha256": "...",
  "updated_at": "2026-08-07T12:00:00Z"
}
```

at `/var/lib/vzctl/utils.manifest.json`. VMs with a matching `bundle_id` are
skipped.

Binary transfer uses repeated `exec` calls with `stdin_b64` chunks (decoded cap
256 KiB per frame). The host verifies SHA256 on the guest before atomically
replacing the binary, backs up the previous binary to
`/usr/local/sbin/vzctl-agent.bak`, then restarts the agent (`systemctl` or
OpenRC). The control plane polls `version` until `agent_version` matches.

Rollout failures abort `apply` (aggregated per VM, same semantics as CA
rollout). There is no auto-rollout on `vm start` in v1.

## Security requirements

- No shell-string `exec`; only array argv is accepted. Shell behavior requires
  an explicit argv such as `["/bin/sh", "-c", "..."]` from an authorized
  caller and should be rejected by higher-level policy by default.
- The service runs as a dedicated unprivileged account (`vzctl-agent`), not as
  root and without `CAP_SYS_ADMIN`. Passwordless `sudo` is granted for agent
  helpers and interactive `vm exec`; the unit must not set hardening options
  that imply `no_new_privs` (they override `NoNewPrivileges=no`).
- Apply the frame, stdin and output limits before allocating unbounded buffers.
- Do not log tokens, environment values, stdin, stdout or stderr by default.
- Compare tokens in constant time and rate-limit failed handshakes.
- Treat all guest output as untrusted data when forwarding it to CLI/UI/logs.
- vsock isolation and the per-VM token are both required; neither replaces the
  other.
