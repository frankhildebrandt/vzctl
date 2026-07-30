#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
daemon_dir=$(dirname "$script_dir")
helper_path=${1:-"$daemon_dir/.build/debug/vz-helper"}
helper_dir=$(CDPATH= cd -- "$(dirname -- "$helper_path")" && pwd -P)
helper_path="$helper_dir/$(basename "$helper_path")"
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/vzctl-helper-launchd.XXXXXX")
vm_id="launchd-smoke-$$"
plist="$smoke_root/helper.plist"
domain="gui/$(id -u)"
label=

cleanup() {
  if [ -n "$label" ]; then
    launchctl bootout "$domain/$label" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

mkdir -p "$smoke_root/bundle"
mkdir -p "$HOME/Library/Logs/vzctl"
chmod 700 "$HOME/Library/Logs/vzctl"
VZCTL_STATE_DIR="$smoke_root/state" "$helper_path" launchd-plist \
  --vm-id "$vm_id" --bundle "$smoke_root/bundle" \
  --supervisor-sock "$smoke_root/missing.sock" \
  --executable "$helper_path" --mock > "$plist"
plutil -lint "$plist"
label=$(plutil -extract Label raw "$plist")
launchctl bootstrap "$domain" "$plist"
sleep 2
launchctl print "$domain/$label" >/dev/null
launchctl bootout "$domain/$label"
printf 'PASS: launchd bootstrap/bootout %s\n' "$label"
