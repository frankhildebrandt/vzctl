#!/usr/bin/env bash
# Build the ARM64 Linux builder appliance used by `vzctl image seal|bake`
# when local virt-customize is unavailable (macOS).
#
# Requires ARM64 Linux with: curl, sha256sum, qemu-img, virt-customize (libguestfs-tools),
# cloud-init tooling optional (we bake packages into a Debian cloud image).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACTS_DIR="${ARTIFACTS_DIR:-$REPO_ROOT/artifacts}"
WORK_DIR="${WORK_DIR:-$ARTIFACTS_DIR/work-builder}"
OUTPUT_IMAGE="${OUTPUT_IMAGE:-$ARTIFACTS_DIR/vzctl-builder.raw}"
IMAGE_NAME="${IMAGE_NAME:-debian-13-generic-arm64.qcow2}"
IMAGE_URL="${IMAGE_URL:-https://cloud.debian.org/images/cloud/trixie/latest/$IMAGE_NAME}"
CHECKSUM_URL="${CHECKSUM_URL:-https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS}"
SOURCE_IMAGE="$WORK_DIR/$IMAGE_NAME"
CUSTOM_IMAGE="$WORK_DIR/vzctl-builder.qcow2"
BUILDER_VERSION="${BUILDER_VERSION:-1}"

for tool in curl sha256sum qemu-img virt-customize; do
  command -v "$tool" >/dev/null || {
    echo "missing required tool: $tool (run on ARM64 Linux)" >&2
    exit 1
  }
done

mkdir -p "$WORK_DIR" "$ARTIFACTS_DIR"

if [[ ! -f "$SOURCE_IMAGE" ]]; then
  curl --fail --location --progress-bar --output "$SOURCE_IMAGE" "$IMAGE_URL"
fi

CHECKSUMS_FILE="$WORK_DIR/SHA512SUMS"
curl --fail --location --output "$CHECKSUMS_FILE" "$CHECKSUM_URL"
EXPECTED="$(awk -v image="$IMAGE_NAME" '$2 == image || $2 == "*" image {print $1; exit}' "$CHECKSUMS_FILE")"
if [[ ! "$EXPECTED" =~ ^[0-9a-fA-F]{128}$ ]]; then
  echo "could not resolve SHA-512 for $IMAGE_NAME" >&2
  exit 1
fi
printf '%s  %s\n' "$EXPECTED" "$SOURCE_IMAGE" | sha512sum --check -

cp "$SOURCE_IMAGE" "$CUSTOM_IMAGE"
virt-customize -a "$CUSTOM_IMAGE" \
  --install libguestfs-tools,qemu-utils,cloud-init \
  --run-command 'cloud-init clean --logs --machine-id' \
  --run-command 'truncate -s 0 /etc/machine-id' \
  --run-command 'rm -f /var/lib/dbus/machine-id /etc/ssh/ssh_host_* /var/lib/systemd/random-seed' \
  --run-command 'printf "vzctl-builder %s\n" "'"$BUILDER_VERSION"'" > /etc/vzctl-builder-release' \
  --run-command 'sync'

qemu-img convert -p -f qcow2 -O raw "$CUSTOM_IMAGE" "$OUTPUT_IMAGE"
DIGEST="$(sha256sum "$OUTPUT_IMAGE" | awk '{print $1}')"
qemu-img info "$OUTPUT_IMAGE"
echo "builder appliance: $OUTPUT_IMAGE"
echo "sha256: $DIGEST"
echo "Install for local macOS use:"
echo "  mkdir -p \"\${VZCTL_IMAGES_DIR:-\$HOME/Library/Application Support/vzctl/images}/builder\""
echo "  cp \"$OUTPUT_IMAGE\" \"\${VZCTL_IMAGES_DIR:-\$HOME/Library/Application Support/vzctl/images}/builder/vzctl-builder.raw\""
echo "  printf '%s\\n' \"$DIGEST\" > \"\${VZCTL_IMAGES_DIR:-\$HOME/Library/Application Support/vzctl/images}/builder/vzctl-builder.sha256\""
echo "Or: export VZCTL_BUILDER_IMAGE=$OUTPUT_IMAGE"
