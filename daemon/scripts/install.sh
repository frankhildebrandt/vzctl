#!/bin/sh
set -eu

if [ "$#" -lt 5 ] || [ "$#" -gt 6 ]; then
  echo "usage: install.sh <vzctl> <vz-net> <vz-edge> <vz-supervisor> <vz-helper> [vz-dns-bind]" >&2
  exit 2
fi

cli_source=$1
net_source=$2
edge_source=$3
supervisor_source=$4
helper_source=$5
dns_bind_source=${6:-}
prefix=${PREFIX:-"$HOME/.local"}
bindir=${BINDIR:-"$prefix/bin"}
launch_agents_dir=${LAUNCH_AGENTS_DIR:-"$HOME/Library/LaunchAgents"}
log_dir=${LOG_DIR:-"$HOME/Library/Logs/vzctl"}
activate=${ACTIVATE:-1}
net_label=com.vzctl.net
edge_label=com.vzctl.edge
supervisor_label=com.vzctl.supervisor
net_plist_path="$launch_agents_dir/$net_label.plist"
edge_plist_path="$launch_agents_dir/$edge_label.plist"
supervisor_plist_path="$launch_agents_dir/$supervisor_label.plist"
domain="gui/$(id -u)"
action=installed
script_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
stop_helpers="$script_dir/stop-vz-helpers.sh"

case "$activate" in
  0|1) ;;
  *)
    echo "ACTIVATE must be 0 or 1" >&2
    exit 2
    ;;
esac

for binary in "$cli_source" "$net_source" "$edge_source" "$supervisor_source" "$helper_source"; do
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
if [ -e "$bindir/vzctl" ] || [ -e "$bindir/vz-net" ] || [ -e "$bindir/vz-edge" ] \
    || [ -e "$bindir/vz-supervisor" ] || [ -e "$bindir/vz-helper" ] \
    || [ -e "$net_plist_path" ] || [ -e "$edge_plist_path" ] \
    || [ -e "$supervisor_plist_path" ]; then
  action=updated
fi
if [ -n "$dns_bind_source" ] && [ -e "$bindir/vz-dns-bind" ]; then
  action=updated
fi

stage_dir=$(mktemp -d "$bindir/.vzctl-install.XXXXXX")
net_plist_tmp=$(mktemp "$launch_agents_dir/.com.vzctl.net.XXXXXX")
edge_plist_tmp=$(mktemp "$launch_agents_dir/.com.vzctl.edge.XXXXXX")
supervisor_plist_tmp=$(mktemp "$launch_agents_dir/.com.vzctl.supervisor.XXXXXX")
cleanup() {
  rm -rf "$stage_dir"
  rm -f "$net_plist_tmp" "$edge_plist_tmp" "$supervisor_plist_tmp"
}
trap cleanup EXIT INT TERM

install -m 0755 "$cli_source" "$stage_dir/vzctl"
install -m 0755 "$net_source" "$stage_dir/vz-net"
install -m 0755 "$edge_source" "$stage_dir/vz-edge"
install -m 0755 "$supervisor_source" "$stage_dir/vz-supervisor"
install -m 0755 "$helper_source" "$stage_dir/vz-helper"
if [ -n "$dns_bind_source" ]; then
  install -m 0755 "$dns_bind_source" "$stage_dir/vz-dns-bind"
fi
codesign --verify --strict "$stage_dir/vz-net"
codesign --verify --strict "$stage_dir/vz-edge"
codesign --verify --strict "$stage_dir/vz-supervisor"
codesign --verify --strict "$stage_dir/vz-helper"
"$stage_dir/vzctl" version >/dev/null
"$stage_dir/vz-net" version >/dev/null
"$stage_dir/vz-edge" version >/dev/null
"$stage_dir/vz-supervisor" version >/dev/null
"$stage_dir/vz-helper" version >/dev/null
if [ -n "$dns_bind_source" ]; then
  "$stage_dir/vz-dns-bind" version >/dev/null
