# vzctl guest agent

`vzctl-agent` is the unprivileged Linux endpoint for the protocol in
[`docs/specs/guest-agent-v1.md`](../docs/specs/guest-agent-v1.md). The P0
boot-proof slice implements:

- little-endian length-prefixed JSON framing, capped at 1 MiB;
- first-frame `hello`, constant-time token comparison and bounded auth delay;
- `ping`, `version`, `health` and the no-op/unknown `cancel` response;
- explicit `unsupported` responses for `exec`, `report_ip` and `time_hint`.

Only implemented command methods are advertised as capabilities. Helper E2E,
`exec` and IP reporting remain #15; clock handling remains #16.

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

## Runtime

The image pipeline creates the system user `vzctl-agent`, installs the binary
at `/usr/local/sbin/vzctl-agent` and enables
`vzctl-agent.service`. The service waits for `cloud-final.service`, then reads
`/run/vzctl/agent.token` and listens on AF_VSOCK port `21950`.

The per-VM NoCloud seed must write an unpadded base64url token containing at
least 256 random bits. The file owner is `vzctl-agent:vzctl-agent`, mode `0600`.
The token is never part of the base image and is never logged.
