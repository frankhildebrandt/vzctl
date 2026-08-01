import type { Environment } from "@/domain/hypernetwork/schema";
import type { DiagramState } from "@/domain/diagram/types";
import { networkCellId, vmCellId } from "@/domain/hypernetwork/ids";

const NET_PAD_X = 16;
const NET_PAD_TOP = 52;
const NET_PAD_BOTTOM = 16;
const VM_W = 200;
const VM_H = 100;
const VM_GAP = 16;
const NET_GAP = 80;

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

/** Explicit auto-layout — single-homed in containers, multi-homed below. */
export function layoutByNetwork(env: Environment): DiagramState["nodes"] {
  const nodes: DiagramState["nodes"] = {};
  const netNames = Object.keys(env.spec.networks);

  const singleByNet = new Map<string, string[]>();
  const multiHomed: string[] = [];
  for (const [vmName, vm] of Object.entries(env.spec.vms)) {
    if (vm.networks.length > 1) {
      multiHomed.push(vmName);
      continue;
    }
    const primary = vm.networks[0]?.name ?? netNames[0] ?? "lan";
    const list = singleByNet.get(primary) ?? [];
    list.push(vmName);
    singleByNet.set(primary, list);
  }

  let cursorX = 60;
  let maxBottom = 80;
  netNames.forEach((netName) => {
    const vms = singleByNet.get(netName) ?? [];
    const size = containerSizeForCount(vms.length);
    nodes[networkCellId(netName)] = {
      x: cursorX,
      y: 80,
      width: size.width,
      height: size.height,
    };
    if (
      env.spec.networks[netName]?.natEgress !== false &&
      env.spec.networks[netName]?.backend !== "docker"
    ) {
      nodes[`igw:${netName}`] = {
        x: cursorX + size.width / 2 - 50,
        y: -12,
        width: 100,
        height: 68,
      };
    }
    maxBottom = Math.max(maxBottom, 80 + size.height);

    const cols = Math.max(1, Math.min(3, vms.length || 1));
    vms.forEach((vmName, i) => {
      const col = i % cols;
      const row = Math.floor(i / cols);
      nodes[vmCellId(vmName)] = {
        x: cursorX + NET_PAD_X + col * (VM_W + VM_GAP),
        y: 80 + NET_PAD_TOP + row * (VM_H + VM_GAP),
        width: VM_W,
        height: VM_H,
      };
    });

    cursorX += size.width + NET_GAP;
  });

  const cols = 3;
  multiHomed.forEach((vmName, i) => {
    const col = i % cols;
    const row = Math.floor(i / cols);
    nodes[vmCellId(vmName)] = {
      x: 60 + col * (VM_W + VM_GAP + 24),
      y: maxBottom + 48 + row * (VM_H + VM_GAP),
      width: VM_W,
      height: VM_H,
    };
  });

  return nodes;
}
