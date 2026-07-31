# vzctl guest agent

`vzctl-agent` is the unprivileged Linux endpoint for the protocol in
[`docs/specs/guest-agent-v1.md`](../docs/specs/guest-agent-v1.md). The P0
P0 agent slice implements:

- little-endian length-prefixed JSON framing, capped at 1 MiB;
- first-frame `hello`, constant-time token comparison and bounded auth delay;
- `ping`, `version`, `health`, `exec` and `report_ip`;
- concurrent in-flight requests plus `cancel`, process-group termination and
  helper/agent deadlines;
- argv-only execution with 256 KiB stdin/stdout/stderr caps and truncation;
- interactive `exec` with `tty: true` (capability `exec_tty`) upgrades the
  vsock connection to length-prefixed mux frames over a Linux PTY;
- active non-loopback address reporting that rejects reserved IPv4 `.0`;
- `time_hint` measurement and thresholded `CLOCK_REALTIME` stepping.

Only implemented command methods are advertised as capabilities. `time_hint`
defaults to a 1-second threshold. The systemd service runs as `vzctl-agent` with
`CAP_SYS_TIME` plus `CAP_SETUID`/`CAP_SETGID` for passwordless `sudo` (helpers
and interactive `vm exec`). Hardening flags that imply `no_new_privs` are
omitted so setuid sudo works.

## Build and test

```bash
cd guest-agent
go test ./...

VERSION="$(tr -d '[:space:]' < VERSION)"
CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build \
  -trimpath \
  -ldflags "-s -w -buildid= -X main.version=$VERSION" \
  -o vzctl-agent \
  ./cmd/vzctl-agent
file vzctl-agent
```

The release build has no C runtime dependency and is an ARM64 Linux ELF.

For a no-write validation run, append `--time-hint-dry-run`. Override the
threshold with e.g. `--time-hint-threshold 1500ms`.

## Runtime

The image pipeline creates the system user `vzctl-agent`, installs the binary
at `/usr/local/sbin/vzctl-agent` and enables `vzctl-agent.service` plus
`vzctl-agent.path`. The service starts after `cloud-config.service` (NoCloud
`write_files` for the token), gated by `ConditionFileNotEmpty` on
`/var/lib/vzctl/agent.token`, and listens on AF_VSOCK port `21950`. The path unit
retriggers start if the token appears after the first boot attempt.

The per-VM NoCloud seed must write an unpadded base64url token containing at
least 256 random bits. The file owner is `vzctl-agent:vzctl-agent`, mode `0600`.
The token is never part of the base image and is never logged.

`vzctl vm create` also writes a fresh NoCloud instance UUID, hostname/FQDN,
per-NIC MAC match, static primary address, default route `via .0 on-link`,
Bridge-`.0` as the only nameserver, the project Search-Domain and SSH host-key
regeneration policy. The sealed base keeps an empty machine-id and no SSH host
keys, so first boot regenerates both per clone. No fixed network or identity
values are written into the base image.

The macOS helper client and live-boot harness are documented in
[`docs/spikes/p0-helper-agent-e2e.md`](../docs/spikes/p0-helper-agent-e2e.md).
