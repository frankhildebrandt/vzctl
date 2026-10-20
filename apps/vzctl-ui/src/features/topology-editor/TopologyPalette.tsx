import { useEffect, useMemo, useRef, useState } from "react";
import { useEditorStore } from "@/store/editorStore";
import {
  PaletteKindIcon,
  type PaletteKind,
} from "@/features/topology-editor/PaletteIcons";
import { useT } from "@/lib/i18n";

type PaletteItem = {
  id: string;
  labelKey: "topo.palette.network" | "topo.palette.vm" | "topo.palette.router" | "topo.palette.docker";
  descriptionKey: "topo.palette.networkDesc" | "topo.palette.vmDesc" | "topo.palette.routerDesc" | "topo.palette.dockerDesc";
  kind: PaletteKind;
};

const PALETTE: PaletteItem[] = [
  {
    id: "network",
    labelKey: "topo.palette.network",
    descriptionKey: "topo.palette.networkDesc",
    kind: "network",
  },
  {
    id: "vm",
    labelKey: "topo.palette.vm",
    descriptionKey: "topo.palette.vmDesc",
    kind: "vm",
  },
  {
    id: "router",
    labelKey: "topo.palette.router",
    descriptionKey: "topo.palette.routerDesc",
    kind: "router",
  },
  {
    id: "docker",
    labelKey: "topo.palette.docker",
    descriptionKey: "topo.palette.dockerDesc",
    kind: "docker",
  },
];

type Props = {
  onClickCreate: (kind: PaletteKind) => void;
  /** X6 Dnd start — materializes a preview node under the cursor. */
  onDragStart: (kind: PaletteKind, event: MouseEvent) => void;
};

export function TopologyPalette({ onClickCreate, onDragStart }: Props) {
  const t = useT();
  const filter = useEditorStore((s) => s.ui.paletteFilter);
  const setFilter = useEditorStore((s) => s.setPaletteFilter);
  const [dragging, setDragging] = useState<string | null>(null);
  const draggedRef = useRef(false);
  const cleanupDragRef = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanupDragRef.current?.(), []);

  const items = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return PALETTE;
    return PALETTE.filter((p) => {
      const label = t(p.labelKey).toLowerCase();
      const description = t(p.descriptionKey).toLowerCase();
      return label.includes(q) || description.includes(q);
    });
  }, [filter, t]);

  return (
    <aside className="topology-palette" aria-label={t("topo.paletteTitle")}>
      <h3 className="topology-panel-title">{t("topo.paletteTitle")}</h3>
      <label className="topology-field">
        <span className="sr-only">{t("topo.paletteFilter")}</span>
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("topo.paletteFilter")}
          aria-label={t("topo.paletteFilterAria")}
        />
      </label>
      <ul className="topology-palette-list">
        {items.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              className={`topology-palette-item${dragging === item.id ? " dragging" : ""}`}
              aria-grabbed={dragging === item.id}
              onMouseDown={(e) => {
                if (e.button !== 0) return;
                draggedRef.current = false;
                cleanupDragRef.current?.();
                const start = { x: e.clientX, y: e.clientY };
                const cleanup = () => {
                  window.removeEventListener("mousemove", move);
                  window.removeEventListener("mouseup", up);
                  cleanupDragRef.current = null;
                };
                const move = (event: MouseEvent) => {
                  if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < 5) return;
                  window.removeEventListener("mousemove", move);
                  draggedRef.current = true;
                  setDragging(item.id);
                  // Starting X6 on mousedown installs an overlay that swallows
                  // an ordinary click. Start only once this is actually a drag.
                  onDragStart(item.kind, event);
                };
                const up = () => {
                  setDragging(null);
                  cleanup();
                };
                cleanupDragRef.current = cleanup;
                window.addEventListener("mousemove", move);
                window.addEventListener("mouseup", up, { once: true });
              }}
              onClick={() => {
                if (draggedRef.current) {
                  draggedRef.current = false;
                  return;
                }
                onClickCreate(item.kind);
              }}
            >
              <span className="topology-palette-icon" aria-hidden>
                <PaletteKindIcon kind={item.kind} size={36} />
              </span>
              <span className="topology-palette-text">
                <strong>{t(item.labelKey)}</strong>
                <span className="muted">{t(item.descriptionKey)}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
      <p className="muted topology-palette-hint">{t("topo.paletteHint")}</p>
    </aside>
  );
}