fi

# LaunchAgents default to PATH=/usr/bin:/bin:/usr/sbin:/sbin. Apply jobs spawn
# vzctl which needs docker/ssh helpers from Homebrew and ~/.local/bin.
agent_path="${bindir}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

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
  plutil -insert EnvironmentVariables -dictionary "$plist"
  plutil -insert EnvironmentVariables.PATH -string "$agent_path" "$plist"
  plutil -insert StandardOutPath -string "$log_dir/${log_name}.log" "$plist"
  plutil -insert StandardErrorPath -string "$log_dir/${log_name}.error.log" "$plist"
  plutil -lint "$plist"
  chmod 0644 "$plist"
}

write_agent_plist "$net_plist_tmp" "$net_label" "$bindir/vz-net" serve net
write_agent_plist "$edge_plist_tmp" "$edge_label" "$bindir/vz-edge" serve edge
write_agent_plist "$supervisor_plist_tmp" "$supervisor_label" "$bindir/vz-supervisor" serve supervisor

state_dir=${VZCTL_STATE_DIR:-"$HOME/Library/Application Support/vzctl"}
net_sock="$state_dir/net.sock"
edge_sock="$state_dir/edge.sock"
stopped_services=0

agent_loaded() {
  launchctl print "$domain/$1" >/dev/null 2>&1
}

wait_agent_gone() {
  label=$1
  timeout_tenths=$2
  i=0
  while [ "$i" -lt "$timeout_tenths" ]; do
    if ! agent_loaded "$label"; then
      return 0
    fi
    i=$((i + 1))
    sleep 0.1
  done
  return 1
}

wait_path_gone() {
  path=$1
  timeout_tenths=$2
  i=0
  while [ "$i" -lt "$timeout_tenths" ]; do
    if [ ! -e "$path" ]; then
      return 0
    fi
    i=$((i + 1))
    sleep 0.1
  done
  return 1
}

wait_sock() {
  path=$1
  timeout_tenths=$2
  i=0
  while [ "$i" -lt "$timeout_tenths" ]; do
    if [ -S "$path" ]; then
      return 0
    fi
    i=$((i + 1))
    sleep 0.1
  done
  return 1
}

# Stop dependents before replacing binaries. vz-net must exit via SIGTERM
# (launchctl bootout) so vmnet refs release cleanly — never SIGKILL/-k.
if [ "$(uname -s)" = Darwin ]; then
  if [ ! -x "$stop_helpers" ]; then
    echo "missing executable: $stop_helpers" >&2
    exit 3
  fi
  # Helper-side refs become unusable when vz-net recreates the network. Stop
  # every VM first; refuse the install if graceful shutdown does not finish.
  "$stop_helpers" "$(id -u)"
  if agent_loaded "$supervisor_label" || agent_loaded "$edge_label" || agent_loaded "$net_label"; then
    printf 'stopping running agents before install…\n'
  fi
  if agent_loaded "$supervisor_label"; then
    launchctl bootout "$domain/$supervisor_label" >/dev/null 2>&1 || true
    if ! wait_agent_gone "$supervisor_label" 100; then
      echo "error: $supervisor_label still loaded after bootout" >&2
      exit 5
    fi
    stopped_services=1
  fi
  if agent_loaded "$edge_label"; then
    launchctl bootout "$domain/$edge_label" >/dev/null 2>&1 || true
    if ! wait_agent_gone "$edge_label" 100; then
      echo "error: $edge_label still loaded after bootout" >&2
      exit 5
    fi
    stopped_services=1
  fi
  if agent_loaded "$net_label"; then
    printf 'stopping vz-net gracefully (vmnet ref release)…\n'
    launchctl bootout "$domain/$net_label" >/dev/null 2>&1 || true
    # NativeVmnetHandle.deinit may take ~5s per interface; allow generous time.
    if ! wait_agent_gone "$net_label" 600; then
      echo "error: vz-net did not exit cleanly after bootout; refusing SIGKILL (would orphan CIDRs until reboot)" >&2
      exit 5
    fi
    if ! wait_path_gone "$net_sock" 50; then
      echo "warn: $net_sock still present after vz-net exit; removing stale socket" >&2
      rm -f "$net_sock"
    fi
    # Ensure no stray vz-net from a previous manual run holds refs.
    if pgrep -x vz-net >/dev/null 2>&1; then
      echo "error: vz-net process still running after launchd bootout; refuse install to avoid orphaned CIDRs" >&2
      exit 5
    fi
    printf 'vz-net stopped cleanly\n'
    stopped_services=1
  elif [ -S "$net_sock" ]; then
    echo "warn: stale $net_sock without loaded $net_label; removing" >&2
    rm -f "$net_sock"
  fi
