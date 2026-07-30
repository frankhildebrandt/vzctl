#!/usr/bin/env bash
# Feasibility smoke: boot builder appliance, attach a fixture disk, mutate via
# virt-customize, verify, clean shutdown. Block Slice B until this passes.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <vzctl-builder.raw> [vz-helper-path]" >&2
  exit 2
fi

APPLIANCE=$1
HELPER=${2:-"${VZCTL_HELPER_PATH:-vz-helper}"}
command -v "$HELPER" >/dev/null || HELPER=$(command -v vz-helper || true)
if [[ -z "$HELPER" || ! -x "$HELPER" ]]; then
  echo "vz-helper not found; set VZCTL_HELPER_PATH" >&2
  exit 12
fi
test -f "$APPLIANCE"

SMOKE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-builder-smoke.XXXXXX")
cleanup() {
  if [[ -n "${HELPER_PID:-}" ]]; then kill "$HELPER_PID" 2>/dev/null || true; wait "$HELPER_PID" 2>/dev/null || true; fi
  rm -rf "$SMOKE_ROOT"
}
trap cleanup EXIT INT TERM

BUNDLE="$SMOKE_ROOT/bundle"
mkdir -p "$BUNDLE" "$SMOKE_ROOT/state" "$SMOKE_ROOT/seed"
cp -c "$APPLIANCE" "$BUNDLE/disk.raw" 2>/dev/null || cp "$APPLIANCE" "$BUNDLE/disk.raw"

# Minimal ext4-ish raw fixture: 64 MiB zeros with a marker file checked via virt-customize
# after we create a tiny partitioned image is heavy; instead attach a qcow2-converted
# empty raw and only prove virt-customize can open /dev/vdb and --run-command succeeds.
FIXTURE="$BUNDLE/dataDisk.raw"
dd if=/dev/zero of="$FIXTURE" bs=1m count=64 status=none
# Put a recognizable string; virt-customize needs a real filesystem — create one with
# qemu-img + guestfish if available, else skip FS mutate and only prove --version path
# via cloud-init against root. Prefer guestfish mkfs when present.
if command -v guestfish >/dev/null; then
  guestfish -a "$FIXTURE" <<'EOF'
run
part-disk /dev/sda mbr
mkfs ext4 /dev/sda1
mount /dev/sda1 /
touch /smoke-before
umount /
EOF
  TARGET_DEV=/dev/vdb1
else
  # Without guestfish on the host, still verify appliance boots and virt-customize works
  # against itself is wrong; require guestfish for full smoke.
  echo "WARN: guestfish missing on host; running boot+version-only smoke" >&2
  TARGET_DEV=
fi

INSTANCE_ID="builder-smoke-$(date +%s)"
cat >"$SMOKE_ROOT/seed/meta-data" <<EOF
instance-id: $INSTANCE_ID
local-hostname: vzctl-builder-smoke
EOF

if [[ -n "$TARGET_DEV" ]]; then
  cat >"$SMOKE_ROOT/seed/user-data" <<EOF
#cloud-config
runcmd:
  - |
    set -e
    virt-customize --version
    virt-customize -a /dev/vdb --format raw --run-command 'test -e /smoke-before'
    virt-customize -a /dev/vdb --format raw --run-command 'touch /smoke-after && sync'
    virt-customize -a /dev/vdb --format raw --run-command 'test -e /smoke-after'
    sync
    printf 'VZCTL_BUILDER_RESULT {"ok":true,"op":"smoke","exit":0}\\n'
    poweroff
EOF
else
  cat >"$SMOKE_ROOT/seed/user-data" <<'EOF'
#cloud-config
runcmd:
  - |
    set -e
    virt-customize --version
    sync
    printf 'VZCTL_BUILDER_RESULT {"ok":true,"op":"smoke-version","exit":0}\n'
    poweroff
EOF
fi

hdiutil makehybrid -iso -joliet -default-volume-name cidata \
  -o "$BUNDLE/cidata.iso" "$SMOKE_ROOT/seed" >/dev/null

export VZCTL_STATE_DIR="$SMOKE_ROOT/state"
"$HELPER" run --vm-id "builder-smoke-$$" --bundle "$BUNDLE" \
  --supervisor-sock "$SMOKE_ROOT/missing.sock" \
  >"$SMOKE_ROOT/helper.out" 2>"$SMOKE_ROOT/helper.err" &
HELPER_PID=$!

SERIAL=""
for _ in $(seq 1 30); do
  if [[ -f "$SMOKE_ROOT/helper.out" ]]; then
    SERIAL=$(awk -F= '/serial=/ {print $NF; exit}' "$SMOKE_ROOT/helper.out" || true)
    [[ -n "$SERIAL" && -f "$SERIAL" ]] && break
  fi
  # Fallback: Logs directory
  CAND=$(ls "$HOME/Library/Logs/vzctl/"*builder-smoke*.serial.log 2>/dev/null | tail -1 || true)
  if [[ -n "$CAND" ]]; then SERIAL=$CAND; break; fi
  sleep 1
done

echo "waiting for VZCTL_BUILDER_RESULT (serial=${SERIAL:-unknown})…"
DEADLINE=$((SECONDS + 600))
RESULT=""
while (( SECONDS < DEADLINE )); do
  if [[ -n "$SERIAL" && -f "$SERIAL" ]]; then
    if RESULT=$(grep -E 'VZCTL_BUILDER_RESULT ' "$SERIAL" | tail -1); then
      [[ -n "$RESULT" ]] && break
    fi
  fi
  if ! kill -0 "$HELPER_PID" 2>/dev/null; then
    echo "helper exited early" >&2
    cat "$SMOKE_ROOT/helper.err" >&2 || true
    exit 13
  fi
  sleep 2
done

kill "$HELPER_PID" 2>/dev/null || true
wait "$HELPER_PID" 2>/dev/null || true
HELPER_PID=

if [[ -z "$RESULT" ]]; then
  echo "FAIL: no VZCTL_BUILDER_RESULT within timeout" >&2
  [[ -n "$SERIAL" ]] && tail -100 "$SERIAL" >&2 || true
  exit 13
fi

echo "PASS: $RESULT"
