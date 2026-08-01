import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent,
} from "react";
import { createPortal } from "react-dom";
import type { Dnd, Graph } from "@antv/x6";
import { TopologyCanvas } from "@/features/topology-editor/TopologyCanvas";
import { TopologyPalette } from "@/features/topology-editor/TopologyPalette";
import { TopologyInspector } from "@/features/topology-editor/TopologyInspector";
import {
  TopologyContextMenu,
  type ContextMenuState,
} from "@/features/topology-editor/TopologyContextMenu";
import { useEditorStore } from "@/store/editorStore";
import { saveProjectFlexible } from "@/features/persistence/projectIo";
import { layoutByNetwork } from "@/diagram/projections/layout";
import { findNetworkAtPoint } from "@/diagram/projections/projectToGraph";
import { runVzctl, parseEnvelope } from "@/lib/vzctl";
import {
  createPalettePreviewNode,
  paletteNameBase,
  paletteRoles,
} from "@/diagram/interactions/paletteDnd";
import type { PaletteKind } from "@/features/topology-editor/PaletteIcons";
import {
  IconButton,
  IconFit,
  IconLayout,
  IconRedo,
  IconSave,
  IconUndo,
} from "@/components/IconButton";
import { useT } from "@/lib/i18n";

type Props = {
  projectPath: string;
  toolbarHost?: HTMLElement | null;
};

function focusInspectorName() {
  requestAnimationFrame(() => {
    const input = document.querySelector<HTMLInputElement>(
      ".topology-inspector input[data-topology-name]",
    );
    if (!input) return;
    input.focus();
    input.select();
  });
}

