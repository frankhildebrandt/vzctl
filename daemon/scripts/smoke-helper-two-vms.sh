#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: $0 <source.raw> [cidata.iso]" >&2
  exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
daemon_dir=$(dirname "$script_dir")
helper_path="$daemon_dir/.build/debug/vz-helper"
supervisor_path="$daemon_dir/.build/debug/vz-supervisor"
source_disk=$1
cidata=${2:-}
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-helper-vms.XXXXXX")
vm_a="vm-smoke-a-$$"
vm_b="vm-smoke-b-$$"
supervisor_pid=
pid_a=
pid_b=

cleanup() {
  if [ -n "$pid_a" ]; then kill "$pid_a" 2>/dev/null || true; fi
  if [ -n "$pid_b" ]; then kill "$pid_b" 2>/dev/null || true; fi
  if [ -n "$supervisor_pid" ]; then kill "$supervisor_pid" 2>/dev/null || true; fi
  wait "$pid_a" 2>/dev/null || true
  wait "$pid_b" 2>/dev/null || true
  wait "$supervisor_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

test -f "$source_disk"
mkdir -p "$smoke_root/a" "$smoke_root/b"
cp -c "$source_disk" "$smoke_root/a/disk.raw"
cp -c "$source_disk" "$smoke_root/b/disk.raw"
if [ -n "$cidata" ]; then
  test -f "$cidata"
  cp "$cidata" "$smoke_root/a/cidata.iso"
  cp "$cidata" "$smoke_root/b/cidata.iso"
fi

VZCTL_STATE_DIR="$smoke_root/state" "$supervisor_path" serve \
  >"$smoke_root/supervisor.log" 2>&1 &
supervisor_pid=$!
for _ in 1 2 3 4 5; do
  test -S "$smoke_root/state/vz.sock" && break
  sleep 1
done
test -S "$smoke_root/state/vz.sock"

VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id "$vm_a" --bundle "$smoke_root/a" \
  >"$smoke_root/a/helper.log" 2>&1 &
pid_a=$!
VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" run \
  --vm-id "$vm_b" --bundle "$smoke_root/b" \
  >"$smoke_root/b/helper.log" 2>&1 &
pid_b=$!

for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  grep 'state=running serial=' "$smoke_root/a/helper.log" >/dev/null 2>&1 &&
    grep 'state=running serial=' "$smoke_root/b/helper.log" >/dev/null 2>&1 &&
    break
  kill -0 "$pid_a"
  kill -0 "$pid_b"
  sleep 1
done
grep 'state=running serial=' "$smoke_root/a/helper.log" >/dev/null
grep 'state=running serial=' "$smoke_root/b/helper.log" >/dev/null

serial_a=
serial_b=
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30; do
  serial_a=$(find "$HOME/Library/Logs/vzctl" -name "$vm_a-*.serial.log" -size +0 -print -quit)
  serial_b=$(find "$HOME/Library/Logs/vzctl" -name "$vm_b-*.serial.log" -size +0 -print -quit)
  test -n "$serial_a" && test -n "$serial_b" && break
  sleep 1
done
test -n "$serial_a"
test -n "$serial_b"
test -s "$serial_a"
test -s "$serial_b"

kill -9 "$pid_a"
wait "$pid_a" 2>/dev/null || true
pid_a=
kill -0 "$pid_b"
kill -TERM "$pid_b"
wait "$pid_b"
pid_b=
printf 'PASS: two VZVirtualMachines started; A kill -9 left B alive; B stopped via SIGTERM\n'
printf 'serial A: %s\nserial B: %s\n' "$serial_a" "$serial_b"
