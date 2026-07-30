#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN=".build/debug/G0Spike"
LOG=/tmp/g0-phase-d.log
WITH_GUEST="${1:-}"

swift build
codesign --force --sign - --entitlements "$ROOT/G0Spike.entitlements" "$BIN"

if [[ "$WITH_GUEST" == "--guest" ]]; then
  ASSETS="$ROOT/assets"
  mkdir -p "$ASSETS/cidata-crash"
  cat > "$ASSETS/cidata-crash/meta-data" <<EOF
instance-id: g0-crash-$(date +%s)
local-hostname: crash
EOF
  cat > "$ASSETS/cidata-crash/user-data" <<'EOF'
#cloud-config
hostname: crash
users:
  - name: ubuntu
    sudo: ALL=(ALL) NOPASSWD:ALL
    lock_passwd: false
    plain_text_passwd: "ubuntu"
ssh_pwauth: true
package_update: false
EOF
  cat > "$ASSETS/cidata-crash/network-config" <<'EOF'
version: 2
ethernets:
  id0:
    match:
      macaddress: "52:54:00:90:01:10"
    dhcp4: false
    addresses: [10.93.1.10/24]
    routes:
      - to: default
        via: 10.93.1.0
        on-link: true
    nameservers:
      addresses: [10.93.1.0]
EOF
  xorriso -as mkisofs -R -V cidata -o "$ASSETS/cidata-crash.iso" "$ASSETS/cidata-crash" >/dev/null
  [[ -f $ASSETS/base.raw ]] || { echo "missing base.raw"; exit 1; }
  rm -f "$ASSETS/crash.raw" "$ASSETS/crash-nvram.bin"
  cp -c "$ASSETS/base.raw" "$ASSETS/crash.raw"
fi

rm -f /tmp/g0-crash-state.json "$LOG"
echo "== starting hold-crash =="
stdbuf -oL -eL "$BIN" hold-crash ${WITH_GUEST:+--guest} >"$LOG" 2>&1 &
HPID=$!
echo "holder pid=$HPID"

# Wait for CRASH_READY
for i in $(seq 1 90); do
  if rg -q 'CRASH_READY' "$LOG" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$HPID" 2>/dev/null; then
    echo "holder exited early:"; cat "$LOG"; exit 1
  fi
  sleep 2
done
rg 'CRASH_READY|HOST_PING|error' "$LOG" || true
STATE_PID=$(python3 -c "import json; print(json.load(open('/tmp/g0-crash-state.json'))['pid'])")
GUEST=$(python3 -c "import json; print(json.load(open('/tmp/g0-crash-state.json')).get('guestIP') or '')")

echo "== pre-kill guest ping =="
if [[ -n "$GUEST" ]]; then
  ping -c1 -W1000 "$GUEST" && echo PRE_KILL_GUEST_OK || echo PRE_KILL_GUEST_FAIL
fi

echo "== kill -9 $STATE_PID =="
kill -9 "$STATE_PID" || true
sleep 2
kill -0 "$STATE_PID" 2>/dev/null && echo 'still alive?!' || echo KILL_OK

echo "== post-kill guest ping =="
if [[ -n "$GUEST" ]]; then
  ping -c1 -W1000 "$GUEST" && echo POST_KILL_GUEST_ALIVE || echo POST_KILL_GUEST_DEAD
fi

echo "== bridges after kill =="
ifconfig | rg '10\.93\.1|bridge10' || echo 'no 10.93 bridge'

echo "== recreate-probe =="
stdbuf -oL -eL "$BIN" recreate-probe | tee -a "$LOG"

echo "== phase-d-crash done — see $LOG =="
