import type {
  Environment,
  AllowRule,
  NetworkBackend,
  NetworkMode,
  Protocol,
} from "@/domain/hypernetwork/schema";
import type { DiagramState } from "@/domain/diagram/types";
import { networkCellId, vmCellId, attachmentEdgeId } from "@/domain/hypernetwork/ids";
import { validateEnvironment, type ValidationIssue } from "@/application/validation/topology";
import { ipInCidr, parseCidr } from "@/application/validation/topology";
import { getT, type MessageKey, type MessageParams } from "@/lib/i18n";

export type ConnectionHint = {
  key: MessageKey;
  params?: MessageParams;
} | null;

export type EditorSnapshot = {
  env: Environment;
  diagram: DiagramState;
};

export type SelectionState = {
  nodeIds: string[];
  edgeIds: string[];
};

export type EditorUiState = {
  dirty: boolean;
  projecting: boolean;
  paletteFilter: string;
  lastError: string | null;
  connectionHint: ConnectionHint;
};

function mutationError(key: MessageKey, params?: MessageParams): never {
  throw new Error(getT()(key, params));
}

function cloneEnv(env: Environment): Environment {
  return structuredClone(env);
}

function cloneDiagram(diagram: DiagramState): DiagramState {
  return structuredClone(diagram);
}

function hostOffsetIp(
  cidr: string,
  host: number,
): string | null {
  const parsed = parseCidr(cidr);
  if (!parsed) return null;
  const ipNum = (parsed.network + host) >>> 0;
  return [
    (ipNum >>> 24) & 255,
    (ipNum >>> 16) & 255,
    (ipNum >>> 8) & 255,
    ipNum & 255,
  ].join(".");
}

/** Next free guest IP (.10+), or .2 for docker-backend networks. */
function nextIp(env: Environment, networkName: string): string {
  const net = env.spec.networks[networkName];
  if (!net) mutationError("topo.mutation.netMissing", { name: networkName });
  const parsed = parseCidr(net.cidr);
  if (!parsed) mutationError("topo.mutation.invalidCidr", { cidr: net.cidr });
  const used = new Set<string>();
  for (const vm of Object.values(env.spec.vms)) {
    for (const nic of vm.networks) {
      if (nic.name === networkName) used.add(nic.ip);
    }
  }
  if (net.backend === "docker") {
    const bip = hostOffsetIp(net.cidr, 2);
    if (!bip || !ipInCidr(bip, net.cidr)) {
      mutationError("topo.mutation.noRouterIp", { name: networkName });
    }
    if (used.has(bip)) {
      mutationError("topo.mutation.dockerOwnerExists", { name: networkName });
    }
    return bip;
  }
  for (let host = 10; host < 254; host++) {
    const ip = hostOffsetIp(net.cidr, host);
    if (ip && !used.has(ip) && ipInCidr(ip, net.cidr)) return ip;
  }
  mutationError("topo.mutation.noFreeIp", { name: networkName });
}

export function applyCreateNetwork(
  snap: EditorSnapshot,
  name: string,
  cidr: string,
  mode: NetworkMode,
  position: { x: number; y: number },
  opts?: {
    natEgress?: boolean;
    withPolicy?: boolean;
    backend?: NetworkBackend;
  },
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  if (env.spec.networks[name]) mutationError("topo.mutation.netExists", { name });
  const backend = opts?.backend ?? "vmnet";
  const natEgress = backend === "docker" ? false : (opts?.natEgress ?? true);
  env.spec.networks[name] = {
    cidr,
    mode,
    dhcp: false,
    natEgress,
    backend,
  };
  diagram.nodes[networkCellId(name)] = {
    x: position.x,
    y: position.y,
    width: 320,
    height: 200,
  };
  if (opts?.withPolicy || !natEgress || backend === "docker") {
    const policyName = `${name}-default`;
    if (!env.spec.policies.some((p) => p.name === policyName || p.network === name)) {
      env.spec.policies.push({
        name: policyName,
        network: name,
        forward: "deny-all",
        allow: [],
      });
    }
  }
  return { env, diagram };
}

