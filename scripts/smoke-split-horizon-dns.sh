#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
vzctl_bin="${VZCTL_BIN:-$repo_root/target/debug/vzctl}"
if [[ ! -x "$vzctl_bin" ]] && command -v vzctl >/dev/null; then
  vzctl_bin="$(command -v vzctl)"
fi
config="${CONFIG:-$repo_root/examples/edge-dmz}"
project="${PROJECT:-edge-dmz}"
vm_a="${VM_A:-$project/web}"
vm_b="${VM_B:-$project/host}"
network_a="${NETWORK_A:-dmz}"
network_b="${NETWORK_B:-lan}"
ingress_host="${INGRESS_HOST:-web.svc.$project.vz.test}"
foreign_host="${FOREIGN_HOST:-web.svc.foreign.vz.test}"
ingress_port="${INGRESS_PORT:-443}"
blocked_port="${BLOCKED_PORT:-18081}"
docker_image="${DOCKER_IMAGE:-curlimages/curl:8.15.0}"
dns_backend_port="${VZCTL_DNS_GUEST_BACKEND_PORT:-15054}"
pf_anchor="com.apple/vzctl"
service_pid=""
service_log=""

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

cleanup() {
  if [[ -n "$service_pid" ]]; then
    kill "$service_pid" 2>/dev/null || true
    wait "$service_pid" 2>/dev/null || true
  fi
  if [[ -n "$service_log" ]]; then
    rm -f -- "$service_log"
  fi
}
trap cleanup EXIT

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
  fail "als normaler Benutzer starten; das Skript nutzt sudo nur für PF-Lesezugriffe"
