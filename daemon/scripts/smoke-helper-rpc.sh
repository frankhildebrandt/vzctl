#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
daemon_dir=$(dirname "$script_dir")
helper_path=${1:-"$daemon_dir/.build/debug/vz-helper"}
supervisor_path=${2:-"$daemon_dir/.build/debug/vz-supervisor"}
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-helper-rpc.XXXXXX")
supervisor_pid=
helper_pid=

cleanup() {
  if [ -n "$helper_pid" ]; then kill "$helper_pid" 2>/dev/null || true; fi
  if [ -n "$supervisor_pid" ]; then kill "$supervisor_pid" 2>/dev/null || true; fi
  wait "$helper_pid" 2>/dev/null || true
  wait "$supervisor_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

VZCTL_STATE_DIR="$smoke_root/state" "$supervisor_path" serve \
  >"$smoke_root/supervisor.log" 2>&1 &
supervisor_pid=$!
for _ in 1 2 3 4 5; do
  test -S "$smoke_root/state/vz.sock" && break
  sleep 1
done
test -S "$smoke_root/state/vz.sock"

VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id rpc-smoke --bundle "$smoke_root/bundle" --mock \
  >"$smoke_root/helper.log" 2>&1 &
helper_pid=$!
sleep 1

response=$(
  printf '%s\n' '{"jsonrpc":"2.0","method":"vm.list","id":1}' |
    nc -U "$smoke_root/state/vz.sock"
)
printf '%s\n' "$response" | grep '"vm_id":"rpc-smoke"' >/dev/null
printf '%s\n' "$response" | grep '"state":"running"' >/dev/null
printf 'PASS: helper.hello/state visible through vm.list\n'
