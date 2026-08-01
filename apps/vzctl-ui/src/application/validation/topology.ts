import type { Environment } from "@/domain/hypernetwork/schema";
import type { MessageParams } from "@/lib/i18n";
import { validationIssue } from "@/application/validation/formatIssue";

export type ValidationIssue = {
  id: string;
  severity: "info" | "warning" | "error";
  code: string;
  message: string;
  params?: MessageParams;
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
        issues.push(
          validationIssue({
            id: `cidr-invalid-${a}`,
            severity: "error",
            code: "CIDR_INVALID",
            params: { name: a, cidr: ca },
            nodeId: `net:${a}`,
          }),
        );
      }
      if (!parseCidr(cb)) {
        issues.push(
          validationIssue({
            id: `cidr-invalid-${b}`,
            severity: "error",
            code: "CIDR_INVALID",
            params: { name: b, cidr: cb },
            nodeId: `net:${b}`,
          }),
        );
      }
      if (parseCidr(ca) && parseCidr(cb) && cidrsOverlap(ca, cb)) {
        issues.push(
          validationIssue({
            id: `cidr-overlap-${a}-${b}`,
            severity: "error",
            code: "CIDR_OVERLAP",
            params: { a, b },
            nodeId: `net:${a}`,
          }),
        );
      }
    }
  }

  for (const [netName, net] of Object.entries(nets)) {
    if (net.backend !== "docker") continue;
    if (net.dhcp) {
      issues.push(
        validationIssue({
          id: `docker-backend-dhcp-${netName}`,
          severity: "error",
          code: "DOCKER_BACKEND_DHCP",
          params: { name: netName },
          nodeId: `net:${netName}`,
        }),
      );
    }
    if (net.natEgress !== false) {
      issues.push(
        validationIssue({
          id: `docker-backend-nat-${netName}`,
          severity: "error",
          code: "DOCKER_BACKEND_NAT",
          params: { name: netName },
          nodeId: `net:${netName}`,
        }),
      );
    }
    const owners = Object.entries(vms).filter(([, vm]) =>
      vm.networks.some((nic) => nic.name === netName),
    );
    if (owners.length !== 1) {
      issues.push(
        validationIssue({
          id: `docker-backend-owner-${netName}`,
          severity: "error",
          code: "DOCKER_BACKEND_OWNER",
          params: { name: netName, count: owners.length },
          nodeId: `net:${netName}`,
        }),
      );
    }
  }

  const usedIps = new Map<string, string>();
  for (const [vmName, vm] of Object.entries(vms)) {
    if (vm.networks.length === 0) {
      issues.push(
        validationIssue({
          id: `vm-isolated-${vmName}`,
          severity: "warning",
          code: "VM_NO_NETWORK",
          params: { name: vmName },
          nodeId: `vm:${vmName}`,
        }),
      );
    }
    const attachesDockerBackend = vm.networks.some(
      (nic) => nets[nic.name]?.backend === "docker",
    );
    const hasVmnetAttachment = vm.networks.some(
      (nic) => nets[nic.name] && nets[nic.name]!.backend !== "docker",
    );
    if (attachesDockerBackend && !hasVmnetAttachment) {
      issues.push(
        validationIssue({
          id: `docker-backend-parent-${vmName}`,
          severity: "error",
          code: "DOCKER_BACKEND_NEEDS_VMNET",
          params: { name: vmName },
          nodeId: `vm:${vmName}`,
        }),
      );
    }
    if (attachesDockerBackend) {
      const hasDocker = vm.roles.includes("docker");
      const hasRouter = vm.roles.includes("router");
      if (!hasDocker || !hasRouter) {
        issues.push(
          validationIssue({
            id: `docker-backend-roles-${vmName}`,
            severity: "error",
            code: "DOCKER_BACKEND_ROLES",
            params: { name: vmName },
            nodeId: `vm:${vmName}`,
          }),
        );
      }
    }
    for (const nic of vm.networks) {
      const net = nets[nic.name];
      if (!net) {
        issues.push(
          validationIssue({
            id: `nic-missing-net-${vmName}-${nic.name}`,
            severity: "error",
            code: "NIC_UNKNOWN_NETWORK",
            params: { vm: vmName, net: nic.name },
            nodeId: `vm:${vmName}`,
            connectionId: `attach:${vmName}:${nic.name}`,
          }),
        );
        continue;
      }
      if (!ipInCidr(nic.ip, net.cidr)) {
        issues.push(
          validationIssue({
            id: `ip-out-${vmName}-${nic.name}`,
            severity: "error",
            code: "IP_OUT_OF_CIDR",
            params: { vm: vmName, ip: nic.ip, cidr: net.cidr },
            nodeId: `vm:${vmName}`,
          }),
        );
      }
      if (isReservedHostOffset(nic.ip, net.cidr)) {
        issues.push(
          validationIssue({
            id: `ip-reserved-${vmName}-${nic.name}`,
            severity: "error",
            code: "IP_RESERVED",
            params: { vm: vmName, ip: nic.ip },
            nodeId: `vm:${vmName}`,
          }),
        );
      }
      if (net.backend === "docker") {
        const parsed = parseCidr(net.cidr);
        const parts = nic.ip.split(".").map(Number);
        if (parsed && parts.length === 4) {
          const host =
            (((parts[0]! << 24) >>> 0) +
              (parts[1]! << 16) +
              (parts[2]! << 8) +
              parts[3]!) -
            parsed.network;
          if (host !== 2) {
            issues.push(
              validationIssue({
                id: `docker-bip-${vmName}-${nic.name}`,
                severity: "error",
                code: "DOCKER_BACKEND_BIP",
                params: { vm: vmName, net: nic.name },
                nodeId: `vm:${vmName}`,
              }),
            );
          }
        }
      }
      const prev = usedIps.get(`${nic.name}|${nic.ip}`);
      if (prev) {
        issues.push(
          validationIssue({
            id: `ip-dup-${vmName}-${nic.ip}`,
            severity: "error",
            code: "IP_DUPLICATE",
            params: { ip: nic.ip, net: nic.name, a: prev, b: vmName },
            nodeId: `vm:${vmName}`,
          }),
        );
      } else {
        usedIps.set(`${nic.name}|${nic.ip}`, vmName);
      }
    }
    for (const role of vm.roles) {
      if (role !== "router" && role !== "docker") {
        issues.push(
          validationIssue({
            id: `role-${vmName}-${role}`,
            severity: "error",
            code: "ROLE_INVALID",
            params: { vm: vmName, role },
            nodeId: `vm:${vmName}`,
          }),
        );
      }
    }
    for (const dep of vm.dependsOn) {
      if (!vmNames.includes(dep)) {
        issues.push(
          validationIssue({
            id: `dep-${vmName}-${dep}`,
            severity: "error",
            code: "DEPENDS_UNKNOWN",
            params: { vm: vmName, dep },
            nodeId: `vm:${vmName}`,
          }),
        );
      }
    }
  }

  for (const route of env.spec.routes) {
    if (!nets[route.from]) {
      issues.push(
        validationIssue({
          id: `route-from-${route.name}`,
          severity: "error",
          code: "ROUTE_UNKNOWN_FROM",
          params: { name: route.name, net: route.from },
          connectionId: `route:${route.name}`,
        }),
      );
    }
    if (!nets[route.to]) {
      issues.push(
        validationIssue({
          id: `route-to-${route.name}`,
          severity: "error",
          code: "ROUTE_UNKNOWN_TO",
          params: { name: route.name, net: route.to },
          connectionId: `route:${route.name}`,
        }),
      );
    }
    const via = vms[route.via];
    if (!via) {
      issues.push(
        validationIssue({
          id: `route-via-${route.name}`,
          severity: "error",
          code: "ROUTE_UNKNOWN_VIA",
          params: { name: route.name, via: route.via },
          connectionId: `route:${route.name}`,
        }),
      );
    } else {
      if (!via.roles.includes("router")) {
        issues.push(
          validationIssue({
            id: `route-via-role-${route.name}`,
            severity: "error",
            code: "ROUTE_VIA_NOT_ROUTER",
            params: { name: route.name, via: route.via },
            connectionId: `route:${route.name}`,
            nodeId: `vm:${route.via}`,
          }),
        );
      }
      const attached = new Set(via.networks.map((n) => n.name));
      if (!attached.has(route.from) || !attached.has(route.to)) {
        issues.push(
          validationIssue({
            id: `route-via-attach-${route.name}`,
            severity: "error",
            code: "ROUTE_VIA_NOT_ATTACHED",
            params: { name: route.name, from: route.from, to: route.to },
            connectionId: `route:${route.name}`,
            nodeId: `vm:${route.via}`,
          }),
        );
      }
    }
  }

  for (const policy of env.spec.policies) {
    if (!nets[policy.network]) {
      issues.push(
        validationIssue({
          id: `policy-net-${policy.name}`,
          severity: "error",
          code: "POLICY_UNKNOWN_NETWORK",
          params: { name: policy.name, net: policy.network },
          policyName: policy.name,
          nodeId: `net:${policy.network}`,
        }),
      );
    }
    policy.allow.forEach((rule, idx) => {
      if (rule.to === "internet") {
        const source = policy.network;
        const sourceIsDocker = nets[source]?.backend === "docker";
        const hasRouter = Object.values(vms).some(
          (vm) =>
            vm.roles.includes("router") &&
            vm.networks.some((n) => n.name === source) &&
            vm.networks.some((n) => nets[n.name]?.natEgress !== false),
        );
        if (!hasRouter && !sourceIsDocker) {
          issues.push(
            validationIssue({
              id: `policy-internet-${policy.name}-${idx}`,
              severity: "error",
              code: "POLICY_INTERNET_NO_ROUTER",
              params: { name: policy.name, net: source },
              policyName: policy.name,
              ruleIndex: idx,
              nodeId: `net:${source}`,
            }),
          );
        }
      } else if (!nets[rule.to]) {
        issues.push(
          validationIssue({
            id: `policy-to-${policy.name}-${idx}`,
            severity: "error",
            code: "POLICY_UNKNOWN_TO",
            params: { name: policy.name, i: idx + 1, net: rule.to },
            policyName: policy.name,
            ruleIndex: idx,
          }),
        );
      }
      if ((rule.proto === "tcp" || rule.proto === "udp") && rule.ports.length === 0) {
        issues.push(
          validationIssue({
            id: `policy-ports-${policy.name}-${idx}`,
            severity: "error",
            code: "POLICY_PORTS_REQUIRED",
            params: { name: policy.name, i: idx + 1, proto: rule.proto },
            policyName: policy.name,
            ruleIndex: idx,
          }),
        );
      }
      if (rule.proto === "icmp" && rule.ports.length > 0) {
        issues.push(
          validationIssue({
            id: `policy-icmp-ports-${policy.name}-${idx}`,
            severity: "error",
            code: "POLICY_ICMP_NO_PORTS",
            params: { name: policy.name, i: idx + 1 },
            policyName: policy.name,
            ruleIndex: idx,
          }),
        );
      }
    });
  }

  for (const [name, net] of Object.entries(nets)) {
    const attached = Object.values(vms).some((vm) =>
      vm.networks.some((n) => n.name === name),
    );
    if (!attached) {
      issues.push(
        validationIssue({
          id: `net-empty-${name}`,
          severity: "info",
          code: "NETWORK_EMPTY",
          params: { name },
          nodeId: `net:${name}`,
        }),
      );
    }
    if (net.dhcp) {
      const staticOnDhcp = Object.entries(vms).filter(([, vm]) =>
        vm.networks.some((n) => n.name === name),
      );
      if (staticOnDhcp.length > 0) {
        issues.push(
          validationIssue({
            id: `dhcp-static-${name}`,
            severity: "error",
            code: "DHCP_WITH_STATIC",
            params: { name },
            nodeId: `net:${name}`,
          }),
        );
      }
    }
  }

  return issues;
}
