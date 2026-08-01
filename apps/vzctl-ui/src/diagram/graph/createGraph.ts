import {
  Graph,
  Selection,
  Snapline,
  Keyboard,
  Clipboard,
  Scroller,
  MiniMap,
  Transform,
  Export,
  Dnd,
} from "@antv/x6";
import { registerShapes, SHAPE_EDGE } from "@/diagram/nodes/shapes";
import type { PaletteKind } from "@/features/topology-editor/PaletteIcons";
import { paletteKindFromNode } from "@/diagram/interactions/paletteDnd";
import {
  isNetworkAttachPortId,
  isVmNicPortId,
} from "@/domain/hypernetwork/ids";

export type GraphBundle = {
  graph: Graph;
  dnd: Dnd;
  dispose: () => void;
};

export type CreateGraphOptions = {
  container: HTMLElement;
  minimapContainer?: HTMLElement | null;
  validating?: (
    sourceCellId: string,
    targetCellId: string,
    sourcePortId: string,
    targetPortId: string,
    edgeId?: string,
  ) => boolean | string;
  /** Called when a palette preview is dropped — Domain should materialize the item. */
  onPaletteMaterialize?: (
    kind: PaletteKind,
    localX: number,
    localY: number,
  ) => void;
};

function magnetPortId(magnet: Element | null | undefined): string {
  if (!magnet) return "";
  return (
    magnet.getAttribute("port") ||
    magnet.getAttribute("data-port") ||
    ""
  );
}

export function createTopologyGraph(options: CreateGraphOptions): GraphBundle {
  registerShapes();

  const graph: Graph = new Graph({
    container: options.container,
    autoResize: true,
    grid: {
      visible: true,
      type: "dot",
      args: { color: "#ddd2c0", thickness: 1 },
    },
    panning: false,
    mousewheel: {
      enabled: true,
      modifiers: ["ctrl", "meta"],
      minScale: 0.3,
      maxScale: 2.5,
    },
    interacting: {
      arrowheadMovable(cellView) {
        const cell = cellView.cell;
        return cell.isEdge() && String(cell.id).startsWith("attach:");
      },
      vertexMovable(cellView) {
        const cell = cellView.cell;
        return cell.isEdge() && String(cell.id).startsWith("attach:");
      },
    },
    embedding: {
      enabled: true,
      findParent({ node }) {
        if (!node.id.startsWith("vm:")) return [];
        const data = node.getData() as { multiHomed?: boolean } | null;
        // Multi-homed VMs stay outside containers
        if (data?.multiHomed) return [];
        const bbox = node.getBBox();
        const center = bbox.getCenter();
        const candidates = this.getNodes().filter((candidate) => {
          if (candidate.id === node.id) return false;
          const kind = (candidate.getData() as { kind?: string } | null)?.kind;
          if (kind !== "network") return false;
          return candidate.getBBox().containsPoint(center);
        });
        // Smallest containing network wins (most specific).
        return candidates.sort((a, b) => {
          const aa = a.getBBox();
          const bb = b.getBBox();
          return aa.width * aa.height - bb.width * bb.height;
        });
      },
    },
    // No translating.restrict — single-homed VMs must leave a container
    // to be re-embedded into another network.
    connecting: {
      router: "manhattan",
      connector: { name: "rounded", args: { radius: 6 } },
      anchor: "center",
      connectionPoint: "anchor",
      allowBlank: false,
      allowLoop: false,
      allowNode: false,
      allowEdge: false,
      allowPort: true,
      allowMulti: "withPort",
      highlight: true,
      snap: { radius: 28 },
      createEdge(): ReturnType<Graph["createEdge"]> {
        return graph.createEdge({
          shape: SHAPE_EDGE,
          attrs: {
            line: { stroke: "#0f6a5a", strokeDasharray: "6 4" },
          },
        });
      },
      validateMagnet({ magnet, cell }) {
        if (!cell?.isNode() || !magnet) return false;
        const portId = magnetPortId(magnet);
        if (cell.id.startsWith("vm:")) return isVmNicPortId(portId);
        if (cell.id.startsWith("net:")) return isNetworkAttachPortId(portId);
        return false;
      },
      validateConnection({
        sourceCell,
        targetCell,
        sourcePort,
        targetPort,
        sourceMagnet,
        targetMagnet,
        edge,
      }) {
        if (!sourceCell || !targetCell) return false;
        if (!sourceMagnet || !targetMagnet) return false;
        if (sourceCell.id === targetCell.id) return false;
        const sp = sourcePort || magnetPortId(sourceMagnet);
        const tp = targetPort || magnetPortId(targetMagnet);
        if (options.validating) {
          return (
            options.validating(
              sourceCell.id,
              targetCell.id,
              sp,
              tp,
              edge?.id,
            ) === true
          );
        }
        return true;
      },
    },
    highlighting: {
      magnetAvailable: {
        name: "stroke",
        args: { attrs: { fill: "#fff", stroke: "#0f6a5a" } },
      },
      magnetAdsorbed: {
        name: "stroke",
        args: { attrs: { fill: "#0f6a5a", stroke: "#0f6a5a" } },
      },
    },
  });

  const selection = new Selection({
    enabled: true,
    multiple: true,
    rubberband: true,
    showNodeSelectionBox: true,
    modifiers: ["shift"],
  });
  const snapline = new Snapline({ enabled: true, sharp: true });
  const keyboard = new Keyboard({ enabled: true, global: true });
  const clipboard = new Clipboard({ enabled: true });
  const scroller = new Scroller({
    enabled: true,
    pageVisible: false,
    pageBreak: false,
    pannable: true,
    autoResize: true,
  });
  const transform = new Transform({
    resizing: {
      enabled(node) {
        return node.id.startsWith("net:");
      },
      minWidth: 280,
      minHeight: 180,
      orthogonal: false,
    },
    rotating: false,
  });
  const exporter = new Export();

  graph.use(selection);
  graph.use(snapline);
  graph.use(keyboard);
  graph.use(clipboard);
  graph.use(scroller);
  graph.use(transform);
  graph.use(exporter);

  let minimap: MiniMap | null = null;
  if (options.minimapContainer) {
    minimap = new MiniMap({
      container: options.minimapContainer,
      width: 160,
      height: 110,
      padding: 8,
    });
    graph.use(minimap);
  }

  const dnd = new Dnd({
    target: graph,
    scaled: true,
    validateNode(droppingNode) {
      const kind = paletteKindFromNode(droppingNode);
      if (!kind || !options.onPaletteMaterialize) return false;
      const pos = droppingNode.position();
      options.onPaletteMaterialize(kind, pos.x, pos.y);
      return false;
    },
  });

  const dispose = () => {
    dnd.dispose();
    minimap?.dispose();
    selection.dispose();
    snapline.dispose();
    keyboard.dispose();
    clipboard.dispose();
    scroller.dispose();
    transform.dispose();
    exporter.dispose();
    graph.dispose();
  };

  return { graph, dnd, dispose };
}
