/** Stable cell / projection IDs derived from hypernetwork names. */

export function networkCellId(name: string): string {
  return `net:${name}`;
}

export function vmCellId(name: string): string {
  return `vm:${name}`;
}

export function gatewayCellId(networkName: string): string {
  return `igw:${networkName}`;
}

export function attachmentEdgeId(vmName: string, networkName: string): string {
  return `attach:${vmName}:${networkName}`;
}

export function routeEdgeId(routeName: string): string {
  return `route:${routeName}`;
}

export function vmNicPortId(vmName: string, networkName: string): string {
  return `nic:${vmName}:${networkName}`;
}

/** Free port to drag a new NIC attachment from a VM. */
export function vmNewNicPortId(vmName: string): string {
  return `nic:${vmName}:new`;
}

export function networkAttachPortId(networkName: string): string {
  return `attach:${networkName}`;
}

export function isVmNicPortId(portId: string): boolean {
  return portId.startsWith("nic:");
}

export function isVmNewNicPortId(portId: string): boolean {
  return /^nic:[^:]+:new$/.test(portId);
}

export function isNetworkAttachPortId(portId: string): boolean {
  const parts = portId.split(":");
  return parts.length === 2 && parts[0] === "attach" && Boolean(parts[1]);
}

export function networkUplinkPortId(networkName: string): string {
  return `uplink:${networkName}`;
}

export function networkRouteLeftPortId(networkName: string): string {
  return `route-in:${networkName}`;
}

export function networkRouteRightPortId(networkName: string): string {
  return `route-out:${networkName}`;
}

export function gatewayDownPortId(): string {
  return "down";
}

export function parseCellId(
  id: string,
):
  | { kind: "network"; name: string }
  | { kind: "vm"; name: string }
  | { kind: "gateway"; networkName: string }
  | { kind: "unknown"; raw: string } {
  if (id.startsWith("net:")) return { kind: "network", name: id.slice(4) };
  if (id.startsWith("vm:")) return { kind: "vm", name: id.slice(3) };
  if (id.startsWith("igw:"))
    return { kind: "gateway", networkName: id.slice(4) };
  return { kind: "unknown", raw: id };
}
