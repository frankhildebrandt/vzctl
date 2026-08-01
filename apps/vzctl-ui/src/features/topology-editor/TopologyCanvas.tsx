import { useEffect, useMemo, useRef } from "react";
import type { Dnd, Graph } from "@antv/x6";
import { createTopologyGraph } from "@/diagram/graph/createGraph";
import { projectToGraph } from "@/diagram/projections/projectToGraph";
import { useEditorStore } from "@/store/editorStore";
import { validateConnection } from "@/application/validation/connection";
import type { PaletteKind } from "@/features/topology-editor/PaletteIcons";
import type { ContextMenuState } from "@/features/topology-editor/TopologyContextMenu";
import { useT } from "@/lib/i18n";
import { useSettingsStore } from "@/store/settingsStore";

function editableCellId(cellId: string | null): string | null {
  if (!cellId) return null;
  if (cellId.startsWith("vm:") || cellId.startsWith("net:")) return cellId;
  if (cellId.startsWith("igw:")) return `net:${cellId.slice(4)}`;
  return null;
}

function deletableFromCell(
  cellId: string | null,
  selection: { nodeIds: string[]; edgeIds: string[] },
): boolean {
  if (cellId) {
    return (
      cellId.startsWith("vm:") ||
      cellId.startsWith("net:") ||
      cellId.startsWith("attach:") ||
      cellId.startsWith("route:")
    );
  }
  return (
    selection.nodeIds.some(
      (id) => id.startsWith("vm:") || id.startsWith("net:"),
    ) ||
    selection.edgeIds.some(
      (id) => id.startsWith("attach:") || id.startsWith("route:"),
    )
  );
}

type Props = {
  onReady?: (graph: Graph, dnd: Dnd) => void;
  onPaletteMaterialize?: (
    kind: PaletteKind,
    localX: number,
    localY: number,
  ) => void;
  onContextMenu?: (menu: ContextMenuState) => void;
};