fi

mv -f "$stage_dir/vzctl" "$bindir/vzctl"
mv -f "$stage_dir/vz-net" "$bindir/vz-net"
mv -f "$stage_dir/vz-edge" "$bindir/vz-edge"
mv -f "$stage_dir/vz-supervisor" "$bindir/vz-supervisor"
mv -f "$stage_dir/vz-helper" "$bindir/vz-helper"
if [ -n "$dns_bind_source" ]; then
  mv -f "$stage_dir/vz-dns-bind" "$bindir/vz-dns-bind"
fi
mv -f "$net_plist_tmp" "$net_plist_path"
mv -f "$edge_plist_tmp" "$edge_plist_path"
mv -f "$supervisor_plist_tmp" "$supervisor_plist_path"

if [ "$activate" = 1 ]; then
  if [ "$(uname -s)" != Darwin ]; then
    echo "launchd activation requires macOS" >&2
    exit 4
  fi
  # Start order: vz-net → vz-edge → vz-supervisor. Never kickstart -k on vz-net.
  launchctl bootstrap "$domain" "$net_plist_path"
  launchctl kickstart "$domain/$net_label"
  launchctl print "$domain/$net_label" >/dev/null
  if ! wait_sock "$net_sock" 100; then
    echo "error: net.sock not ready after starting vz-net" >&2
    exit 5
  fi
  launchctl bootstrap "$domain" "$edge_plist_path"
  launchctl kickstart -k "$domain/$edge_label"
  launchctl print "$domain/$edge_label" >/dev/null
  if ! wait_sock "$edge_sock" 100; then
    echo "error: edge.sock not ready after starting vz-edge" >&2
    exit 5
  fi
  launchctl bootstrap "$domain" "$supervisor_plist_path"
  launchctl kickstart -k "$domain/$supervisor_label"
  launchctl print "$domain/$supervisor_label" >/dev/null
elif [ "$stopped_services" = 1 ]; then
  printf 'activation skipped (ACTIVATE=0); agents were stopped and left down\n'
fi

printf '%s: %s\n' "$action" "$bindir/vzctl"
printf '%s: %s\n' "$action" "$bindir/vz-net"
printf '%s: %s\n' "$action" "$bindir/vz-edge"
printf '%s: %s\n' "$action" "$bindir/vz-supervisor"
printf '%s: %s\n' "$action" "$bindir/vz-helper"
if [ -n "$dns_bind_source" ]; then
  printf '%s: %s\n' "$action" "$bindir/vz-dns-bind"
fi
printf 'launch agent: %s\n' "$net_plist_path"
printf 'launch agent: %s\n' "$edge_plist_path"
printf 'launch agent: %s\n' "$supervisor_plist_path"
if [ "$activate" = 1 ]; then
  printf 'started: %s/%s\n' "$domain" "$net_label"
  printf 'started: %s/%s\n' "$domain" "$edge_label"
  printf 'started: %s/%s\n' "$domain" "$supervisor_label"
elif [ "$stopped_services" != 1 ]; then
  printf 'activation skipped (ACTIVATE=0)\n'
fi
printf 'note: guest DNS :53 needs: sudo vzctl dns install-bind-helper\n'
