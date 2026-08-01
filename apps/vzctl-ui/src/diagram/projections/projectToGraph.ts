import type { Graph, Node } from "@antv/x6";
import type { Environment } from "@/domain/hypernetwork/schema";
import type { DiagramState } from "@/domain/diagram/types";
import {
  attachmentEdgeId,
  gatewayDownPortId,
  networkAttachPortId,
  networkCellId,
  networkRouteLeftPortId,
  networkRouteRightPortId,
  networkUplinkPortId,
  routeEdgeId,
  vmCellId,
  vmNewNicPortId,
  vmNicPortId,
} from "@/domain/hypernetwork/ids";
import {
  SHAPE_EDGE,
  SHAPE_GATEWAY,
  SHAPE_NETWORK,
  SHAPE_VM,
} from "@/diagram/nodes/shapes";
import { getT } from "@/lib/i18n";

export type ProjectedModel = {
  nodeIds: Set<string>;
  edgeIds: Set<string>;
};

const NET_PAD_X = 16;
const NET_PAD_TOP = 52;
const NET_PAD_BOTTOM = 16;
const VM_W = 200;
const VM_H = 100;
const VM_GAP = 16;

function memoryLabel(memory: string | number | undefined): string {
  if (memory === undefined) return "1 GB";
  if (typeof memory === "number") return `${Math.round(memory / 1024)} GB`;
  const m = String(memory);
  if (/MiB?$/i.test(m)) {
    const n = Number.parseInt(m, 10);
    return `${Math.round(n / 1024)} GB`;
  }
  if (/GiB?$/i.test(m)) return `${Number.parseInt(m, 10)} GB`;
  return m;
}

function defaultPos(
  diagram: DiagramState,
  id: string,
  fallback: { x: number; y: number; width: number; height: number },
) {
  const p = diagram.nodes[id];
  return {
    x: p?.x ?? fallback.x,
    y: p?.y ?? fallback.y,
    width: p?.width ?? fallback.width,
    height: p?.height ?? fallback.height,
  };
}

function ensurePort(
  node: Node,
  id: string,
  group: string,
  label?: string,
): void {
  if (node.hasPort(id)) {
    node.setPortProp(id, "group", group);
    if (label !== undefined) {
      node.setPortProp(id, "attrs/text/text", label);
    }
    return;
  }
  node.addPort({
    id,
    group,
    attrs: label ? { text: { text: label, fontSize: 9, fill: "#5b564c" } } : {},
  });
}

/** Single-homed VMs live in the container; multi-homed sit outside with edges. */
function isMultiHomed(networks: { name: string }[]): boolean {
  return networks.length > 1;
}

/** Ports: multi-homed = all NICs + "+"; single-homed = only "+" (primary = container). */
function syncVmPorts(
  node: Node,
  vmName: string,
  networks: string[],
): void {
  const multi = isMultiHomed(networks.map((name) => ({ name })));
  const linked = multi ? networks : [];
  const newPort = vmNewNicPortId(vmName);
  const desired = new Set([
    newPort,
    ...linked.map((n) => vmNicPortId(vmName, n)),
  ]);
  for (const port of node.getPorts()) {
    if (port.id && !desired.has(port.id)) {
      node.removePort(port.id);
    }
  }
  ensurePort(node, newPort, "nic", "+");
  for (const netName of linked) {
    ensurePort(node, vmNicPortId(vmName, netName), "nic", netName);
  }
}

function syncNetworkPorts(node: Node, networkName: string): void {
  ensurePort(node, networkUplinkPortId(networkName), "uplink");
  ensurePort(node, networkAttachPortId(networkName), "attach");
  ensurePort(node, networkRouteLeftPortId(networkName), "routeLeft");
  ensurePort(node, networkRouteRightPortId(networkName), "routeRight");
}

/** Single-homed members of a network (embedded in the container). */
function vmsInNetwork(env: Environment, networkName: string): string[] {
  return Object.entries(env.spec.vms)
    .filter(
      ([, vm]) =>
        !isMultiHomed(vm.networks) && vm.networks[0]?.name === networkName,
    )
    .map(([name]) => name);
}

function multiHomedVms(env: Environment): string[] {
  return Object.entries(env.spec.vms)
    .filter(([, vm]) => isMultiHomed(vm.networks))
    .map(([name]) => name);
}

function containerSizeForCount(count: number): { width: number; height: number } {
  const cols = Math.max(1, Math.min(3, count || 1));
  const rows = Math.max(1, Math.ceil((count || 1) / cols));
  return {
    width: Math.max(320, NET_PAD_X * 2 + cols * VM_W + (cols - 1) * VM_GAP),
    height: Math.max(
      200,
      NET_PAD_TOP + NET_PAD_BOTTOM + rows * VM_H + (rows - 1) * VM_GAP,
    ),
  };
}

