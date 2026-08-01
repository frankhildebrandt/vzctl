import { useMemo, useRef, useState, type MouseEvent } from "react";
import { useEditorStore } from "@/store/editorStore";
import {
  PaletteKindIcon,
  type PaletteKind,
} from "@/features/topology-editor/PaletteIcons";

const PALETTE: Array<{
  id: string;
  label: string;
  description: string;
  kind: PaletteKind;
}> = [
  {
    id: "network",
    label: "Netzwerk",
    description: "CIDR · Container",
    kind: "network",
  },
  {
    id: "vm",
    label: "Host",
    description: "Compute · NICs",
    kind: "vm",
  },
  {
    id: "router",
    label: "Router",
    description: "roles: [router]",
    kind: "router",
  },
  {
    id: "docker",
    label: "Docker",
    description: "roles: [docker, router]",
    kind: "docker",
  },
];

type Props = {
  onClickCreate: (kind: PaletteKind) => void;
  /** X6 Dnd start — materializes a preview node under the cursor. */
  onDragStart: (kind: PaletteKind, event: MouseEvent) => void;
};

export function TopologyPalette({ onClickCreate, onDragStart }: Props) {
  const filter = useEditorStore((s) => s.ui.paletteFilter);
  const setFilter = useEditorStore((s) => s.setPaletteFilter);
  const [dragging, setDragging] = useState<string | null>(null);
  const draggedRef = useRef(false);

  const items = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return PALETTE;
    return PALETTE.filter(
      (p) =>
        p.label.toLowerCase().includes(q) ||
        p.description.toLowerCase().includes(q),
    );
  }, [filter]);

  return (
    <aside className="topology-palette" aria-label="Komponentenpalette">
      <h3 className="topology-panel-title">Komponenten</h3>
      <label className="topology-field">
        <span className="sr-only">Filter</span>
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filtern…"
          aria-label="Palette filtern"
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
                setDragging(item.id);
                const mark = () => {
                  draggedRef.current = true;
                };
                window.addEventListener("mousemove", mark, { once: true });
                window.addEventListener(
                  "mouseup",
                  () => {
                    setDragging(null);
                    window.removeEventListener("mousemove", mark);
                  },
                  { once: true },
                );
                onDragStart(item.kind, e);
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
                <strong>{item.label}</strong>
                <span className="muted">{item.description}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
      <p className="muted topology-palette-hint">
        Ziehen materialisiert eine Vorschau auf dem Canvas. Klick fügt in der
        Mitte ein.
      </p>
    </aside>
  );
}
