import type { Environment } from "@/domain/hypernetwork/schema";

export type ValidationIssue = {
  id: string;
  severity: "info" | "warning" | "error";
  code: string;
  message: string;
  nodeId?: string;
  connectionId?: string;
  policyName?: string;
  ruleIndex?: number;
};

function parseIpv4(ip: string): number[] | null {
  const parts = ip.split(".");
  if (parts.length !== 4) return null;
  const nums = parts.map((p) => Number(p));
  if (nums.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return null;
  return nums;
}

function ipv4ToInt(parts: number[]): number {
  return ((parts[0]! << 24) >>> 0) + (parts[1]! << 16) + (parts[2]! << 8) + parts[3]!;
}

export function parseCidr(cidr: string): { network: number; mask: number; prefix: number } | null {
  const [ip, prefixRaw] = cidr.split("/");
  if (!ip || prefixRaw === undefined) return null;
  const prefix = Number(prefixRaw);
  if (!Number.isInteger(prefix) || prefix < 0 || prefix > 32) return null;
  const parts = parseIpv4(ip);
  if (!parts) return null;
  const mask = prefix === 0 ? 0 : (~0 << (32 - prefix)) >>> 0;
  const network = ipv4ToInt(parts) & mask;
  return { network, mask, prefix };
}

export function ipInCidr(ip: string, cidr: string): boolean {
  const parsed = parseCidr(cidr);
  const parts = parseIpv4(ip);
  if (!parsed || !parts) return false;
  return (ipv4ToInt(parts) & parsed.mask) === parsed.network;
}

export function isReservedHostOffset(ip: string, cidr: string): boolean {
  const parsed = parseCidr(cidr);
  const parts = parseIpv4(ip);
  if (!parsed || !parts) return false;
  const addr = ipv4ToInt(parts);
  const network = parsed.network;
  const broadcast = network | (~parsed.mask >>> 0);
  if (addr === network || addr === broadcast) return true;
  // .0 and .1 offsets within /24-style nets (host gateway / reserved)
  const host = addr - network;
  return host === 0 || host === 1;
}

export function cidrsOverlap(a: string, b: string): boolean {
  const left = parseCidr(a);
  const right = parseCidr(b);
  if (!left || !right) return false;
  return (
    (left.network & right.mask) === right.network ||
    (right.network & left.mask) === left.network
  );
}

export function validateEnvironment(env: Environment): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const nets = env.spec.networks;
  const vms = env.spec.vms;
  const netNames = Object.keys(nets);
  const vmNames = Object.keys(vms);

  for (let i = 0; i < netNames.length; i++) {
    for (let j = i + 1; j < netNames.length; j++) {
      const a = netNames[i]!;
      const b = netNames[j]!;
      const ca = nets[a]!.cidr;
      const cb = nets[b]!.cidr;
      if (!parseCidr(ca)) {
        issues.push({
          id: `cidr-invalid-${a}`,
          severity: "error",
          code: "CIDR_INVALID",
          message: `Netz ${a}: ungültiges CIDR ${ca}`,
          nodeId: `net:${a}`,
        });
      }
      if (!parseCidr(cb)) {
        issues.push({
          id: `cidr-invalid-${b}`,
          severity: "error",
          code: "CIDR_INVALID",
          message: `Netz ${b}: ungültiges CIDR ${cb}`,
          nodeId: `net:${b}`,
        });
      }
      if (parseCidr(ca) && parseCidr(cb) && cidrsOverlap(ca, cb)) {
        issues.push({
          id: `cidr-overlap-${a}-${b}`,
          severity: "error",
          code: "CIDR_OVERLAP",
          message: `CIDR von ${a} und ${b} überlappen`,
          nodeId: `net:${a}`,
        });
      }
    }
  }

  const usedIps = new Map<string, string>();
  for (const [vmName, vm] of Object.entries(vms)) {
    if (vm.networks.length === 0) {
      issues.push({
        id: `vm-isolated-${vmName}`,
        severity: "warning",
        code: "VM_NO_NETWORK",
        message: `VM ${vmName} hat kein Netzwerk`,
        nodeId: `vm:${vmName}`,
      });
    }
    for (const nic of vm.networks) {
      const net = nets[nic.name];
      if (!net) {
        issues.push({
          id: `nic-missing-net-${vmName}-${nic.name}`,
          severity: "error",
          code: "NIC_UNKNOWN_NETWORK",
          message: `VM ${vmName}: Netz ${nic.name} fehlt`,
          nodeId: `vm:${vmName}`,
          connectionId: `attach:${vmName}:${nic.name}`,
        });
        continue;
      }
      if (!ipInCidr(nic.ip, net.cidr)) {
        issues.push({
          id: `ip-out-${vmName}-${nic.name}`,
          severity: "error",
          code: "IP_OUT_OF_CIDR",
          message: `VM ${vmName}: IP ${nic.ip} nicht in ${net.cidr}`,
          nodeId: `vm:${vmName}`,
        });
      }
      if (isReservedHostOffset(nic.ip, net.cidr)) {
        issues.push({
          id: `ip-reserved-${vmName}-${nic.name}`,
          severity: "error",
          code: "IP_RESERVED",
          message: `VM ${vmName}: IP ${nic.ip} ist reserviert (.0/.1)`,
          nodeId: `vm:${vmName}`,
        });
      }
      const prev = usedIps.get(`${nic.name}|${nic.ip}`);
      if (prev) {
        issues.push({
          id: `ip-dup-${vmName}-${nic.ip}`,
          severity: "error",
          code: "IP_DUPLICATE",
          message: `Doppelte IP ${nic.ip} auf ${nic.name} (${prev}, ${vmName})`,
          nodeId: `vm:${vmName}`,
        });
      } else {
        usedIps.set(`${nic.name}|${nic.ip}`, vmName);
      }
    }
    for (const role of vm.roles) {
      if (role !== "router" && role !== "docker") {
        issues.push({
          id: `role-${vmName}-${role}`,
          severity: "error",
          code: "ROLE_INVALID",
          message: `VM ${vmName}: unbekannte Role ${role}`,
          nodeId: `vm:${vmName}`,
        });
      }
    }
    for (const dep of vm.dependsOn) {
      if (!vmNames.includes(dep)) {
        issues.push({
          id: `dep-${vmName}-${dep}`,
          severity: "error",
          code: "DEPENDS_UNKNOWN",
          message: `VM ${vmName}: dependsOn ${dep} unbekannt`,
          nodeId: `vm:${vmName}`,
        });
      }
    }
  }

  for (const route of env.spec.routes) {
    if (!nets[route.from]) {
      issues.push({
        id: `route-from-${route.name}`,
        severity: "error",
        code: "ROUTE_UNKNOWN_FROM",
        message: `Route ${route.name}: from ${route.from} unbekannt`,
        connectionId: `route:${route.name}`,
      });
    }
    if (!nets[route.to]) {
      issues.push({
        id: `route-to-${route.name}`,
        severity: "error",
        code: "ROUTE_UNKNOWN_TO",
        message: `Route ${route.name}: to ${route.to} unbekannt`,
        connectionId: `route:${route.name}`,
      });
    }
    const via = vms[route.via];
    if (!via) {
      issues.push({
        id: `route-via-${route.name}`,
        severity: "error",
        code: "ROUTE_UNKNOWN_VIA",
        message: `Route ${route.name}: via ${route.via} unbekannt`,
        connectionId: `route:${route.name}`,
      });
    } else {
      if (!via.roles.includes("router")) {
        issues.push({
          id: `route-via-role-${route.name}`,
          severity: "error",
          code: "ROUTE_VIA_NOT_ROUTER",
          message: `Route ${route.name}: via ${route.via} braucht roles: [router]`,
          connectionId: `route:${route.name}`,
          nodeId: `vm:${route.via}`,
        });
      }
      const attached = new Set(via.networks.map((n) => n.name));
      if (!attached.has(route.from) || !attached.has(route.to)) {
        issues.push({
          id: `route-via-attach-${route.name}`,
          severity: "error",
          code: "ROUTE_VIA_NOT_ATTACHED",
          message: `Route ${route.name}: Router muss an ${route.from} und ${route.to} hängen`,
          connectionId: `route:${route.name}`,
          nodeId: `vm:${route.via}`,
        });
      }
    }
  }

  for (const policy of env.spec.policies) {
    if (!nets[policy.network]) {
      issues.push({
        id: `policy-net-${policy.name}`,
        severity: "error",
        code: "POLICY_UNKNOWN_NETWORK",
        message: `Policy ${policy.name}: Netz ${policy.network} fehlt`,
        policyName: policy.name,
        nodeId: `net:${policy.network}`,
      });
    }
    policy.allow.forEach((rule, idx) => {
      if (rule.to === "internet") {
        const source = policy.network;
        const hasRouter = Object.values(vms).some(
          (vm) =>
            vm.roles.includes("router") &&
            vm.networks.some((n) => n.name === source) &&
            vm.networks.some((n) => nets[n.name]?.natEgress !== false),
        );
        if (!hasRouter) {
          issues.push({
            id: `policy-internet-${policy.name}-${idx}`,
            severity: "error",
            code: "POLICY_INTERNET_NO_ROUTER",
            message: `Policy ${policy.name}: to:internet braucht Router an ${source} mit natEgress-Netz`,
            policyName: policy.name,
            ruleIndex: idx,
            nodeId: `net:${source}`,
          });
        }
      } else if (!nets[rule.to]) {
        issues.push({
          id: `policy-to-${policy.name}-${idx}`,
          severity: "error",
          code: "POLICY_UNKNOWN_TO",
          message: `Policy ${policy.name} Regel ${idx + 1}: Zielnetz ${rule.to} fehlt`,
          policyName: policy.name,
          ruleIndex: idx,
        });
      }
      if ((rule.proto === "tcp" || rule.proto === "udp") && rule.ports.length === 0) {
        issues.push({
          id: `policy-ports-${policy.name}-${idx}`,
          severity: "error",
          code: "POLICY_PORTS_REQUIRED",
          message: `Policy ${policy.name} Regel ${idx + 1}: ${rule.proto} braucht Ports`,
          policyName: policy.name,
          ruleIndex: idx,
        });
      }
      if (rule.proto === "icmp" && rule.ports.length > 0) {
        issues.push({
          id: `policy-icmp-ports-${policy.name}-${idx}`,
          severity: "error",
          code: "POLICY_ICMP_NO_PORTS",
          message: `Policy ${policy.name} Regel ${idx + 1}: ICMP ohne Ports`,
          policyName: policy.name,
          ruleIndex: idx,
        });
      }
    });
  }

  for (const [name, net] of Object.entries(nets)) {
    const attached = Object.values(vms).some((vm) =>
      vm.networks.some((n) => n.name === name),
    );
    if (!attached) {
      issues.push({
        id: `net-empty-${name}`,
        severity: "info",
        code: "NETWORK_EMPTY",
        message: `Netz ${name} hat keine VMs`,
        nodeId: `net:${name}`,
      });
    }
    if (net.dhcp) {
      const staticOnDhcp = Object.entries(vms).filter(([, vm]) =>
        vm.networks.some((n) => n.name === name),
      );
      if (staticOnDhcp.length > 0) {
        issues.push({
          id: `dhcp-static-${name}`,
          severity: "error",
          code: "DHCP_WITH_STATIC",
          message: `Netz ${name}: DHCP und statische IPs unzulässig`,
          nodeId: `net:${name}`,
        });
      }
    }
  }

  return issues;
}