function defaultVmSlot(
  netX: number,
  netY: number,
  index: number,
  count: number,
): { x: number; y: number } {
  const cols = Math.max(1, Math.min(3, count || 1));
  const col = index % cols;
  const row = Math.floor(index / cols);
  return {
    x: netX + NET_PAD_X + col * (VM_W + VM_GAP),
    y: netY + NET_PAD_TOP + row * (VM_H + VM_GAP),
  };
}

function outsideVmFallback(
  env: Environment,
  diagram: DiagramState,
  index: number,
): { x: number; y: number; width: number; height: number } {
  let maxBottom = 280;
  let minX = 60;
  for (const name of Object.keys(env.spec.networks)) {
    const p = diagram.nodes[networkCellId(name)];
    if (!p) continue;
    maxBottom = Math.max(maxBottom, (p.y ?? 80) + (p.height ?? 200));
    minX = Math.min(minX, p.x ?? 60);
  }
  const cols = 3;
  const col = index % cols;
  const row = Math.floor(index / cols);
  return {
    x: minX + col * (VM_W + VM_GAP + 24),
    y: maxBottom + 48 + row * (VM_H + VM_GAP),
    width: VM_W,
    height: VM_H,
  };
}

function positionInsideAnyNetwork(
  env: Environment,
  diagram: DiagramState,
  x: number,
  y: number,
): boolean {
  const pt = { x: x + VM_W / 2, y: y + VM_H / 2 };
  for (const name of Object.keys(env.spec.networks)) {
    const p = diagram.nodes[networkCellId(name)];
    if (!p) continue;
    const left = p.x ?? 0;
    const top = p.y ?? 0;
    const right = left + (p.width ?? 320);
    const bottom = top + (p.height ?? 200);
    if (pt.x >= left && pt.x <= right && pt.y >= top && pt.y <= bottom) {
      return true;
    }
  }
  return false;
}

const ATTACH_EDGE_TOOLS = [
  {
    name: "source-arrowhead",
    args: {
      attrs: {
        d: "M -4 -4 4 0 -4 4 Z",
        fill: "#0f6a5a",
        stroke: "#0f6a5a",
      },
    },
  },
  {
    name: "target-arrowhead",
    args: {
      attrs: {
        d: "M -4 -4 4 0 -4 4 Z",
        fill: "#0f6a5a",
        stroke: "#0f6a5a",
      },
    },
  },
];

/**
 * Incremental projection: Domain + DiagramState → X6 cells.
 * Single-homed VMs embed in their primary network; multi-homed sit outside.
 */