export function TopologyEditor({ projectPath, toolbarHost }: Props) {
  const t = useT();
  const graphRef = useRef<Graph | null>(null);
  const dndRef = useRef<Dnd | null>(null);
  const env = useEditorStore((s) => s.env);
  const diagram = useEditorStore((s) => s.diagram);
  const dirty = useEditorStore((s) => s.ui.dirty);
  const lastError = useEditorStore((s) => s.ui.lastError);
  const connectionHint = useEditorStore((s) => s.ui.connectionHint);
  const createNetwork = useEditorStore((s) => s.createNetwork);
  const createVm = useEditorStore((s) => s.createVm);
  const deleteSelection = useEditorStore((s) => s.deleteSelection);
  const setSelection = useEditorStore((s) => s.setSelection);
  const undo = useEditorStore((s) => s.undo);
  const redo = useEditorStore((s) => s.redo);
  const markSaved = useEditorStore((s) => s.markSaved);
  const setLastError = useEditorStore((s) => s.setLastError);
  const replaceDiagram = useEditorStore((s) => s.replaceDiagram);
  const validation = useEditorStore((s) => s.validation);
  const pastLen = useEditorStore((s) => s.past.length);
  const futureLen = useEditorStore((s) => s.future.length);

  const [busy, setBusy] = useState<string | null>(null);
  const [validateMsg, setValidateMsg] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(
    null,
  );
  const [graphReady, setGraphReady] = useState(0);
  const fitOnceForPath = useRef<string | null>(null);

  useEffect(() => {
    fitOnceForPath.current = null;
  }, [projectPath]);

  useEffect(() => {
    const graph = graphRef.current;
    if (!env || !graph || graphReady === 0) return;
    if (fitOnceForPath.current === projectPath) return;
    fitOnceForPath.current = projectPath;
    requestAnimationFrame(() => {
      graph.zoomToFit({ padding: 32, maxScale: 1.2 });
    });
  }, [env, projectPath, diagram, graphReady]);

  const uniqueName = useCallback(
    (base: string, existing: Set<string>) => {
      if (!existing.has(base)) return base;
      let i = 2;
      while (existing.has(`${base}-${i}`)) i += 1;
      return `${base}-${i}`;
    },
    [],
  );

  const materializeAt = useCallback(
    (kind: PaletteKind, x: number, y: number) => {
      if (!env) return;
      if (kind === "network") {
        const name = uniqueName(
          "net",
          new Set(Object.keys(env.spec.networks)),
        );
        const octet = 80 + Object.keys(env.spec.networks).length * 10;
        createNetwork(name, `10.${octet}.0.0/24`, "shared", { x, y });
        return;
      }
      const name = uniqueName(
        paletteNameBase(kind),
        new Set(Object.keys(env.spec.vms)),
      );
      const graph = graphRef.current;
      const networkName = graph
        ? (findNetworkAtPoint(graph, x + 100, y + 50) ?? undefined)
        : undefined;
      createVm(name, {
        roles: paletteRoles(kind),
        networkName,
        position: { x, y },
      });
    },
    [env, createNetwork, createVm, uniqueName],
  );

  const createAtCenter = useCallback(
    (kind: PaletteKind) => {
      const graph = graphRef.current;
      if (!graph) {
        materializeAt(kind, 200, 200);
        return;
      }
      const area = graph.container.getBoundingClientRect();
      const local = graph.clientToLocal(
        area.left + area.width / 2,
        area.top + area.height / 2,
      );
      materializeAt(kind, local.x - 110, local.y - 60);
    },
    [materializeAt],
  );

  const onPaletteDragStart = useCallback(
    (kind: PaletteKind, event: MouseEvent) => {
      const graph = graphRef.current;
      const dnd = dndRef.current;
      if (!graph || !dnd) return;
      const preview = createPalettePreviewNode(graph, kind);
      dnd.start(preview, event.nativeEvent);
    },
    [],
  );

  const onContextAdd = useCallback(
    (kind: PaletteKind) => {
      if (!contextMenu) return;
      materializeAt(kind, contextMenu.localX - 100, contextMenu.localY - 50);
    },
    [contextMenu, materializeAt],
  );

  const onContextDelete = useCallback(() => {
    deleteSelection();
  }, [deleteSelection]);

  const onContextEdit = useCallback(() => {
    if (!contextMenu) return;
    let editId = contextMenu.cellId;
    if (editId?.startsWith("igw:")) {
      editId = `net:${editId.slice(4)}`;
    }
    if (editId && (editId.startsWith("vm:") || editId.startsWith("net:"))) {
      setSelection({ nodeIds: [editId], edgeIds: [] });
      const graph = graphRef.current;
      const cell = graph?.getCellById(editId);
      if (graph && cell) {
        graph.cleanSelection();
        graph.select(cell);
      }
    }
    focusInspectorName();
  }, [contextMenu, setSelection]);

  const onSave = async () => {
    if (!env) return;
    setBusy(t("topo.saveBusy"));
    setValidateMsg(null);
    try {
      await saveProjectFlexible(projectPath, env, diagram);
      markSaved();
      try {
        const raw = await runVzctl(projectPath, "validate");
        const envelope = parseEnvelope(raw);
        if (envelope.status === "fail" || (envelope.exit_code ?? 0) !== 0) {
          setValidateMsg(t("topo.savedValidateFail"));
        } else {
          setValidateMsg(t("topo.savedValidated"));
        }
      } catch {
        setValidateMsg(t("topo.savedNoValidate"));
      }
    } catch (err) {
      setLastError(String(err instanceof Error ? err.message : err));
    } finally {
      setBusy(null);
    }
  };

  const onAutoLayout = () => {
    if (!env) return;
    const nodes = layoutByNetwork(env);
    replaceDiagram({
      ...diagram,
      nodes: { ...diagram.nodes, ...nodes },
    });
  };

  const onFit = () => {
    graphRef.current?.zoomToFit({ padding: 32, maxScale: 1.2 });
  };

  const errorCount = validation.filter((v) => v.severity === "error").length;
  const saveLabel = busy ?? (dirty ? t("topo.saveDirty") : t("topo.save"));
  const showHints = validation.length > 0 || Boolean(validateMsg);

  const toolbar = (
    <>
      <IconButton
        label={saveLabel}
        showLabel
        disabled={Boolean(busy)}
        tone="primary"
        onClick={() => void onSave()}
      >
        <IconSave />
      </IconButton>
      <IconButton
        label={t("topo.undo")}
        showLabel
        disabled={pastLen === 0}
        tone="quiet"
        onClick={() => undo()}
      >
        <IconUndo />
      </IconButton>
      <IconButton
        label={t("topo.redo")}
        showLabel
        disabled={futureLen === 0}
        tone="quiet"
        onClick={() => redo()}
      >
        <IconRedo />
      </IconButton>
      <IconButton label={t("topo.fitView")} showLabel tone="quiet" onClick={onFit}>
        <IconFit />
      </IconButton>
      <IconButton
        label={t("topo.autoLayout")}
        showLabel
        tone="quiet"
        onClick={onAutoLayout}
      >
        <IconLayout />
      </IconButton>
      {showHints ? (
        <span
          className={`topology-toolbar-meta${errorCount > 0 ? " has-errors" : ""}`}
          aria-live="polite"
        >
          {errorCount > 0
            ? t("topo.errorsCount", { n: errorCount })
            : validation.length > 0
              ? t("topo.hintsCount", { n: validation.length })
              : null}
          {validateMsg
            ? `${validation.length > 0 || errorCount > 0 ? " · " : ""}${validateMsg}`
            : null}
        </span>
      ) : null}
    </>
  );

  return (
    <div className="topology-editor">
      {toolbarHost ? createPortal(toolbar, toolbarHost) : null}
      {lastError ? (
        <div className="card error-card topology-banner" role="alert">
          {lastError}
        </div>
      ) : null}
      {connectionHint ? (
        <div className="topology-banner warn" role="status">
          {t(connectionHint.key, connectionHint.params)}
        </div>
      ) : null}
      <div className="topology-body">
        <TopologyPalette
          onClickCreate={createAtCenter}
          onDragStart={onPaletteDragStart}
        />
        <TopologyCanvas
          onReady={(g, dnd) => {
            graphRef.current = g;
            dndRef.current = dnd;
            setGraphReady((n) => n + 1);
          }}
          onPaletteMaterialize={materializeAt}
          onContextMenu={setContextMenu}
        />
        <TopologyInspector />
      </div>
      <TopologyContextMenu
        menu={contextMenu}
        onClose={() => setContextMenu(null)}
        onAdd={onContextAdd}
        onDelete={onContextDelete}
        onEdit={onContextEdit}
      />
    </div>
  );
}
