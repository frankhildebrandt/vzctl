#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
daemon_dir="$repo_root/daemon"
state_root=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-edge-smoke.XXXXXX")
edge_pid=""
net_pid=""
supervisor_pid=""

cleanup() {
  [ -z "$supervisor_pid" ] || kill "$supervisor_pid" >/dev/null 2>&1 || true
  [ -z "$edge_pid" ] || kill "$edge_pid" >/dev/null 2>&1 || true
  [ -z "$net_pid" ] || kill "$net_pid" >/dev/null 2>&1 || true
  rm -rf "$state_root"
}
trap cleanup EXIT INT TERM

swift build --package-path "$daemon_dir"
net_bin="$daemon_dir/.build/debug/vz-net"
edge_bin="$daemon_dir/.build/debug/vz-edge"
supervisor_bin="$daemon_dir/.build/debug/vz-supervisor"

wait_socket() {
  socket_path=$1
  attempt=0
  while [ "$attempt" -lt 100 ]; do
    [ ! -S "$socket_path" ] || return 0
    attempt=$((attempt + 1))
    sleep 0.05
  done
  echo "socket did not appear: $socket_path" >&2
  return 1
}

rpc() {
  socket_path=$1
  method=$2
  /usr/bin/python3 - "$socket_path" "$method" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(3)
s.connect(sys.argv[1])
s.sendall((json.dumps({"jsonrpc":"2.0","id":1,"method":sys.argv[2]}) + "\n").encode())
line = b""
while not line.endswith(b"\n"):
    chunk = s.recv(65536)
    if not chunk:
        break
    line += chunk
value = json.loads(line)
if value.get("error"):
    raise SystemExit(value["error"])
print(json.dumps(value["result"], sort_keys=True))
PY
}

VZCTL_STATE_DIR="$state_root" "$net_bin" serve >"$state_root/net.log" 2>&1 &
net_pid=$!
wait_socket "$state_root/net.sock"

VZCTL_STATE_DIR="$state_root" VZCTL_DNS_PORT=25363 VZCTL_DNS_GUEST_PORT=25364 \
  "$edge_bin" serve >"$state_root/edge.log" 2>&1 &
edge_pid=$!
wait_socket "$state_root/edge.sock"

VZCTL_STATE_DIR="$state_root" "$supervisor_bin" serve >"$state_root/supervisor.log" 2>&1 &
supervisor_pid=$!
wait_socket "$state_root/vz.sock"
before=$(rpc "$state_root/edge.sock" edge.status)

kill -9 "$supervisor_pid"
wait "$supervisor_pid" 2>/dev/null || true
supervisor_pid=""
after_cp_kill=$(rpc "$state_root/edge.sock" edge.status)
[ "$before" = "$after_cp_kill" ] || {
  echo "edge state changed after control-plane kill" >&2
  exit 1
}

kill -9 "$edge_pid"
wait "$edge_pid" 2>/dev/null || true
edge_pid=""
VZCTL_STATE_DIR="$state_root" VZCTL_DNS_PORT=25363 VZCTL_DNS_GUEST_PORT=25364 \
  "$edge_bin" serve >>"$state_root/edge.log" 2>&1 &
edge_pid=$!
wait_socket "$state_root/edge.sock"
after_edge_restart=$(rpc "$state_root/edge.sock" edge.status)
[ "$before" = "$after_edge_restart" ] || {
  echo "edge did not restore the last-good generation" >&2
  exit 1
}

echo "PASS: control-plane kill preserved edge; edge restart restored manifest"
