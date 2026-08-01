import type { Graph, Node } from "@antv/x6";
import type { PaletteKind } from "@/features/topology-editor/PaletteIcons";
import {
  SHAPE_NETWORK,
  SHAPE_VM,
} from "@/diagram/nodes/shapes";
import { getT } from "@/lib/i18n";

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
  const t = getT();
  if (kind === "network") {
    return graph.createNode({
      shape: SHAPE_NETWORK,
      width: 320,
      height: 200,
      attrs: {
        body: { opacity: 0.85 },
        title: { text: t("topo.palette.network") },
        subtitle: { text: t("topo.previewNewNet") },
        meta: { text: t("topo.mode.shared") },
      },
      data: { paletteKind: kind },
    });
  }

  const title =
    kind === "router"
      ? t("topo.palette.router")
      : kind === "docker"
        ? t("topo.palette.docker")
        : t("topo.palette.vm");
  return graph.createNode({
    shape: SHAPE_VM,
    width: 200,
    height: 100,
    attrs: {
      body: {
        opacity: 0.85,
        stroke: kind === "router" ? "#9b2c2c" : kind === "docker" ? "#2a5a8a" : "#1c2b27",
        fill: kind === "router" ? "#faf0f0" : kind === "docker" ? "#eef4fa" : "#fffaf0",
      },
      title: { text: title },
      subtitle: { text: t("topo.previewResources") },
      nics: { text: `eth0 ${t("common.ellipsis")}` },
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
