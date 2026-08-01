import type { Graph, Node } from "@antv/x6";
import type { PaletteKind } from "@/features/topology-editor/PaletteIcons";
import {
  SHAPE_NETWORK,
  SHAPE_VM,
} from "@/diagram/nodes/shapes";

function rolesForKind(kind: PaletteKind): Array<"router" | "docker"> {
  if (kind === "router") return ["router"];
  // Docker-Host kann Owner eines backend:docker-Netzes werden → dual role.
  if (kind === "docker") return ["docker", "router"];
  return [];
}

export function paletteRoles(kind: PaletteKind): Array<"router" | "docker"> {
  return rolesForKind(kind);
}

export function paletteNameBase(kind: PaletteKind): string {
  if (kind === "network") return "net";
  if (kind === "router") return "router";
  if (kind === "docker") return "docker";
  return "host";
}

/** Semi-transparent preview node for X6 Dnd materialization. */
export function createPalettePreviewNode(
  graph: Graph,
  kind: PaletteKind,
): Node {
  if (kind === "network") {
    return graph.createNode({
      shape: SHAPE_NETWORK,
      width: 320,
      height: 200,
      attrs: {
        body: { opacity: 0.85 },
        title: { text: "Netzwerk" },
        subtitle: { text: "neues Netz" },
        meta: { text: "shared" },
      },
      data: { paletteKind: kind },
    });
  }

  const isRouter = kind === "router";
  const isDocker = kind === "docker";
  const title = isRouter ? "Router" : isDocker ? "Docker" : "Host";
  return graph.createNode({
    shape: SHAPE_VM,
    width: 200,
    height: 100,
    attrs: {
      body: {
        opacity: 0.85,
        stroke: isRouter ? "#9b2c2c" : isDocker ? "#2a5a8a" : "#1c2b27",
        fill: isRouter ? "#faf0f0" : isDocker ? "#eef4fa" : "#fffaf0",
      },
      title: { text: title },
      subtitle: { text: "2 CPU · 2 GB" },
      nics: { text: "eth0 · …" },
    },
    data: { paletteKind: kind },
  });
}

export function paletteKindFromNode(node: Node): PaletteKind | null {
  const data = node.getData() as { paletteKind?: unknown } | null;
  const kind = data?.paletteKind;
  if (
    kind === "network" ||
    kind === "vm" ||
    kind === "router" ||
    kind === "docker"
  ) {
    return kind;
  }
  return null;
}
