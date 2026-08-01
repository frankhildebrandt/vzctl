#!/usr/bin/env bash
# Spike: CP-style crash must NOT burn CIDRs when vz-net holds the refs.
# Proves ADR 0002 HyperNetwork split (docs/specs/vz-net-v1.md).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/daemon/.build/debug/vz-net"
STATE="$(mktemp -d /tmp/vzctl-net-spike.XXXXXX)"
CIDR="10.211.77.0/24"
NAME="spike-dmz"
LOG="$STATE/vz-net.log"

cleanup() {
  if [[ -n "${NET_PID:-}" ]] && kill -0 "$NET_PID" 2>/dev/null; then
    kill -TERM "$NET_PID" 2>/dev/null || true
    wait "$NET_PID" 2>/dev/null || true
  fi
  rm -rf "$STATE"
}
trap cleanup EXIT

echo "== build + codesign vz-net =="
swift build --package-path "$ROOT/daemon" --product vz-net
codesign --force --sign - --entitlements "$ROOT/daemon/VzHelper.entitlements" "$BIN"

echo "== start vz-net state=$STATE =="
VZCTL_STATE_DIR="$STATE" "$BIN" serve >"$LOG" 2>&1 &
NET_PID=$!
for _ in $(seq 1 50); do
  [[ -S "$STATE/net.sock" ]] && break
  sleep 0.1
done
[[ -S "$STATE/net.sock" ]] || { echo "FAIL: net.sock missing"; cat "$LOG"; exit 1; }
echo "NET_PID=$NET_PID"

rpc() {
  python3 - "$STATE/net.sock" "$1" <<'PY'
import json, socket, sys
path, payload = sys.argv[1], sys.argv[2]
req = payload if payload.startswith("{") else json.dumps({"jsonrpc":"2.0","id":1,"method":payload,"params":{}})
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(path)
s.sendall((req + "\n").encode())
buf = b""
while not buf.endswith(b"\n"):
    chunk = s.recv(4096)
    if not chunk:
        break
    buf += chunk
s.close()
print(buf.decode().strip())
PY
}

echo "== acquire $NAME $CIDR =="
ACQUIRE=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"net.acquire\",\"params\":{\"name\":\"$NAME\",\"cidr\":\"$CIDR\",\"mode\":\"shared\",\"nat_egress\":true}}")
echo "$ACQUIRE"
echo "$ACQUIRE" | rg -q '"gateway"' || { echo "FAIL: acquire"; exit 1; }

echo "== simulate CP kill-9 (no-op on refs; CP never held them) =="
# A throwaway client process that only called acquire already exited.
# Re-acquire must be idempotent while vz-net still holds the reservation.
REACQUIRE=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"net.acquire\",\"params\":{\"name\":\"$NAME\",\"cidr\":\"$CIDR\",\"mode\":\"shared\",\"nat_egress\":true}}")
echo "$REACQUIRE"
echo "$REACQUIRE" | rg -q '"gateway"' || { echo "FAIL: reacquire after simulated CP death"; exit 1; }
echo "CP_KILL_REACQUIRE_OK $CIDR"

echo "== serialize still works =="
SER=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"net.serialize\",\"params\":{\"name\":\"$NAME\"}}")
echo "$SER" | rg -q '"serialization"' || { echo "FAIL: serialize"; exit 1; }
echo "SERIALIZE_OK"

echo "== clean release =="
REL=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"net.release\",\"params\":{\"name\":\"$NAME\"}}")
echo "$REL"
echo "$REL" | rg -q '"released":true' || { echo "FAIL: release"; exit 1; }

echo "== re-create same CIDR after clean release =="
AGAIN=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"net.acquire\",\"params\":{\"name\":\"$NAME\",\"cidr\":\"$CIDR\",\"mode\":\"shared\",\"nat_egress\":true}}")
echo "$AGAIN"
echo "$AGAIN" | rg -q '"gateway"' || { echo "FAIL: recreate after clean release"; exit 1; }
echo "CLEAN_RELEASE_RECREATE_OK $CIDR"

rpc "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"net.release\",\"params\":{\"name\":\"$NAME\"}}" >/dev/null

echo "== spike passed =="
