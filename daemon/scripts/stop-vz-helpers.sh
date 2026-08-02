#!/bin/sh
set -eu

if [ "$#" -gt 1 ]; then
  echo "usage: stop-vz-helpers.sh [uid]" >&2
  exit 2
fi

target_uid=${1:-$(id -u)}
pgrep_bin=${VZCTL_PGREP_BIN:-/usr/bin/pgrep}
kill_bin=${VZCTL_KILL_BIN:-/bin/kill}
sleep_bin=${VZCTL_SLEEP_BIN:-/bin/sleep}
timeout_tenths=${VZCTL_HELPER_STOP_TIMEOUT_TENTHS:-600}

case "$target_uid" in
  ''|*[!0-9]*)
    echo "invalid helper uid: $target_uid" >&2
    exit 2
    ;;
esac
case "$timeout_tenths" in
  ''|*[!0-9]*)
    echo "invalid helper stop timeout: $timeout_tenths" >&2
    exit 2
    ;;
esac

helper_pids=$("$pgrep_bin" -U "$target_uid" -x vz-helper 2>/dev/null || true)
if [ -z "$helper_pids" ]; then
  exit 0
fi

printf 'stopping running VM helpers gracefully…\n'
for pid in $helper_pids; do
  case "$pid" in
    ''|*[!0-9]*)
      echo "error: invalid vz-helper pid from pgrep: $pid" >&2
      exit 5
      ;;
  esac
  "$kill_bin" -TERM "$pid" 2>/dev/null || true
done

i=0
while [ "$i" -lt "$timeout_tenths" ]; do
  alive=
  for pid in $helper_pids; do
    if "$kill_bin" -0 "$pid" 2>/dev/null; then
      alive="$alive $pid"
    fi
  done
  if [ -z "$alive" ]; then
    printf 'VM helpers stopped cleanly\n'
    exit 0
  fi
  i=$((i + 1))
  "$sleep_bin" 0.1
done

echo "error: vz-helper still running:$alive; refusing to stop vz-net" >&2
exit 5
