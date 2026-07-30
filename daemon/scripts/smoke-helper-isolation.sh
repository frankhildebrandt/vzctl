#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
daemon_dir=$(dirname "$script_dir")
helper_path=${1:-"$daemon_dir/.build/debug/vz-helper"}
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-helper-isolation.XXXXXX")
pid_a=
pid_b=

cleanup() {
  if [ -n "$pid_a" ]; then kill "$pid_a" 2>/dev/null || true; fi
  if [ -n "$pid_b" ]; then kill "$pid_b" 2>/dev/null || true; fi
  wait "$pid_a" 2>/dev/null || true
  wait "$pid_b" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

mkdir -p "$smoke_root/a" "$smoke_root/b"
VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id smoke-a --bundle "$smoke_root/a" \
  --supervisor-sock "$smoke_root/missing.sock" --mock &
pid_a=$!
VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id smoke-b --bundle "$smoke_root/b" \
  --supervisor-sock "$smoke_root/missing.sock" --mock &
pid_b=$!

sleep 1
test "$(find "$smoke_root/state/helpers" -name '*.lock' | wc -l | tr -d ' ')" = 2
if VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id smoke-b --bundle "$smoke_root/b" \
  --supervisor-sock "$smoke_root/missing.sock" --mock
then
  echo "FAIL: duplicate helper acquired smoke-b lock" >&2
  exit 1
fi
kill -9 "$pid_a"
wait "$pid_a" 2>/dev/null || true
pid_a=
kill -0 "$pid_b"
printf 'PASS: Helper-A kill -9; Helper-B pid=%s and its lock remain active\n' "$pid_b"
