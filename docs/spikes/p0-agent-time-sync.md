# P0 Guest-Agent Time-Sync

Issue: [#16](https://github.com/frankhildebrandt/vzctl/issues/16)  
Protocol: [`guest-agent-v1.md`](../specs/guest-agent-v1.md)  
G0 procedure: [`g0-network.md` § Sleep](g0-network.md#sleep-nicht-automatisiert)

## Result

The agent advertises `time_hint` and returns the pre-correction guest time,
signed offset and action. The default 1-second threshold is configurable.
Above it, Linux `CLOCK_REALTIME` is stepped directly.

The service stays on the dedicated `vzctl-agent` account and receives only
`CAP_SYS_TIME`. There is no shell command, root account, polkit rule or access
to a broader time daemon. `--time-hint-dry-run` exercises the complete
measurement/decision path without writing the clock.

The helper:

- sends `reason=handshake` after VM/agent startup when `agent.token` exists;
- samples host wall time every second and treats a gap of at least 5 seconds as
  host sleep/wake, then reconnects and sends `reason=wake`;
- supports one-shot manual validation with
  `vz-helper agent-smoke ... --time-hint wake`;
- prints a sanitized `vm.clock_checked`/`vm.clock_corrected` line and reports a
  stepped correction to the Supervisor as `vm.clock_corrected`.

No request, token or secret is logged.

## Deterministic baseline

The unit baseline fixes host and guest timestamps and verifies:

| Host − guest | Mode | Expected / observed action |
|---:|---|---|
| +500 ms | normal, threshold 1,000 ms | `none` |
| +10,000 ms | normal, threshold 1,000 ms | `stepped` to host timestamp |
| +10,000 ms | dry-run, threshold 1,000 ms | `skipped`, no clock write |
| invalid reason | any | protocol error |

The helper client test verifies the signed `offset_ms`, `stepped` action and
`reason=wake`; the wake-detector test uses a 300-second host-time gap.

## Manual 5–10 minute sleep acceptance

A fresh Base Raw is required; none is stored in the repository. Once the
ARM64-Linux builder artifact is available:

```bash
./scripts/smoke-helper-agent-e2e.sh \
  artifacts/ubuntu-24.04-vzctl-base.raw
```

For the non-destructive sleep check, start the normal helper (`run`) with the
same VM bundle, wait for `reason=handshake`, sleep the Mac for 5–10 minutes,
then inspect the helper output:

```text
event=vm.clock_corrected reason=wake observed_guest_unix_ms=... offset_ms=... action=stepped
```

If the measured absolute offset is at most 1 second, the valid result is
`event=vm.clock_checked ... action=none`. To force a one-shot post-wake probe:

```bash
swift run --package-path daemon vz-helper agent-smoke \
  --vm-id time-sync --bundle /path/to/vm-bundle --time-hint wake
```

Record host/guest before sleep, sleep duration, returned `offset_ms`, `action`
and a post-correction host/guest comparison here. The live measurement remains
an ops residual until the Base Raw exists; deterministic code paths are green.

## Verification

```bash
cd guest-agent && go test ./...
cd daemon && swift test
GOCACHE=/tmp/vzctl-go-build-cache \
  CGO_ENABLED=0 GOOS=linux GOARCH=arm64 \
  go build ./guest-agent/cmd/vzctl-agent
```