fi
for tool in "$vzctl_bin" curl jq python3 ifconfig sudo; do
  if [[ "$tool" == */* ]]; then
    [[ -x "$tool" ]] || fail "nicht ausführbar: $tool"
  else
    command -v "$tool" >/dev/null || fail "Tool fehlt: $tool"
  fi
done
[[ -d "$config" || -f "$config" ]] || fail "Config fehlt: $config"
sudo -v

network_json="$($vzctl_bin net list --format json)"
network_cidr() {
  local name="$1"
  local cidr
  cidr="$(jq -r --arg project "$project" --arg name "$name" '
    [.networks[] | select(.project == $project and .name == $name and .runtime_state == "active") | .cidr]
    | if length == 1 then .[0] else empty end
  ' <<<"$network_json")"
  [[ -n "$cidr" ]] || fail "genau ein aktives vmnet $project/$name erwartet"
  printf '%s\n' "$cidr"
}

cidr_value() {
  python3 - "$1" "$2" <<'PY'
import ipaddress
import sys

network = ipaddress.ip_network(sys.argv[1], strict=True)
if sys.argv[2] == "alias":
    print(network.network_address + 1)
elif sys.argv[2] == "gateway":
    print(network.network_address)
elif sys.argv[2] == "mask":
    print(f"0x{int(network.netmask):08x}")
else:
    raise SystemExit(2)
PY
}

cidr_a="$(network_cidr "$network_a")"
cidr_b="$(network_cidr "$network_b")"
alias_a="$(cidr_value "$cidr_a" alias)"
alias_b="$(cidr_value "$cidr_b" alias)"
gateway_a="$(cidr_value "$cidr_a" gateway)"
gateway_b="$(cidr_value "$cidr_b" gateway)"
mask_a="$(cidr_value "$cidr_a" mask)"
mask_b="$(cidr_value "$cidr_b" mask)"

assert_alias() {
  local address="$1"
  local mask="$2"
  if ! ifconfig | grep -Eq "^[[:space:]]*inet ${address//./\\.} netmask $mask([[:space:]]|$)"; then
    fail "Alias $address mit Netzmaske $mask fehlt"
  fi
}

assert_alias "$alias_a" "$mask_a"
assert_alias "$alias_b" "$mask_b"

pf_rules="$(sudo pfctl -a "$pf_anchor" -sr 2>/dev/null)"
pf_redirects="$(sudo pfctl -a "$pf_anchor" -sn 2>/dev/null)"
for address in "$alias_a" "$alias_b"; do
  grep -Fq "to $address" <<<"$pf_rules" || fail "PF-Regeln für $address fehlen"
  grep -Fq "block in quick" <<<"$(grep -F "to $address" <<<"$pf_rules")" \
    || fail "PF-Blockregel für $address fehlt"
done
for gateway in "$gateway_a" "$gateway_b"; do
  grep -F "$gateway" <<<"$pf_redirects" | grep -Fq "$dns_backend_port" \
    || fail "DNS-Redirect $gateway:53 -> $gateway:$dns_backend_port fehlt"
done

host_answers="$($vzctl_bin dns query "$ingress_host" --format json \
  | jq -r '.answers[]? | select(.type == "A") | .data' | sort -u)"
[[ "$host_answers" == "127.0.0.1" ]] \
  || fail "Host-DNS für $ingress_host: erwartet 127.0.0.1, erhalten ${host_answers:-leer}"

for vm in "$vm_a" "$vm_b"; do
  "$vzctl_bin" vm exec "$vm" -- sh -c 'command -v getent >/dev/null && command -v curl >/dev/null' \
    >/dev/null || fail "$vm braucht getent und curl"
done

guest_address() {
  "$vzctl_bin" vm exec "$1" -- getent ahostsv4 "$ingress_host" \
    | awk '{ print $1 }' | sort -u
}

[[ "$(guest_address "$vm_a")" == "$alias_a" ]] \
  || fail "$vm_a erhält nicht ausschließlich $alias_a"
[[ "$(guest_address "$vm_b")" == "$alias_b" ]] \
  || fail "$vm_b erhält nicht ausschließlich $alias_b"
if "$vzctl_bin" vm exec "$vm_a" -- getent ahostsv4 "$foreign_host" >/dev/null 2>&1; then
  fail "projektfremder Ingress-Name $foreign_host wurde in $vm_a aufgelöst"
fi

guest_remote_ip() {
  "$vzctl_bin" vm exec "$1" -- curl -ksS --connect-timeout 5 \
    -o /dev/null -w '%{remote_ip}' "https://$ingress_host:$ingress_port/"
}
[[ "$(guest_remote_ip "$vm_a")" == "$alias_a" ]] || fail "$vm_a erreicht Ingress nicht über $alias_a"
[[ "$(guest_remote_ip "$vm_b")" == "$alias_b" ]] || fail "$vm_b erreicht Ingress nicht über $alias_b"

service_log="$(mktemp "${TMPDIR:-/tmp}/vzctl-split-dns-smoke.XXXXXX")"
python3 -m http.server "$blocked_port" --bind 0.0.0.0 >"$service_log" 2>&1 &
service_pid="$!"
for _ in {1..20}; do
  curl -sS --connect-timeout 1 -o /dev/null "http://127.0.0.1:$blocked_port/" && break
  sleep 0.1
done
curl -sS --connect-timeout 1 -o /dev/null "http://127.0.0.1:$blocked_port/" \
  || fail "Host-Testdienst auf Port $blocked_port ist nicht gestartet"
for vm_and_alias in "$vm_a:$alias_a" "$vm_b:$alias_b"; do
  vm="${vm_and_alias%%:*}"
  address="${vm_and_alias##*:}"
  if "$vzctl_bin" vm exec "$vm" -- curl -sS --connect-timeout 2 \
    -o /dev/null "http://$address:$blocked_port/" >/dev/null 2>&1; then
    fail "$vm erreicht gesperrten Host-Port $address:$blocked_port"
  fi
done

docker_remote="$($vzctl_bin docker --project "$project" -- run --rm "$docker_image" \
  -ksS --connect-timeout 5 -o /dev/null -w '%{remote_ip}' \
  "https://$ingress_host:$ingress_port/")"
[[ "$docker_remote" == "$alias_b" ]] \
  || fail "Docker erreicht Ingress nicht über Primary-$alias_b (erhalten: ${docker_remote:-leer})"
if "$vzctl_bin" docker --project "$project" -- run --rm "$docker_image" \
  -sS --connect-timeout 2 -o /dev/null "http://$alias_b:$blocked_port/" >/dev/null 2>&1; then
  fail "Docker erreicht gesperrten Host-Port $alias_b:$blocked_port"
fi

# Zwei identische Apply-Läufe dürfen weder Aliases noch den PF-Anchor verändern.
"$vzctl_bin" apply -C "$config" --format json >/dev/null
aliases_before="$(ifconfig | grep -E "inet (${alias_a//./\\.}|${alias_b//./\\.}) " | sort)"
pf_before="$(sudo pfctl -a "$pf_anchor" -sr 2>/dev/null)"
pf_redirects_before="$(sudo pfctl -a "$pf_anchor" -sn 2>/dev/null)"
"$vzctl_bin" apply -C "$config" --format json >/dev/null
aliases_after="$(ifconfig | grep -E "inet (${alias_a//./\\.}|${alias_b//./\\.}) " | sort)"
pf_after="$(sudo pfctl -a "$pf_anchor" -sr 2>/dev/null)"
pf_redirects_after="$(sudo pfctl -a "$pf_anchor" -sn 2>/dev/null)"
[[ "$aliases_before" == "$aliases_after" ]] || fail "Alias-Reconcile ist nicht idempotent"
[[ "$pf_before" == "$pf_after" ]] || fail "PF-Reconcile ist nicht idempotent"
[[ "$pf_redirects_before" == "$pf_redirects_after" ]] \
  || fail "PF-DNS-Redirect ist nicht idempotent"

if [[ "${VZCTL_SMOKE_CRASH_HELPER:-0}" == "1" ]]; then
  sudo launchctl kill SIGKILL system/com.vzctl.dns-bind
  pf_after_crash="$(sudo pfctl -a "$pf_anchor" -sr 2>/dev/null)"
  pf_redirects_after_crash="$(sudo pfctl -a "$pf_anchor" -sn 2>/dev/null)"
  [[ "$pf_after_crash" == "$pf_after" ]] || fail "PF-Regeln gingen beim Helper-Crash verloren"
  [[ "$pf_redirects_after_crash" == "$pf_redirects_after" ]] \
    || fail "PF-DNS-Redirect ging beim Helper-Crash verloren"
  for _ in {1..20}; do
    [[ -S /var/run/vzctl/dns-bind.sock ]] && break
    sleep 0.25
  done
  [[ -S /var/run/vzctl/dns-bind.sock ]] || fail "dns-bind wurde nach Crash nicht neu gestartet"
  "$vzctl_bin" apply -C "$config" --format json >/dev/null
  assert_alias "$alias_a" "$mask_a"
  assert_alias "$alias_b" "$mask_b"
fi

echo "split-horizon Multi-Net/Docker smoke passed"
