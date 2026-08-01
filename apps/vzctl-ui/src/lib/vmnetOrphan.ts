import type { Environment } from "@/domain/hypernetwork/schema";
import {
  cidrsOverlap,
  parseCidr,
} from "@/application/validation/topology";

export type VmnetOrphanInfo = {
  cidr: string;
  message: string;
};

const ORPHAN_RE =
  /vmnet reserve\s+(\d{1,3}(?:\.\d{1,3}){3}\/\d{1,2})\s+failed\s*\((1001|1002)\)/i;
const ORPHAN_HINT_RE = /orphaned until reboot/i;

export function parseVmnetOrphanError(error: unknown): VmnetOrphanInfo | null {
  const message = String(error ?? "");
  const match = message.match(ORPHAN_RE);
  if (!match?.[1]) return null;
  if (!ORPHAN_HINT_RE.test(message) && !/\(100[12]\)/.test(message)) return null;
  return { cidr: match[1], message };
}

export function suggestReplacementCidr(
  orphaned: string,
  used: Iterable<string>,
): string {
  const parsed = parseCidr(orphaned);
  if (!parsed) {
    throw new Error(`Ungültiges verwaiste CIDR: ${orphaned}`);
  }
  const usedList = [...used];
  // Prefer bumping the third octet for /24-style nets.
  const start = parsed.network;
  for (let step = 1; step < 200; step++) {
    const candidateNet = (start + (step << 8)) >>> 0;
    const candidate = intToCidr(candidateNet, parsed.prefix);
    if (usedList.some((c) => c === candidate || cidrsOverlap(c, candidate))) {
      continue;
    }
    return candidate;
  }
  throw new Error(`Keine freie Ersatz-CIDR für ${orphaned}`);
}

/** Remap network CIDR and keep host offsets for NIC IPs in that net. */
export function remapNetworkCidr(
  env: Environment,
  networkName: string,
  newCidr: string,
): Environment {
  const next = structuredClone(env);
  const net = next.spec.networks[networkName];
  if (!net) throw new Error(`Netz ${networkName} fehlt`);
  const oldCidr = net.cidr;
  const oldParsed = parseCidr(oldCidr);
  const newParsed = parseCidr(newCidr);
  if (!oldParsed || !newParsed) {
    throw new Error(`Ungültiges CIDR-Remap ${oldCidr} → ${newCidr}`);
  }
  if (oldParsed.prefix !== newParsed.prefix) {
    throw new Error(
      `Prefix muss gleich bleiben (${oldParsed.prefix} → ${newParsed.prefix})`,
    );
  }
  net.cidr = newCidr;
  for (const vm of Object.values(next.spec.vms)) {
    for (const nic of vm.networks) {
      if (nic.name !== networkName) continue;
      nic.ip = remapHostIp(nic.ip, oldParsed.network, newParsed.network);
    }
  }
  return next;
}

export function findNetworksByCidr(
  env: Environment,
  cidr: string,
): string[] {
  return Object.entries(env.spec.networks)
    .filter(([, net]) => net.cidr === cidr)
    .map(([name]) => name);
}

function remapHostIp(ip: string, oldNetwork: number, newNetwork: number): string {
  const parts = ip.split(".").map(Number);
  if (parts.length !== 4 || parts.some((n) => !Number.isInteger(n))) {
    return ip;
  }
  const addr =
    ((parts[0]! << 24) >>> 0) +
    (parts[1]! << 16) +
    (parts[2]! << 8) +
    parts[3]!;
  const offset = (addr - oldNetwork) >>> 0;
  return intToIp((newNetwork + offset) >>> 0);
}

function intToIp(value: number): string {
  return [
    (value >>> 24) & 255,
    (value >>> 16) & 255,
    (value >>> 8) & 255,
    value & 255,
  ].join(".");
}

function intToCidr(network: number, prefix: number): string {
  return `${intToIp(network)}/${prefix}`;
}