export function applyCreateVm(
  snap: EditorSnapshot,
  name: string,
  opts: {
    networkName?: string;
    roles?: Array<"router" | "docker">;
    cpus?: number;
    memory?: string;
    dataDisk?: string;
    position: { x: number; y: number };
  },
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  if (env.spec.vms[name]) mutationError("topo.mutation.vmExists", { name });
  const networks = [];
  if (opts.networkName && env.spec.networks[opts.networkName]) {
    networks.push({
      name: opts.networkName,
      ip: nextIp(env, opts.networkName),
    });
  } else {
    const firstNet = Object.keys(env.spec.networks)[0];
    if (!firstNet) mutationError("topo.mutation.noNetworkFirst");
    networks.push({ name: firstNet, ip: nextIp(env, firstNet) });
  }
  env.spec.vms[name] = {
    from: Object.keys(env.spec.images)[0] ?? "ubuntu-base",
    clone: "linked",
    dataDisk: opts.dataDisk ?? "20G",
    cpus: opts.cpus ?? 2,
    memory: opts.memory ?? "2048MiB",
    networks,
    dependsOn: [],
    roles: opts.roles ?? [],
    requires: [],
    ports: [],
    mounts: [],
  };
  diagram.nodes[vmCellId(name)] = {
    x: opts.position.x,
    y: opts.position.y,
    width: 200,
    height: 100,
  };
  return { env, diagram };
}

export function applyDeleteNodes(
  snap: EditorSnapshot,
  nodeIds: string[],
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  for (const id of nodeIds) {
    if (id.startsWith("net:")) {
      const name = id.slice(4);
      delete env.spec.networks[name];
      delete diagram.nodes[id];
      env.spec.routes = env.spec.routes.filter(
        (r) => r.from !== name && r.to !== name,
      );
      env.spec.policies = env.spec.policies.filter((p) => p.network !== name);
      for (const vm of Object.values(env.spec.vms)) {
        vm.networks = vm.networks.filter((n) => n.name !== name);
      }
      for (const key of Object.keys(diagram.edges)) {
        if (key.includes(`:${name}`) || key.endsWith(`:${name}`)) {
          delete diagram.edges[key];
        }
      }
    } else if (id.startsWith("vm:")) {
      const name = id.slice(3);
      delete env.spec.vms[name];
      delete diagram.nodes[id];
      env.spec.routes = env.spec.routes.filter((r) => r.via !== name);
      for (const vm of Object.values(env.spec.vms)) {
        vm.dependsOn = vm.dependsOn.filter((d) => d !== name);
      }
      for (const key of [...Object.keys(diagram.edges)]) {
        if (key.startsWith(`attach:${name}:`) || key.includes(`:${name}`)) {
          delete diagram.edges[key];
        }
      }
    } else if (id.startsWith("igw:")) {
      delete diagram.nodes[id];
    }
  }
  // Drop VMs that lost all networks
  for (const [vmName, vm] of Object.entries(env.spec.vms)) {
    if (vm.networks.length === 0) {
      delete env.spec.vms[vmName];
      delete diagram.nodes[vmCellId(vmName)];
    }
  }
  return { env, diagram };
}

function placeVmOutsideContainers(
  diagram: DiagramState,
  vmName: string,
): void {
  let maxBottom = 280;
  let minX = 60;
  for (const [id, p] of Object.entries(diagram.nodes)) {
    if (!id.startsWith("net:")) continue;
    maxBottom = Math.max(maxBottom, (p.y ?? 0) + (p.height ?? 200));
    minX = Math.min(minX, p.x ?? 60);
  }
  const prev = diagram.nodes[vmCellId(vmName)];
  diagram.nodes[vmCellId(vmName)] = {
    x: minX,
    y: maxBottom + 48,
    width: prev?.width ?? 200,
    height: prev?.height ?? 100,
  };
}

export function applyAttachNic(
  snap: EditorSnapshot,
  vmName: string,
  networkName: string,
  ip?: string,
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const vm = env.spec.vms[vmName];
  if (!vm) mutationError("topo.mutation.vmMissing", { name: vmName });
  if (!env.spec.networks[networkName]) {
    mutationError("topo.mutation.netMissing", { name: networkName });
  }
  if (vm.networks.some((n) => n.name === networkName)) {
    mutationError("topo.mutation.attachmentExists");
  }
  const becameMulti = vm.networks.length === 1;
  vm.networks.push({
    name: networkName,
    ip: ip ?? nextIp(env, networkName),
  });
  if (becameMulti) {
    placeVmOutsideContainers(diagram, vmName);
    for (const nic of vm.networks) {
      diagram.edges[attachmentEdgeId(vmName, nic.name)] ??= { vertices: [] };
    }
  } else {
    diagram.edges[attachmentEdgeId(vmName, networkName)] = { vertices: [] };
  }
  return { env, diagram };
}

