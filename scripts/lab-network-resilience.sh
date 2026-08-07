#!/usr/bin/env bash
# Opt-in macOS-26 hardware acceptance harness. It never changes LAN/WLAN/VPN
# configuration; the operator performs those transitions outside this script.
set -euo pipefail

if [[ "${VZCTL_NETWORK_LAB:-}" != "1" ]]; then
  echo "SKIP: set VZCTL_NETWORK_LAB=1 on a dedicated macOS-26 lab Mac"
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VZCTL_BIN="${VZCTL_NETWORK_LAB_BIN:-$ROOT/target/debug/vzctl}"
ARTIFACTS="${VZCTL_NETWORK_LAB_ARTIFACTS:-/tmp/vzctl-network-resilience-lab}"
BASELINE="$ARTIFACTS/baseline-networks.json"

mkdir -p "$ARTIFACTS"

require_bin() {
  [[ -x "$VZCTL_BIN" ]] || {
    echo "FAIL: vzctl binary missing: $VZCTL_BIN"
    exit 1
  }
}

snapshot() {
  local label="$1"
  local destination="$ARTIFACTS/$label"
  mkdir -p "$destination"
  "$VZCTL_BIN" net list --format json >"$destination/networks.json"
  "$VZCTL_BIN" doctor --format json >"$destination/doctor.json" || true
  /usr/sbin/netstat -rn -f inet >"$destination/routes.txt"
  /usr/sbin/scutil --dns >"$destination/dns.txt"
  /usr/bin/defaults read NSGlobalDomain AppleLocale >"$destination/locale.txt" 2>/dev/null || true
  /usr/sbin/systemsetup -gettimezone >"$destination/timezone.txt" 2>/dev/null || true
  echo "SNAPSHOT $label $destination"
}

assert_identity() {
  local label="$1"
  local current="$ARTIFACTS/$label/networks.json"
  [[ -f "$BASELINE" ]] || { echo "FAIL: baseline missing"; exit 1; }
  cmp -s "$BASELINE" "$current" || {
    echo "FAIL: network IDs, CIDRs or runtime state changed; inspect $current"
    exit 1
  }
  echo "IDENTITY_OK $label"
}

wait_healthy() {
  local seconds="${1:-30}"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS <= deadline )); do
    if "$VZCTL_BIN" doctor --format json 2>/dev/null | /usr/bin/grep -q '"state":"healthy"'; then
      echo "HEALTHY within ${seconds}s"
      return 0
    fi
    sleep 1
  done
  echo "FAIL: network resilience did not become healthy within ${seconds}s"
  return 1
}

require_bin
case "${1:-}" in
  baseline)
    snapshot baseline
    cp "$ARTIFACTS/baseline/networks.json" "$BASELINE"
    echo "BASELINE_SAVED $BASELINE"
    ;;
  check)
    label="${2:?usage: $0 check LABEL}"
    snapshot "$label"
    assert_identity "$label"
    wait_healthy "${3:-30}"
    ;;
  wait)
    wait_healthy "${2:-30}"
    ;;
  sleep)
    if [[ "${VZCTL_NETWORK_LAB_SLEEP:-}" != "1" ]]; then
      echo "SKIP: set VZCTL_NETWORK_LAB_SLEEP=1 only on the marked lab Mac"
      exit 0
    fi
    /usr/bin/pmset sleepnow
    wait_healthy 30
    snapshot after-wake
    assert_identity after-wake
    ;;
  *)
    echo "usage: $0 baseline | check LABEL [SECONDS] | wait [SECONDS] | sleep"
    exit 2
    ;;
esac
