#!/bin/sh
# Shared package hooks. Commands use PATH so isolated tests can provide fakes.
fail() { echo "vzctl installer: $*" >&2; exit 5; }

resolve_console_user() {
  console_user=$(stat -f%Su /dev/console)
  console_home=
  console_uid=
  case "$console_user" in
    ''|root|loginwindow|_mbsetupuser) console_user=; return ;;
  esac
  console_home=$(dscl . -read "/Users/$console_user" NFSHomeDirectory |
    sed 's/^NFSHomeDirectory: //')
  [ -n "$console_home" ] && [ -d "$console_home" ] ||
    fail "cannot resolve home for $console_user"
  console_uid=$(id -u "$console_user")
  case "$console_uid" in ''|*[!0-9]*) fail "invalid console uid" ;; esac
  domain="gui/$console_uid"
}

wait_sock() {
  socket_path=$1
  attempts=0
  while [ "$attempts" -lt 100 ]; do
    [ ! -S "$socket_path" ] || return 0
    sleep 0.1
    attempts=$((attempts + 1))
  done
  fail "$socket_path not ready"
}
