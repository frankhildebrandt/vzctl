#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT_ROOT="$REPO_ROOT/guest-agent"
ARTIFACTS_DIR="${ARTIFACTS_DIR:-$REPO_ROOT/artifacts}"
WORK_DIR="${WORK_DIR:-$ARTIFACTS_DIR/work}"
IMAGE_NAME="${IMAGE_NAME:-ubuntu-24.04-server-cloudimg-arm64.img}"
IMAGE_URL="${IMAGE_URL:-https://cloud-images.ubuntu.com/releases/noble/release/$IMAGE_NAME}"
CHECKSUM_URL="${CHECKSUM_URL:-https://cloud-images.ubuntu.com/releases/noble/release/SHA256SUMS}"
AGENT_VERSION="${AGENT_VERSION:-$(tr -d '[:space:]' < "$AGENT_ROOT/VERSION")}"
SOURCE_IMAGE="$WORK_DIR/$IMAGE_NAME"
CUSTOM_IMAGE="$WORK_DIR/ubuntu-24.04-vzctl-base.qcow2"
OUTPUT_IMAGE="${OUTPUT_IMAGE:-$ARTIFACTS_DIR/ubuntu-24.04-vzctl-base.raw}"
AGENT_BINARY="$WORK_DIR/vzctl-agent"
STAGING_DIR="$WORK_DIR/staging"

if [[ ! "$AGENT_VERSION" =~ ^[0-9A-Za-z][0-9A-Za-z._+-]*$ ]]; then
  echo "invalid AGENT_VERSION: use only letters, digits, dot, underscore, plus and hyphen" >&2
  exit 1
fi

for tool in curl sha256sum qemu-img virt-customize go; do
  command -v "$tool" >/dev/null || {
    echo "missing required tool: $tool" >&2
    exit 1
  }
done

mkdir -p "$WORK_DIR" "$ARTIFACTS_DIR"

if [[ ! -f "$SOURCE_IMAGE" ]]; then
  curl --fail --location --output "$SOURCE_IMAGE" "$IMAGE_URL"
fi

if [[ -n "${UBUNTU_IMAGE_SHA256:-}" ]]; then
  EXPECTED_SHA256="$UBUNTU_IMAGE_SHA256"
else
  CHECKSUMS_FILE="$WORK_DIR/SHA256SUMS"
  curl --fail --location --output "$CHECKSUMS_FILE" "$CHECKSUM_URL"
  EXPECTED_SHA256="$(awk -v image="$IMAGE_NAME" '$2 == image || $2 == "*" image {print $1; exit}' "$CHECKSUMS_FILE")"
fi
if [[ ! "$EXPECTED_SHA256" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "could not resolve a valid SHA-256 for $IMAGE_NAME" >&2
  exit 1
fi
printf '%s  %s\n' "$EXPECTED_SHA256" "$SOURCE_IMAGE" | sha256sum --check -

(
  cd "$AGENT_ROOT"
  CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build \
    -trimpath \
    -ldflags "-s -w -buildid= -X main.version=$AGENT_VERSION" \
    -o "$AGENT_BINARY" \
    ./cmd/vzctl-agent
)

mkdir -p "$STAGING_DIR"
install -m 0755 "$AGENT_BINARY" "$STAGING_DIR/vzctl-agent"
install -m 0644 "$AGENT_ROOT/systemd/vzctl-agent.service" "$STAGING_DIR/vzctl-agent.service"
install -m 0644 "$AGENT_ROOT/systemd/vzctl-agent-tmpfiles.conf" "$STAGING_DIR/vzctl-agent-tmpfiles.conf"
printf '{"agent_version":"%s","protocol":1,"vsock_port":21950}\n' "$AGENT_VERSION" \
  > "$STAGING_DIR/image-metadata.json"

cp "$SOURCE_IMAGE" "$CUSTOM_IMAGE"
virt-customize -a "$CUSTOM_IMAGE" \
  --mkdir /usr/lib/vzctl-agent \
  --copy-in "$STAGING_DIR/vzctl-agent:/usr/local/sbin" \
  --copy-in "$STAGING_DIR/vzctl-agent.service:/etc/systemd/system" \
  --copy-in "$STAGING_DIR/vzctl-agent-tmpfiles.conf:/usr/lib/tmpfiles.d" \
  --copy-in "$STAGING_DIR/image-metadata.json:/usr/lib/vzctl-agent" \
  --run-command 'id -u vzctl-agent >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin vzctl-agent' \
  --run-command 'chmod 0755 /usr/local/sbin/vzctl-agent && chmod 0644 /etc/systemd/system/vzctl-agent.service /usr/lib/tmpfiles.d/vzctl-agent-tmpfiles.conf /usr/lib/vzctl-agent/image-metadata.json' \
  --run-command 'systemctl enable vzctl-agent.service' \
  --run-command 'cloud-init clean --logs --machine-id' \
  --run-command 'truncate -s 0 /etc/machine-id' \
  --run-command 'rm -f /var/lib/dbus/machine-id /etc/ssh/ssh_host_* /var/lib/systemd/random-seed' \
  --run-command 'sync'

qemu-img convert -p -f qcow2 -O raw "$CUSTOM_IMAGE" "$OUTPUT_IMAGE"
qemu-img info "$OUTPUT_IMAGE"
echo "seal-ready base: $OUTPUT_IMAGE"
echo "agent metadata: /usr/lib/vzctl-agent/image-metadata.json"
