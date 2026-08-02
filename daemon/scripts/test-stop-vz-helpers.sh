#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
subject="$root/daemon/scripts/stop-vz-helpers.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-stop-helpers-test.XXXXXX")
cleanup() {
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

fail() {
  echo "test-stop-vz-helpers: $*" >&2
  exit 1
}

assert_before() {
  file=$1
  first=$2
  second=$3
  first_line=$(grep -nF "$first" "$file" | head -1 | cut -d: -f1)
  second_line=$(grep -nF "$second" "$file" | head -1 | cut -d: -f1)
  [ -n "$first_line" ] || fail "missing install guard in $file: $first"
  [ -n "$second_line" ] || fail "missing shutdown in $file: $second"
  [ "$first_line" -lt "$second_line" ] \
    || fail "helper guard runs too late in $file"
}

mock_pgrep="$tmp/pgrep"
mock_kill="$tmp/kill"
mock_sleep="$tmp/sleep"

cat >"$mock_pgrep" <<'EOF'
#!/bin/sh
if [ -n "${MOCK_HELPER_PIDS:-}" ]; then
  printf '%s\n' "$MOCK_HELPER_PIDS"
  exit 0
fi
exit 1
EOF

cat >"$mock_kill" <<'EOF'
#!/bin/sh
signal=$1
pid=$2
printf '%s %s\n' "$signal" "$pid" >>"$MOCK_KILL_LOG"
if [ "$signal" = "-TERM" ]; then
  if [ "${MOCK_STUBBORN:-0}" != 1 ]; then
    : >"${MOCK_STOPPED}${pid}"
  fi
  exit 0
fi
if [ "$signal" = "-0" ] && [ ! -e "${MOCK_STOPPED}${pid}" ]; then
  exit 0
fi
exit 1
EOF

cat >"$mock_sleep" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$mock_pgrep" "$mock_kill" "$mock_sleep"

run_subject() {
  MOCK_KILL_LOG="$tmp/kill.log" \
  MOCK_STOPPED="$tmp/stopped." \
  VZCTL_PGREP_BIN="$mock_pgrep" \
  VZCTL_KILL_BIN="$mock_kill" \
  VZCTL_SLEEP_BIN="$mock_sleep" \
  VZCTL_HELPER_STOP_TIMEOUT_TENTHS=2 \
  "$subject" 501
}

: >"$tmp/kill.log"
MOCK_HELPER_PIDS= run_subject
[ ! -s "$tmp/kill.log" ] || fail "signalled a process when no helper was running"

: >"$tmp/kill.log"
export MOCK_HELPER_PIDS="101 202"
unset MOCK_STUBBORN
run_subject >"$tmp/graceful.out"
grep -q -- '-TERM 101' "$tmp/kill.log" || fail "helper 101 did not receive SIGTERM"
grep -q -- '-TERM 202' "$tmp/kill.log" || fail "helper 202 did not receive SIGTERM"
grep -q 'VM helpers stopped cleanly' "$tmp/graceful.out" \
  || fail "graceful completion was not reported"

: >"$tmp/kill.log"
rm -f "$tmp/stopped.303"
export MOCK_HELPER_PIDS=303
export MOCK_STUBBORN=1
if run_subject >"$tmp/stubborn.out" 2>"$tmp/stubborn.err"; then
  fail "stubborn helper unexpectedly succeeded"
fi
grep -q 'refusing to stop vz-net' "$tmp/stubborn.err" \
  || fail "stubborn helper did not block vz-net shutdown"
if grep -q -- '-KILL' "$tmp/kill.log"; then
  fail "stubborn helper received SIGKILL"
fi

assert_before \
  "$root/daemon/scripts/install.sh" \
  '"$stop_helpers" "$(id -u)"' \
  'bootout "$domain/$net_label"'
assert_before \
  "$root/packaging/macos/postinstall" \
  '"$stop_helpers" "$uid"' \
  'bootout "$domain/$label_net"'
grep -qF '"$scripts_dir/stop-vz-helpers.sh"' "$root/scripts/package-macos.sh" \
  || fail "pkg does not include the helper shutdown guard"

echo "test-stop-vz-helpers: ok"