export function projectToGraph(
  graph: Graph,
  env: Environment,
  diagram: DiagramState,
  options?: { showGateways?: boolean },
): ProjectedModel {
  const nodeIds = new Set<string>();
  const edgeIds = new Set<string>();
  const showGateways = options?.showGateways ?? true;

  let netIndex = 0;
  for (const [name, net] of Object.entries(env.spec.networks)) {
    const id = networkCellId(name);
    nodeIds.add(id);
    const members = vmsInNetwork(env, name);
    const auto = containerSizeForCount(members.length);
    const pos = defaultPos(diagram, id, {
      x: 60 + netIndex * (auto.width + 80),
      y: 80,
      width: auto.width,
      height: auto.height,
    });
    // Grow to at least fit members if stored size is too small
    const width = Math.max(pos.width, auto.width);
    const height = Math.max(pos.height, auto.height);

    const attrs = {
      title: { text: name },
      subtitle: { text: net.cidr },
      meta: {
        text: `${members.length} VMs · ${net.mode}${net.backend === "docker" ? " · docker" : ""}${net.natEgress === false ? " · isolated" : ""}`,
      },
    };
    const existing = graph.getCellById(id) as Node | null;
    if (existing?.isNode()) {
      existing.position(pos.x, pos.y);
      existing.resize(width, height);
      existing.attr(attrs);
      existing.setData({ kind: "network", name, parent: true });
      existing.setZIndex(1);
      syncNetworkPorts(existing, name);
    } else {
      const node = graph.addNode({
        id,
        shape: SHAPE_NETWORK,
        x: pos.x,
        y: pos.y,
        width,
        height,
        zIndex: 1,
        attrs,
        data: { kind: "network", name, parent: true },
      });
      syncNetworkPorts(node, name);
    }

    if (showGateways && net.natEgress !== false && net.backend !== "docker") {
      const gid = `igw:${name}`;
      nodeIds.add(gid);
      const internetLabel = getT()("topo.gatewayLabel");
      const gpos = defaultPos(diagram, gid, {
        x: pos.x + width / 2 - 50,
        y: pos.y - 92,
        width: 100,
        height: 68,
      });
      let gNode = graph.getCellById(gid) as Node | null;
      if (!gNode?.isNode()) {
        gNode = graph.addNode({
          id: gid,
          shape: SHAPE_GATEWAY,
          x: gpos.x,
          y: gpos.y,
          width: gpos.width,
          height: gpos.height,
          zIndex: 5,
          attrs: { label: { text: internetLabel } },
          data: { kind: "gateway", networkName: name },
        });
      } else {
        gNode.position(gpos.x, gpos.y);
        gNode.resize(gpos.width, gpos.height);
        gNode.attr("label/text", internetLabel);
      }
      const geid = `uplink:${name}`;
      edgeIds.add(geid);
      const uplinkExisting = graph.getCellById(geid);
      if (uplinkExisting?.isEdge()) {
        uplinkExisting.setSource({ cell: gid, port: gatewayDownPortId() });
        uplinkExisting.setTarget({
          cell: id,
          port: networkUplinkPortId(name),
        });
      } else {
        graph.addEdge({
          id: geid,
          shape: SHAPE_EDGE,
          source: { cell: gid, port: gatewayDownPortId() },
          target: { cell: id, port: networkUplinkPortId(name) },
          attrs: {
            line: {
              stroke: "#8aa69c",
              strokeDasharray: "4 4",
              targetMarker: null,
              sourceMarker: null,
            },
          },
          router: {
            name: "manhattan",
            args: {
              padding: 12,
              startDirections: ["bottom"],
              endDirections: ["top"],
            },
          },
          data: { kind: "uplink", networkName: name },
          tools: [],
        });
      }
    }
    netIndex += 1;
  }

  // VMs — single-homed embed; multi-homed outside with edges to all nets
  const memberIndex = new Map<string, number>();
  const multiIndex = new Map<string, number>();
  multiHomedVms(env).forEach((n, i) => multiIndex.set(n, i));

  for (const [name, vm] of Object.entries(env.spec.vms)) {
    const id = vmCellId(name);
    nodeIds.add(id);
    const primary = vm.networks[0]?.name;
    const multi = isMultiHomed(vm.networks);
    const parentId =
      !multi && primary ? networkCellId(primary) : null;
    const parentNode = parentId
      ? (graph.getCellById(parentId) as Node | null)
      : null;

    let fallback = { x: 80, y: 400, width: VM_W, height: VM_H };
    if (multi) {
      fallback = outsideVmFallback(env, diagram, multiIndex.get(name) ?? 0);
    } else if (parentNode?.isNode() && primary) {
      const idx = memberIndex.get(primary) ?? 0;
      memberIndex.set(primary, idx + 1);
      const memberCount = vmsInNetwork(env, primary).length;
      const pb = parentNode.getBBox();
      const slot = defaultVmSlot(pb.x, pb.y, idx, memberCount);
      fallback = { x: slot.x, y: slot.y, width: VM_W, height: VM_H };
    }
    let pos = defaultPos(diagram, id, fallback);
    if (
      multi &&
      diagram.nodes[id] &&
      positionInsideAnyNetwork(env, diagram, pos.x, pos.y)
    ) {
      pos = fallback;
    }

    const isRouter = vm.roles.includes("router");
    const isDocker = vm.roles.includes("docker");
    const title = isRouter
      ? `Router: ${name}`
      : isDocker
        ? `Docker: ${name}`
        : `Host: ${name}`;
    const subtitle = `${vm.cpus ?? 2} CPU · ${memoryLabel(vm.memory)}`;
    const nics = vm.networks
      .map((n, i) => `${i === 0 ? "●" : "○"} ${n.name} ${n.ip}`)
      .join("\n");
    const attrs = {
      body: {
        stroke: isRouter ? "#9b2c2c" : isDocker ? "#2a5a8a" : "#1c2b27",
        fill: isRouter ? "#faf0f0" : isDocker ? "#eef4fa" : "#fffaf0",
      },
      title: { text: title },
      subtitle: { text: subtitle },
      nics: { text: nics },
    };

    const netNames = vm.networks.map((n) => n.name);
    let node = graph.getCellById(id) as Node | null;
    if (node?.isNode()) {
      node.position(pos.x, pos.y);
      node.resize(pos.width, pos.height);
      node.attr(attrs);
      node.setZIndex(10);
      node.setData({
        kind: "vm",
        name,
        roles: vm.roles,
        multiHomed: multi,
      });
      syncVmPorts(node, name, netNames);
    } else {
      node = graph.addNode({
        id,
        shape: SHAPE_VM,
        x: pos.x,
        y: pos.y,
        width: pos.width,
        height: pos.height,
        zIndex: 10,
        attrs,
        data: {
          kind: "vm",
          name,
          roles: vm.roles,
          multiHomed: multi,
        },
      });
      syncVmPorts(node, name, netNames);
    }

    if (parentNode?.isNode()) {
      if (node.getParent() !== parentNode) {
        parentNode.addChild(node);
      }
    } else if (node.getParent()) {
      node.setParent(null);
    }

    // Edges: all NICs when multi-homed; none when single-homed (container)
    if (multi) {
      for (const nic of vm.networks) {
        const eid = attachmentEdgeId(name, nic.name);
        edgeIds.add(eid);
        const vertices = diagram.edges[eid]?.vertices ?? [];
        const source = { cell: id, port: vmNicPortId(name, nic.name) };
        const target = {
          cell: networkCellId(nic.name),
          port: networkAttachPortId(nic.name),
        };
        const isPrimary = nic.name === primary;
        const edgeExisting = graph.getCellById(eid);
        if (edgeExisting?.isEdge()) {
          edgeExisting.setSource(source);
          edgeExisting.setTarget(target);
          edgeExisting.setVertices(vertices);
          edgeExisting.setLabels([
            { attrs: { label: { text: nic.ip, fontSize: 10 } } },
          ]);
          edgeExisting.attr(
            "line/strokeDasharray",
            isPrimary ? "" : "5 3",
          );
          edgeExisting.attr("line/strokeWidth", isPrimary ? 2 : 1.5);
          edgeExisting.setData({
            kind: "attachment",
            vmName: name,
            networkName: nic.name,
          });
          edgeExisting.removeTools();
          edgeExisting.addTools(ATTACH_EDGE_TOOLS);
        } else if (graph.getCellById(networkCellId(nic.name))) {
          graph.addEdge({
            id: eid,
            shape: SHAPE_EDGE,
            source,
            target,
            vertices,
            attrs: {
              line: {
                stroke: "#0f6a5a",
                strokeWidth: isPrimary ? 2 : 1.5,
                strokeDasharray: isPrimary ? undefined : "5 3",
              },
            },
            router: {
              name: "manhattan",
              args: {
                padding: 16,
                startDirections: ["right", "left", "bottom"],
                endDirections: ["bottom", "top", "left", "right"],
              },
            },
            labels: [{ attrs: { label: { text: nic.ip, fontSize: 10 } } }],
            data: {
              kind: "attachment",
              vmName: name,
              networkName: nic.name,
            },
            tools: ATTACH_EDGE_TOOLS,
          });
        }
      }
    }
  }

  for (const route of env.spec.routes) {
    const eid = routeEdgeId(route.name);
    edgeIds.add(eid);
    const vertices = diagram.edges[eid]?.vertices ?? [];
    const fromId = networkCellId(route.from);
    const toId = networkCellId(route.to);
    if (!graph.getCellById(fromId) || !graph.getCellById(toId)) continue;
    const source = {
      cell: fromId,
      port: networkRouteRightPortId(route.from),
    };
    const target = {
      cell: toId,
      port: networkRouteLeftPortId(route.to),
    };
    const existing = graph.getCellById(eid);
    if (existing?.isEdge()) {
      existing.setSource(source);
      existing.setTarget(target);
      existing.setVertices(vertices);
      existing.setLabels([
        { attrs: { label: { text: `via ${route.via}`, fontSize: 10 } } },
      ]);
      existing.attr("line/stroke", "#9b2c2c");
      existing.attr("line/strokeWidth", 2);
    } else {
      graph.addEdge({
        id: eid,
        shape: SHAPE_EDGE,
        source,
        target,
        vertices,
        attrs: {
          line: { stroke: "#9b2c2c", strokeWidth: 2 },
        },
        router: {
          name: "manhattan",
          args: {
            padding: 20,
            startDirections: ["right"],
            endDirections: ["left"],
          },
        },
        labels: [
          { attrs: { label: { text: `via ${route.via}`, fontSize: 10 } } },
        ],
        data: { kind: "route", ...route },
      });
    }
  }

  for (const cell of graph.getCells()) {
    const id = cell.id;
    if (cell.isNode() && !nodeIds.has(id)) {
      graph.removeCell(id);
    } else if (cell.isEdge() && !edgeIds.has(id)) {
      graph.removeCell(id);
    }
  }

  return { nodeIds, edgeIds };
}

/** Find network container under a local point (for palette drop). */
export function findNetworkAtPoint(
  graph: Graph,
  localX: number,
  localY: number,
): string | null {
  for (const node of graph.getNodes()) {
    const data = node.getData() as { kind?: string; name?: string } | null;
    if (data?.kind !== "network" || !data.name) continue;
    if (node.getBBox().containsPoint({ x: localX, y: localY })) {
      return data.name;
    }
  }
  return null;
}
