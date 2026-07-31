#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base_image="${1:-}"
helper_path="${HELPER_PATH:-$repo_root/daemon/.build/debug/vz-helper}"

if [[ -z "$base_image" ]]; then
  echo "usage: $0 /path/to/ubuntu-24.04-vzctl-base.raw" >&2
  exit 2
fi
if [[ ! -f "$base_image" ]]; then
  echo "base image not found: $base_image" >&2
  exit 1
fi
for tool in hdiutil openssl swift codesign cp; do
  command -v "$tool" >/dev/null || {
    echo "missing required tool: $tool" >&2
    exit 1
  }
done

smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/vzctl-helper-agent-e2e.XXXXXX")"
bundle="$smoke_root/bundle"
seed="$smoke_root/seed"
mkdir -p "$bundle" "$seed"

cleanup() {
  if [[ "${KEEP_SMOKE_BUNDLE:-0}" == "1" ]]; then
    echo "kept smoke bundle: $smoke_root"
  else
    rm -rf -- "$smoke_root"
  fi
}
trap cleanup EXIT

if ! cp -c "$base_image" "$bundle/disk.raw"; then
  echo "APFS clone failed; refusing a full raw-image copy" >&2
  exit 1
fi

umask 077
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n' > "$bundle/agent.token"
chmod 0600 "$bundle/agent.token"
token="$(<"$bundle/agent.token")"

cat > "$seed/meta-data" <<EOF
instance-id: vzctl-helper-agent-e2e
local-hostname: vzctl-e2e
EOF
cat > "$seed/network-config" <<'EOF'
version: 2
ethernets:
  primary:
    match:
      name: "en*"
    dhcp4: true
    dhcp6: true
EOF
cat > "$seed/user-data" <<EOF
#cloud-config
hostname: vzctl-e2e
manage_etc_hosts: true
write_files:
  - path: /var/lib/vzctl/agent.token
    owner: vzctl-agent:vzctl-agent
    permissions: "0600"
    content: |
      $token
EOF
unset token

hdiutil makehybrid \
  -o "$bundle/cidata.iso" \
  -hfs -joliet -iso \
  -default-volume-name cidata \
  "$seed" >/dev/null

swift build --package-path "$repo_root/daemon" --product vz-helper
"$repo_root/daemon/scripts/codesign-helper.sh" "$helper_path" >/dev/null

"$helper_path" agent-smoke \
  --vm-id p0-helper-agent-e2e \
  --bundle "$bundle" \
  --disk "$bundle/disk.raw" \
  --cidata "$bundle/cidata.iso" \
  --agent-token "$bundle/agent.token"

echo "helper-agent E2E smoke passed"
