#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
daemon_dir=$(dirname "$script_dir")
helper_path=${1:-"$daemon_dir/.build/debug/vz-helper"}
supervisor_path=${2:-"$daemon_dir/.build/debug/vz-supervisor"}
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-helper-reconnect.XXXXXX")
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
  >"$smoke_root/supervisor-1.log" 2>&1 &
supervisor_pid=$!
for _ in 1 2 3 4 5 6 7 8; do
  test -S "$smoke_root/state/vz.sock" && break
  sleep 0.25
done
test -S "$smoke_root/state/vz.sock"

VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id reconnect-smoke --bundle "$smoke_root/bundle" --mock \
  >"$smoke_root/helper.log" 2>&1 &
helper_pid=$!
sleep 1

before=$(
  printf '%s\n' '{"jsonrpc":"2.0","method":"vm.list","id":1}' |
    nc -U "$smoke_root/state/vz.sock"
)
printf '%s\n' "$before" | grep '"vm_id":"reconnect-smoke"' >/dev/null

kill -9 "$supervisor_pid"
wait "$supervisor_pid" 2>/dev/null || true
supervisor_pid=
sleep 0.5
kill -0 "$helper_pid"

# kill -9 leaves a stale socket inode; fresh serve needs a clean path.
rm -f "$smoke_root/state/vz.sock"

VZCTL_STATE_DIR="$smoke_root/state" "$supervisor_path" serve \
  >"$smoke_root/supervisor-2.log" 2>&1 &
supervisor_pid=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  test -S "$smoke_root/state/vz.sock" && break
  sleep 0.5
done
test -S "$smoke_root/state/vz.sock"

# Heartbeat interval is 5s; wait for re-hello after restart.
reconnected=0
for _ in 1 2 3 4 5 6 7 8; do
  after=$(
    printf '%s\n' '{"jsonrpc":"2.0","method":"vm.list","id":1}' |
      nc -U "$smoke_root/state/vz.sock" || true
  )
  if printf '%s\n' "$after" | grep '"vm_id":"reconnect-smoke"' >/dev/null &&
    printf '%s\n' "$after" | grep '"state":"running"' >/dev/null
  then
    reconnected=1
    break
  fi
  sleep 1
done

test "$reconnected" = 1
kill -0 "$helper_pid"
printf 'PASS: supervisor kill -9; helper stayed up and re-registered via helper.hello\n'