/** Move a NIC from one network to another (port reconnect). */
export function applyReassignNic(
  snap: EditorSnapshot,
  vmName: string,
  fromNetwork: string,
  toNetwork: string,
): EditorSnapshot {
  if (fromNetwork === toNetwork) return snap;
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const vm = env.spec.vms[vmName];
  if (!vm) mutationError("topo.mutation.vmMissing", { name: vmName });
  if (!env.spec.networks[toNetwork]) {
    mutationError("topo.mutation.netMissing", { name: toNetwork });
  }
  if (vm.networks.some((n) => n.name === toNetwork)) {
    mutationError("topo.mutation.attachmentExists");
  }
  const idx = vm.networks.findIndex((n) => n.name === fromNetwork);
  if (idx < 0) mutationError("topo.mutation.interfaceMissing", { name: fromNetwork });
  vm.networks[idx] = {
    name: toNetwork,
    ip: nextIp(env, toNetwork),
  };
  const oldEdge = attachmentEdgeId(vmName, fromNetwork);
  const newEdge = attachmentEdgeId(vmName, toNetwork);
  const prev = diagram.edges[oldEdge];
  delete diagram.edges[oldEdge];
  diagram.edges[newEdge] = prev ?? { vertices: [] };
  env.spec.routes = env.spec.routes.filter((r) => {
    if (r.via !== vmName) return true;
    return r.from !== fromNetwork && r.to !== fromNetwork;
  });
  return { env, diagram };
}

export function applyDetachNic(
  snap: EditorSnapshot,
  vmName: string,
  networkName: string,
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const vm = env.spec.vms[vmName];
  if (!vm) mutationError("topo.mutation.vmMissing", { name: vmName });
  if (vm.networks.length <= 1) {
    mutationError("topo.mutation.lastNic");
  }
  vm.networks = vm.networks.filter((n) => n.name !== networkName);
  delete diagram.edges[attachmentEdgeId(vmName, networkName)];
  if (vm.networks.length === 1) {
    // Single-homed again — primary lives in the container, drop port-edges
    for (const key of Object.keys(diagram.edges)) {
      if (key.startsWith(`attach:${vmName}:`)) delete diagram.edges[key];
    }
  }
  env.spec.routes = env.spec.routes.filter((r) => {
    if (r.via !== vmName) return true;
    return r.from !== networkName && r.to !== networkName;
  });
  return { env, diagram };
}

/**
 * Assign VM to a network container (primary NIC).
 * Already attached → reorder to front. Single NIC elsewhere → replace.
 * Multi-homed without target → add as new primary.
 */
export function applyAssignPrimaryNetwork(
  snap: EditorSnapshot,
  vmName: string,
  networkName: string,
  position?: { x: number; y: number },
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const vm = env.spec.vms[vmName];
  if (!vm) mutationError("topo.mutation.vmMissing", { name: vmName });
  if (!env.spec.networks[networkName]) {
    mutationError("topo.mutation.netMissing", { name: networkName });
  }
  if (vm.networks[0]?.name === networkName) {
    if (position) {
      const id = vmCellId(vmName);
      const prev = diagram.nodes[id] ?? { x: position.x, y: position.y };
      diagram.nodes[id] = { ...prev, x: position.x, y: position.y };
    }
    return { env, diagram };
  }
  const existing = vm.networks.find((n) => n.name === networkName);
  if (existing) {
    vm.networks = [
      existing,
      ...vm.networks.filter((n) => n.name !== networkName),
    ];
  } else if (vm.networks.length <= 1) {
    const old = vm.networks[0];
    if (old) {
      delete diagram.edges[attachmentEdgeId(vmName, old.name)];
    }
    vm.networks = [
      { name: networkName, ip: nextIp(env, networkName) },
    ];
  } else {
    vm.networks = [
      { name: networkName, ip: nextIp(env, networkName) },
      ...vm.networks,
    ];
  }
  if (position) {
    const id = vmCellId(vmName);
    const prev = diagram.nodes[id] ?? { x: position.x, y: position.y };
    diagram.nodes[id] = { ...prev, x: position.x, y: position.y };
  }
  return { env, diagram };
}

export function applyMoveNode(
  snap: EditorSnapshot,
  nodeId: string,
  x: number,
  y: number,
): EditorSnapshot {
  const diagram = cloneDiagram(snap.diagram);
  const prev = diagram.nodes[nodeId] ?? { x, y };
  diagram.nodes[nodeId] = { ...prev, x, y };
  return { env: snap.env, diagram };
}

