#!/bin/sh
set -eu

if [ "$#" -lt 4 ] || [ "$#" -gt 5 ]; then
  echo "usage: install.sh <vzctl> <vz-net> <vz-supervisor> <vz-helper> [vz-dns-bind]" >&2
  exit 2
fi

cli_source=$1
net_source=$2
supervisor_source=$3
helper_source=$4
dns_bind_source=${5:-}
prefix=${PREFIX:-"$HOME/.local"}
bindir=${BINDIR:-"$prefix/bin"}
launch_agents_dir=${LAUNCH_AGENTS_DIR:-"$HOME/Library/LaunchAgents"}
log_dir=${LOG_DIR:-"$HOME/Library/Logs/vzctl"}
activate=${ACTIVATE:-1}
net_label=com.vzctl.net
supervisor_label=com.vzctl.supervisor
net_plist_path="$launch_agents_dir/$net_label.plist"
supervisor_plist_path="$launch_agents_dir/$supervisor_label.plist"
domain="gui/$(id -u)"
action=installed

case "$activate" in
  0|1) ;;
  *)
    echo "ACTIVATE must be 0 or 1" >&2
    exit 2
    ;;
esac

for binary in "$cli_source" "$net_source" "$supervisor_source" "$helper_source"; do
  if [ ! -x "$binary" ]; then
    echo "missing executable: $binary" >&2
    exit 3
  fi
done
if [ -n "$dns_bind_source" ] && [ ! -x "$dns_bind_source" ]; then
  echo "missing executable: $dns_bind_source" >&2
  exit 3
fi

mkdir -p "$bindir" "$launch_agents_dir" "$log_dir"
chmod 700 "$log_dir"
if [ -e "$bindir/vzctl" ] || [ -e "$bindir/vz-net" ] \
    || [ -e "$bindir/vz-supervisor" ] || [ -e "$bindir/vz-helper" ] \
    || [ -e "$net_plist_path" ] || [ -e "$supervisor_plist_path" ]; then
  action=updated
fi
if [ -n "$dns_bind_source" ] && [ -e "$bindir/vz-dns-bind" ]; then
  action=updated
fi

stage_dir=$(mktemp -d "$bindir/.vzctl-install.XXXXXX")
net_plist_tmp=$(mktemp "$launch_agents_dir/.com.vzctl.net.XXXXXX")
supervisor_plist_tmp=$(mktemp "$launch_agents_dir/.com.vzctl.supervisor.XXXXXX")
cleanup() {
  rm -rf "$stage_dir"
  rm -f "$net_plist_tmp" "$supervisor_plist_tmp"
}
trap cleanup EXIT INT TERM

install -m 0755 "$cli_source" "$stage_dir/vzctl"
install -m 0755 "$net_source" "$stage_dir/vz-net"
install -m 0755 "$supervisor_source" "$stage_dir/vz-supervisor"
install -m 0755 "$helper_source" "$stage_dir/vz-helper"
if [ -n "$dns_bind_source" ]; then
  install -m 0755 "$dns_bind_source" "$stage_dir/vz-dns-bind"
fi
codesign --verify --strict "$stage_dir/vz-net"
codesign --verify --strict "$stage_dir/vz-supervisor"
codesign --verify --strict "$stage_dir/vz-helper"
"$stage_dir/vzctl" version >/dev/null
"$stage_dir/vz-net" version >/dev/null
"$stage_dir/vz-supervisor" version >/dev/null
"$stage_dir/vz-helper" version >/dev/null
if [ -n "$dns_bind_source" ]; then
  "$stage_dir/vz-dns-bind" version >/dev/null
fi

write_agent_plist() {
  plist=$1
  label=$2
  binary=$3
  command=$4
  log_name=$5
  plutil -create xml1 "$plist"
  plutil -insert Label -string "$label" "$plist"
  plutil -insert ProgramArguments -array "$plist"
  plutil -insert ProgramArguments.0 -string "$binary" "$plist"
  plutil -insert ProgramArguments.1 -string "$command" "$plist"
  plutil -insert RunAtLoad -bool true "$plist"
  plutil -insert KeepAlive -bool true "$plist"
  plutil -insert ProcessType -string Background "$plist"
  plutil -insert StandardOutPath -string "$log_dir/${log_name}.log" "$plist"
  plutil -insert StandardErrorPath -string "$log_dir/${log_name}.error.log" "$plist"
  plutil -lint "$plist"
  chmod 0644 "$plist"
}

write_agent_plist "$net_plist_tmp" "$net_label" "$bindir/vz-net" serve net
write_agent_plist "$supervisor_plist_tmp" "$supervisor_label" "$bindir/vz-supervisor" serve supervisor

mv -f "$stage_dir/vzctl" "$bindir/vzctl"
mv -f "$stage_dir/vz-net" "$bindir/vz-net"
mv -f "$stage_dir/vz-supervisor" "$bindir/vz-supervisor"
mv -f "$stage_dir/vz-helper" "$bindir/vz-helper"
if [ -n "$dns_bind_source" ]; then
  mv -f "$stage_dir/vz-dns-bind" "$bindir/vz-dns-bind"
fi
mv -f "$net_plist_tmp" "$net_plist_path"
mv -f "$supervisor_plist_tmp" "$supervisor_plist_path"

if [ "$activate" = 1 ]; then
  if [ "$(uname -s)" != Darwin ]; then
    echo "launchd activation requires macOS" >&2
    exit 4
  fi
  # HyperNetwork Supervisor first — CP depends on net.sock.
  launchctl bootout "$domain/$supervisor_label" >/dev/null 2>&1 || true
  launchctl bootout "$domain/$net_label" >/dev/null 2>&1 || true
  launchctl bootstrap "$domain" "$net_plist_path"
  launchctl kickstart -k "$domain/$net_label"
  launchctl print "$domain/$net_label" >/dev/null
  # Wait briefly for net.sock before starting the control plane.
  state_dir=${VZCTL_STATE_DIR:-"$HOME/Library/Application Support/vzctl"}
  i=0
  while [ "$i" -lt 50 ]; do
    if [ -S "$state_dir/net.sock" ]; then
      break
    fi
    i=$((i + 1))
    sleep 0.1
  done
  launchctl bootstrap "$domain" "$supervisor_plist_path"
  launchctl kickstart -k "$domain/$supervisor_label"
  launchctl print "$domain/$supervisor_label" >/dev/null
fi

printf '%s: %s\n' "$action" "$bindir/vzctl"
printf '%s: %s\n' "$action" "$bindir/vz-net"
printf '%s: %s\n' "$action" "$bindir/vz-supervisor"
printf '%s: %s\n' "$action" "$bindir/vz-helper"
if [ -n "$dns_bind_source" ]; then
  printf '%s: %s\n' "$action" "$bindir/vz-dns-bind"
fi
printf 'launch agent: %s\n' "$net_plist_path"
printf 'launch agent: %s\n' "$supervisor_plist_path"
if [ "$activate" = 1 ]; then
  printf 'restarted: %s/%s\n' "$domain" "$net_label"
  printf 'restarted: %s/%s\n' "$domain" "$supervisor_label"
else
  printf 'activation skipped (ACTIVATE=0)\n'
fi
printf 'note: guest DNS :53 needs: sudo vzctl dns install-bind-helper\n'