export function TopologyCanvas({
  onReady,
  onPaletteMaterialize,
  onContextMenu,
}: Props) {
  const t = useT();
  const locale = useSettingsStore((s) => s.locale);
  const containerRef = useRef<HTMLDivElement>(null);
  const minimapRef = useRef<HTMLDivElement>(null);
  const graphRef = useRef<Graph | null>(null);
  const materializeRef = useRef(onPaletteMaterialize);
  materializeRef.current = onPaletteMaterialize;
  const contextMenuRef = useRef(onContextMenu);
  contextMenuRef.current = onContextMenu;

  const env = useEditorStore((s) => s.env);
  const diagram = useEditorStore((s) => s.diagram);
  const projecting = useEditorStore((s) => s.ui.projecting);
  const validation = useEditorStore((s) => s.validation);
  const setProjecting = useEditorStore((s) => s.setProjecting);
  const setSelection = useEditorStore((s) => s.setSelection);
  const setConnectionHint = useEditorStore((s) => s.setConnectionHint);
  const moveNode = useEditorStore((s) => s.moveNode);
  const moveNodes = useEditorStore((s) => s.moveNodes);
  const resizeNode = useEditorStore((s) => s.resizeNode);
  const attachNic = useEditorStore((s) => s.attachNic);
  const reassignNic = useEditorStore((s) => s.reassignNic);
  const assignPrimaryNetwork = useEditorStore((s) => s.assignPrimaryNetwork);
  const deleteSelection = useEditorStore((s) => s.deleteSelection);
  const undo = useEditorStore((s) => s.undo);
  const redo = useEditorStore((s) => s.redo);

  const errorNodeIds = useMemo(() => {
    const set = new Set<string>();
    for (const issue of validation) {
      if (issue.severity === "error" && issue.nodeId) set.add(issue.nodeId);
    }
    return set;
  }, [validation]);

  useEffect(() => {
    if (!containerRef.current) return;
    const store = useEditorStore.getState;
    const { graph, dnd, dispose } = createTopologyGraph({
      container: containerRef.current,
      minimapContainer: minimapRef.current,
      onPaletteMaterialize: (kind, x, y) => {
        materializeRef.current?.(kind, x, y);
      },
      validating: (sourceId, targetId, sourcePort, targetPort, edgeId) => {
        const state = store();
        if (!state.env) return false;
        const existing = new Set<string>();
        for (const [vmName, vm] of Object.entries(state.env.spec.vms)) {
          for (const nic of vm.networks) {
            existing.add(`${vmName}|${nic.name}`);
          }
        }
        let ignoreAttachment: string | undefined;
        if (edgeId?.startsWith("attach:")) {
          const parts = edgeId.split(":");
          if (parts[1] && parts[2]) {
            ignoreAttachment = `${parts[1]}|${parts[2]}`;
          }
        }
        const result = validateConnection({
          source: { nodeId: sourceId, portId: sourcePort },
          target: { nodeId: targetId, portId: targetPort },
          existingAttachments: existing,
          existingRoutes: new Set(),
          env: state.env,
          ignoreAttachment,
        });
        if (!result.ok) {
          state.setConnectionHint({
            key: result.reasonKey,
            params: result.reasonParams,
          });
          return false;
        }
        state.setConnectionHint(null);
        return true;
      },
    });
    graphRef.current = graph;
    onReady?.(graph, dnd);

    const openContextMenu = (
      e: { preventDefault(): void; clientX: number; clientY: number },
      localX: number,
      localY: number,
      cellId: string | null,
    ) => {
      e.preventDefault();
      const state = store();
      let selection = state.selection;
      if (cellId) {
        const cell = graph.getCellById(cellId);
        const nodeIds = cell?.isNode() ? [cellId] : [];
        const edgeIds = cell?.isEdge() ? [cellId] : [];
        if (nodeIds.length || edgeIds.length) {
          selection = { nodeIds, edgeIds };
          setSelection(selection);
          graph.cleanSelection();
          if (cell) graph.select(cell);
        }
      }
      const editId = editableCellId(cellId);
      contextMenuRef.current?.({
        clientX: e.clientX,
        clientY: e.clientY,
        localX,
        localY,
        cellId,
        canDelete: deletableFromCell(cellId, selection),
        canEdit:
          Boolean(editId) ||
          selection.nodeIds.some(
            (id) => id.startsWith("vm:") || id.startsWith("net:"),
          ),
      });
    };

    graph.on("blank:contextmenu", ({ e, x, y }) => {
      openContextMenu(e, x, y, null);
    });

    graph.on("cell:contextmenu", ({ e, x, y, cell }) => {
      openContextMenu(e, x, y, cell.id);
    });

    graph.on("selection:changed", ({ selected }) => {
      if (store().ui.projecting) return;
      setSelection({
        nodeIds: selected.filter((c) => c.isNode()).map((c) => c.id),
        edgeIds: selected.filter((c) => c.isEdge()).map((c) => c.id),
      });
    });

    graph.on("node:moved", ({ node }) => {
      if (store().ui.projecting) return;
      const pos = node.position();
      if (node.id.startsWith("net:")) {
        const positions: Array<{ nodeId: string; x: number; y: number }> = [
          { nodeId: node.id, x: pos.x, y: pos.y },
        ];
        for (const child of node.getChildren() ?? []) {
          if (!child.isNode() || !child.id.startsWith("vm:")) continue;
          const cp = child.position();
          positions.push({ nodeId: child.id, x: cp.x, y: cp.y });
        }
        moveNodes(positions);
        return;
      }
      moveNode(node.id, pos.x, pos.y);
    });

    graph.on("node:resized", ({ node }) => {
      if (store().ui.projecting) return;
      if (!node.id.startsWith("net:")) return;
      const size = node.size();
      resizeNode(node.id, size.width, size.height);
    });

    graph.on("node:change:parent", ({ node, current }) => {
      if (store().ui.projecting) return;
      if (!node.id.startsWith("vm:")) return;
      if (typeof current !== "string" || !current.startsWith("net:")) return;
      const vmName = node.id.slice(3);
      const networkName = current.slice(4);
      const state = store();
      const vm = state.env?.spec.vms[vmName];
      if (!vm) return;
      // Multi-homed: parenting is visual-only; attachments stay via edges.
      if (vm.networks.length > 1) return;
      const pos = node.position();
      if (vm.networks[0]?.name === networkName) {
        moveNode(node.id, pos.x, pos.y);
        return;
      }
      assignPrimaryNetwork(vmName, networkName, { x: pos.x, y: pos.y });
    });

    graph.on("edge:connected", ({ edge, isNew }) => {
      if (store().ui.projecting) return;
      const source = edge.getSourceCellId();
      const target = edge.getTargetCellId();
      const sourcePort = edge.getSourcePortId() ?? "";
      const targetPort = edge.getTargetPortId() ?? "";
      const state = store();
      if (!state.env) return;
      const existing = new Set<string>();
      for (const [vmName, vm] of Object.entries(state.env.spec.vms)) {
        for (const nic of vm.networks) existing.add(`${vmName}|${nic.name}`);
      }

      const data = edge.getData() as
        | { kind?: string; vmName?: string; networkName?: string }
        | null;

      if (!isNew && data?.kind === "attachment" && data.vmName && data.networkName) {
        const ignoreAttachment = `${data.vmName}|${data.networkName}`;
        const result = validateConnection({
          source: { nodeId: source ?? "", portId: sourcePort },
          target: { nodeId: target ?? "", portId: targetPort },
          existingAttachments: existing,
          existingRoutes: new Set(),
          env: state.env,
          ignoreAttachment,
        });
        if (
          result.ok &&
          result.kind === "attachment" &&
          result.vmName &&
          result.networkName
        ) {
          if (result.networkName !== data.networkName) {
            reassignNic(result.vmName, data.networkName, result.networkName);
          }
          return;
        }
        setConnectionHint(
          result.ok
            ? { key: "conn.reconnectInvalid" }
            : { key: result.reasonKey, params: result.reasonParams },
        );
        const latest = store();
        if (latest.env) {
          setProjecting(true);
          try {
            projectToGraph(graph, latest.env, latest.diagram);
          } finally {
            requestAnimationFrame(() => setProjecting(false));
          }
        }
        return;
      }

      // New rubber-band edge — domain owns the real edge
      if (isNew) graph.removeEdge(edge.id);
      if (!source || !target) return;
      const result = validateConnection({
        source: { nodeId: source, portId: sourcePort },
        target: { nodeId: target, portId: targetPort },
        existingAttachments: existing,
        existingRoutes: new Set(),
        env: state.env,
      });
      if (
        result.ok &&
        result.kind === "attachment" &&
        result.vmName &&
        result.networkName
      ) {
        attachNic(result.vmName, result.networkName);
      } else if (!result.ok) {
        setConnectionHint({
          key: result.reasonKey,
          params: result.reasonParams,
        });
      }
    });

    const isEditableTarget = (el: EventTarget | null) => {
      if (!(el instanceof HTMLElement)) return false;
      return Boolean(
        el.closest("input, textarea, select, [contenteditable=true]"),
      );
    };

    graph.bindKey(["backspace", "delete"], () => {
      if (isEditableTarget(document.activeElement)) return;
      deleteSelection();
    });
    graph.bindKey(["meta+z", "ctrl+z"], () => undo());
    graph.bindKey(["meta+shift+z", "ctrl+shift+z", "ctrl+y", "meta+y"], () =>
      redo(),
    );
    graph.bindKey("escape", () => {
      graph.cleanSelection();
      setConnectionHint(null);
    });
    graph.bindKey(["meta+a", "ctrl+a"], (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      e.preventDefault();
      graph.select(graph.getCells());
    });

    return () => {
      dispose();
      graphRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const graph = graphRef.current;
    if (!graph || !env) return;
    setProjecting(true);
    try {
      projectToGraph(graph, env, diagram);
      for (const cell of graph.getNodes()) {
        const hasError = errorNodeIds.has(cell.id);
        if (cell.shape === "vzctl-vm" || cell.shape === "vzctl-network") {
          cell.attr("body/strokeWidth", hasError ? 2.5 : 1.5);
          if (hasError) {
            cell.attr("body/stroke", "#9b2c2c");
          }
        }
      }
    } finally {
      requestAnimationFrame(() => setProjecting(false));
    }
  }, [env, diagram, errorNodeIds, setProjecting, locale]);

  void projecting;

  return (
    <div className="topology-canvas-wrap">
      <div
        ref={containerRef}
        className="topology-canvas"
        role="application"
        aria-label={t("topo.canvasAria")}
        onContextMenu={(e) => e.preventDefault()}
      />
      <div ref={minimapRef} className="topology-minimap" aria-hidden />
    </div>
  );
}
