#!/usr/bin/env bash
set -euo pipefail

IMAGE_PATH="${1:-}"
if [[ -z "$IMAGE_PATH" ]]; then
  echo "usage: $0 /path/to/ubuntu-24.04-vzctl-base.raw" >&2
  exit 2
fi
for tool in file virt-cat virt-ls; do
  command -v "$tool" >/dev/null || {
    echo "missing required tool: $tool" >&2
    exit 1
  }
done

AGENT="$(virt-ls -l -a "$IMAGE_PATH" /usr/local/sbin/vzctl-agent)"
BINARY_TYPE="$(virt-cat -a "$IMAGE_PATH" /usr/local/sbin/vzctl-agent | file -)"
UNIT="$(virt-cat -a "$IMAGE_PATH" /etc/systemd/system/vzctl-agent.service)"
ENABLED="$(virt-cat -a "$IMAGE_PATH" /etc/systemd/system/multi-user.target.wants/vzctl-agent.service)"
METADATA="$(virt-cat -a "$IMAGE_PATH" /usr/lib/vzctl-agent/image-metadata.json)"
MACHINE_ID="$(virt-cat -a "$IMAGE_PATH" /etc/machine-id)"

grep -Eq '^-rwxr-xr-x ' <<<"$AGENT"
grep -q 'vzctl-agent$' <<<"$AGENT"
grep -Eq 'ELF 64-bit.*ARM aarch64.*statically linked' <<<"$BINARY_TYPE"
grep -q 'ExecStart=/usr/local/sbin/vzctl-agent --port 21950' <<<"$UNIT"
grep -q 'User=vzctl-agent' <<<"$UNIT"
grep -q 'Restart=on-failure' <<<"$UNIT"
grep -q '^CapabilityBoundingSet=CAP_SYS_TIME$' <<<"$UNIT"
grep -q '^AmbientCapabilities=CAP_SYS_TIME$' <<<"$UNIT"
grep -q '^ProtectClock=no$' <<<"$UNIT"
test "$ENABLED" = "$UNIT"
grep -Eq '"agent_version":"[^"]+"' <<<"$METADATA"
grep -q '"protocol":1' <<<"$METADATA"
grep -q '"vsock_port":21950' <<<"$METADATA"
test -z "$MACHINE_ID"
if virt-ls -a "$IMAGE_PATH" /run/vzctl/agent.token >/dev/null 2>&1; then
  echo "base must not contain /run/vzctl/agent.token" >&2
  exit 1
fi

echo "offline smoke passed: binary/unit enabled, metadata present, identity cleared"
echo "boot proof: clone with guest-agent/image/cloud-init, then run:"
echo "  systemctl is-active vzctl-agent"
echo "  systemctl show -p User -p MainPID vzctl-agent"
echo "  ss --vsock --listening --numeric | grep ':21950'"