/** Batch-update absolute positions (e.g. container + embedded VMs). */
export function applyMoveNodes(
  snap: EditorSnapshot,
  positions: Array<{ nodeId: string; x: number; y: number }>,
): EditorSnapshot {
  const diagram = cloneDiagram(snap.diagram);
  for (const { nodeId, x, y } of positions) {
    const prev = diagram.nodes[nodeId] ?? { x, y };
    diagram.nodes[nodeId] = { ...prev, x, y };
  }
  return { env: snap.env, diagram };
}

export function applyResizeNode(
  snap: EditorSnapshot,
  nodeId: string,
  width: number,
  height: number,
): EditorSnapshot {
  const diagram = cloneDiagram(snap.diagram);
  const prev = diagram.nodes[nodeId] ?? { x: 0, y: 0, width, height };
  diagram.nodes[nodeId] = { ...prev, width, height };
  return { env: snap.env, diagram };
}

function assertValidResourceName(name: string): string {
  const trimmed = name.trim();
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$/.test(trimmed)) {
    mutationError("topo.mutation.nameInvalid");
  }
  return trimmed;
}

function remapDiagramKey(
  diagram: DiagramState,
  from: string,
  to: string,
): void {
  if (from === to) return;
  if (diagram.nodes[from]) {
    diagram.nodes[to] = diagram.nodes[from]!;
    delete diagram.nodes[from];
  }
  if (diagram.edges[from]) {
    diagram.edges[to] = diagram.edges[from]!;
    delete diagram.edges[from];
  }
}

/** Rename VM key and all references (dependsOn, routes.via, ports, diagram). */
export function applyRenameVm(
  snap: EditorSnapshot,
  oldName: string,
  newNameRaw: string,
): EditorSnapshot {
  const newName = assertValidResourceName(newNameRaw);
  if (oldName === newName) return snap;
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const vm = env.spec.vms[oldName];
  if (!vm) mutationError("topo.mutation.vmMissing", { name: oldName });
  if (env.spec.vms[newName]) {
    mutationError("topo.mutation.vmExistsRename", { name: newName });
  }

  env.spec.vms[newName] = vm;
  delete env.spec.vms[oldName];

  for (const other of Object.values(env.spec.vms)) {
    other.dependsOn = other.dependsOn.map((d) => (d === oldName ? newName : d));
  }
  for (const route of env.spec.routes) {
    if (route.via === oldName) route.via = newName;
  }
  env.spec.ports = env.spec.ports.map((p) =>
    p.replace(new RegExp(`:${oldName}:`), `:${newName}:`),
  );

  remapDiagramKey(diagram, vmCellId(oldName), vmCellId(newName));
  for (const key of Object.keys(diagram.edges)) {
    if (key.startsWith(`attach:${oldName}:`)) {
      const net = key.slice(`attach:${oldName}:`.length);
      remapDiagramKey(diagram, key, attachmentEdgeId(newName, net));
    }
  }

  return { env, diagram };
}

/** Rename network key and all NIC/route/policy/diagram references. */
export function applyRenameNetwork(
  snap: EditorSnapshot,
  oldName: string,
  newNameRaw: string,
): EditorSnapshot {
  const newName = assertValidResourceName(newNameRaw);
  if (oldName === newName) return snap;
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const net = env.spec.networks[oldName];
  if (!net) mutationError("topo.mutation.netMissing", { name: oldName });
  if (env.spec.networks[newName]) {
    mutationError("topo.mutation.netExistsRename", { name: newName });
  }

  env.spec.networks[newName] = net;
  delete env.spec.networks[oldName];

  for (const vm of Object.values(env.spec.vms)) {
    for (const nic of vm.networks) {
      if (nic.name === oldName) nic.name = newName;
    }
  }
  for (const route of env.spec.routes) {
    if (route.from === oldName) route.from = newName;
    if (route.to === oldName) route.to = newName;
  }
  for (const policy of env.spec.policies) {
    if (policy.network === oldName) policy.network = newName;
    for (const rule of policy.allow) {
      if (rule.to === oldName) rule.to = newName;
    }
  }

  remapDiagramKey(diagram, networkCellId(oldName), networkCellId(newName));
  remapDiagramKey(diagram, `igw:${oldName}`, `igw:${newName}`);
  remapDiagramKey(diagram, `uplink:${oldName}`, `uplink:${newName}`);
  for (const key of Object.keys(diagram.edges)) {
    if (key.startsWith("attach:") && key.endsWith(`:${oldName}`)) {
      const vmName = key.slice("attach:".length, key.length - `:${oldName}`.length);
      remapDiagramKey(diagram, key, attachmentEdgeId(vmName, newName));
    }
  }

  return { env, diagram };
}

