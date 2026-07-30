#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: install.sh <vzctl> <vz-supervisor> <vz-helper>" >&2
  exit 2
fi

cli_source=$1
supervisor_source=$2
helper_source=$3
prefix=${PREFIX:-"$HOME/.local"}
bindir=${BINDIR:-"$prefix/bin"}
launch_agents_dir=${LAUNCH_AGENTS_DIR:-"$HOME/Library/LaunchAgents"}
log_dir=${LOG_DIR:-"$HOME/Library/Logs/vzctl"}
activate=${ACTIVATE:-1}
label=com.vzctl.supervisor
plist_path="$launch_agents_dir/$label.plist"
domain="gui/$(id -u)"
action=installed

case "$activate" in
  0|1) ;;
  *)
    echo "ACTIVATE must be 0 or 1" >&2
    exit 2
    ;;
esac

for binary in "$cli_source" "$supervisor_source" "$helper_source"; do
  if [ ! -x "$binary" ]; then
    echo "missing executable: $binary" >&2
    exit 3
  fi
done

mkdir -p "$bindir" "$launch_agents_dir" "$log_dir"
chmod 700 "$log_dir"
if [ -e "$bindir/vzctl" ] || [ -e "$bindir/vz-supervisor" ] \
  || [ -e "$bindir/vz-helper" ] || [ -e "$plist_path" ]; then
  action=updated
fi

stage_dir=$(mktemp -d "$bindir/.vzctl-install.XXXXXX")
plist_tmp=$(mktemp "$launch_agents_dir/.com.vzctl.supervisor.XXXXXX")
cleanup() {
  rm -rf "$stage_dir"
  rm -f "$plist_tmp"
}
trap cleanup EXIT INT TERM

install -m 0755 "$cli_source" "$stage_dir/vzctl"
install -m 0755 "$supervisor_source" "$stage_dir/vz-supervisor"
install -m 0755 "$helper_source" "$stage_dir/vz-helper"
codesign --verify --strict "$stage_dir/vz-supervisor"
codesign --verify --strict "$stage_dir/vz-helper"
"$stage_dir/vzctl" version >/dev/null
"$stage_dir/vz-supervisor" version >/dev/null
"$stage_dir/vz-helper" version >/dev/null

plutil -create xml1 "$plist_tmp"
plutil -insert Label -string "$label" "$plist_tmp"
plutil -insert ProgramArguments -array "$plist_tmp"
plutil -insert ProgramArguments.0 -string "$bindir/vz-supervisor" "$plist_tmp"
plutil -insert ProgramArguments.1 -string serve "$plist_tmp"
plutil -insert RunAtLoad -bool true "$plist_tmp"
plutil -insert KeepAlive -bool true "$plist_tmp"
plutil -insert ProcessType -string Background "$plist_tmp"
plutil -insert StandardOutPath -string "$log_dir/supervisor.log" "$plist_tmp"
plutil -insert StandardErrorPath -string "$log_dir/supervisor.error.log" "$plist_tmp"
plutil -lint "$plist_tmp"
chmod 0644 "$plist_tmp"

mv -f "$stage_dir/vzctl" "$bindir/vzctl"
mv -f "$stage_dir/vz-supervisor" "$bindir/vz-supervisor"
mv -f "$stage_dir/vz-helper" "$bindir/vz-helper"
mv -f "$plist_tmp" "$plist_path"

if [ "$activate" = 1 ]; then
  if [ "$(uname -s)" != Darwin ]; then
    echo "launchd activation requires macOS" >&2
    exit 4
  fi
  launchctl bootout "$domain/$label" >/dev/null 2>&1 || true
  launchctl bootstrap "$domain" "$plist_path"
  launchctl kickstart -k "$domain/$label"
  launchctl print "$domain/$label" >/dev/null
fi

printf '%s: %s\n' "$action" "$bindir/vzctl"
printf '%s: %s\n' "$action" "$bindir/vz-supervisor"
printf '%s: %s\n' "$action" "$bindir/vz-helper"
printf 'launch agent: %s\n' "$plist_path"
if [ "$activate" = 1 ]; then
  printf 'restarted: %s/%s\n' "$domain" "$label"
else
  printf 'activation skipped (ACTIVATE=0)\n'
fi