export function applyUpdateVm(
  snap: EditorSnapshot,
  name: string,
  patch: Partial<{
    cpus: number;
    memory: string;
    dataDisk: string;
    roles: Array<"router" | "docker">;
    dependsOn: string[];
  }>,
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const vm = env.spec.vms[name];
  if (!vm) mutationError("topo.mutation.vmMissing", { name });
  Object.assign(vm, patch);
  return { env, diagram: snap.diagram };
}

export function applyUpdateNetwork(
  snap: EditorSnapshot,
  name: string,
  patch: Partial<{
    cidr: string;
    mode: NetworkMode;
    dhcp: boolean;
    natEgress: boolean;
    backend: NetworkBackend;
  }>,
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const net = env.spec.networks[name];
  if (!net) mutationError("topo.mutation.netMissing", { name });
  Object.assign(net, patch);
  if (patch.backend === "docker") {
    net.natEgress = false;
    net.dhcp = false;
  }
  if (net.backend === "docker" && patch.natEgress === true) {
    net.natEgress = false;
  }
  if (net.backend === "docker" && patch.dhcp === true) {
    net.dhcp = false;
  }
  if (patch.natEgress === false || net.backend === "docker" || net.natEgress === false) {
    const policyName = `${name}-default`;
    if (!env.spec.policies.some((p) => p.network === name)) {
      env.spec.policies.push({
        name: policyName,
        network: name,
        forward: "deny-all",
        allow: [],
      });
    }
    delete diagram.nodes[`igw:${name}`];
  }
  return { env, diagram };
}

export function applyUpdateNicIp(
  snap: EditorSnapshot,
  vmName: string,
  networkName: string,
  ip: string,
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const vm = env.spec.vms[vmName];
  if (!vm) mutationError("topo.mutation.vmMissing", { name: vmName });
  const nic = vm.networks.find((n) => n.name === networkName);
  if (!nic) mutationError("topo.mutation.nicMissing", { name: networkName });
  nic.ip = ip;
  return { env, diagram: snap.diagram };
}

export function applyUpsertRoute(
  snap: EditorSnapshot,
  route: { name: string; from: string; to: string; via: string },
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  const idx = env.spec.routes.findIndex((r) => r.name === route.name);
  if (idx >= 0) env.spec.routes[idx] = route;
  else env.spec.routes.push(route);
  diagram.edges[`route:${route.name}`] = diagram.edges[`route:${route.name}`] ?? {
    vertices: [],
  };
  return { env, diagram };
}

export function applyDeleteRoute(snap: EditorSnapshot, name: string): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const diagram = cloneDiagram(snap.diagram);
  env.spec.routes = env.spec.routes.filter((r) => r.name !== name);
  delete diagram.edges[`route:${name}`];
  return { env, diagram };
}

export function applyUpsertPolicy(
  snap: EditorSnapshot,
  policy: {
    name: string;
    network: string;
    allow: AllowRule[];
  },
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const idx = env.spec.policies.findIndex((p) => p.name === policy.name);
  const next = {
    name: policy.name,
    network: policy.network,
    forward: "deny-all" as const,
    allow: policy.allow,
  };
  if (idx >= 0) env.spec.policies[idx] = next;
  else env.spec.policies.push(next);
  return { env, diagram: snap.diagram };
}

export function applyDeletePolicy(snap: EditorSnapshot, name: string): EditorSnapshot {
  const env = cloneEnv(snap.env);
  env.spec.policies = env.spec.policies.filter((p) => p.name !== name);
  return { env, diagram: snap.diagram };
}

export function applySetAllowRules(
  snap: EditorSnapshot,
  policyName: string,
  allow: AllowRule[],
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const policy = env.spec.policies.find((p) => p.name === policyName);
  if (!policy) mutationError("topo.mutation.policyMissing", { name: policyName });
  policy.allow = allow;
  return { env, diagram: snap.diagram };
}

export function applyEnsurePolicyForNetwork(
  snap: EditorSnapshot,
  networkName: string,
): EditorSnapshot {
  const env = cloneEnv(snap.env);
  const existing = env.spec.policies.find((p) => p.network === networkName);
  if (existing) return { env, diagram: snap.diagram };
  env.spec.policies.push({
    name: `${networkName}-default`,
    network: networkName,
    forward: "deny-all",
    allow: [],
  });
  return { env, diagram: snap.diagram };
}

export function revalidate(env: Environment): ValidationIssue[] {
  return validateEnvironment(env);
}

export function suggestAllowRule(
  to: string,
  proto: Protocol,
  ports: number[],
): AllowRule {
  return { to, proto, ports: proto === "icmp" ? [] : ports };
}
