import { useEffect, useRef, useState } from "react";
import type { PaletteKind } from "@/features/topology-editor/PaletteIcons";

export type ContextMenuState = {
  /** Screen coordinates for fixed positioning. */
  clientX: number;
  clientY: number;
  /** Graph-local drop/create point. */
  localX: number;
  localY: number;
  /** Cell under cursor, if any. */
  cellId: string | null;
  canDelete: boolean;
  canEdit: boolean;
};

type Props = {
  menu: ContextMenuState | null;
  onClose: () => void;
  onAdd: (kind: PaletteKind) => void;
  onDelete: () => void;
  onEdit: () => void;
};

const ADD_ITEMS: Array<{ kind: PaletteKind; label: string }> = [
  { kind: "network", label: "Netzwerk" },
  { kind: "vm", label: "Host" },
  { kind: "router", label: "Router" },
  { kind: "docker", label: "Docker" },
];

export function TopologyContextMenu({
  menu,
  onClose,
  onAdd,
  onDelete,
  onEdit,
}: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [submenuOpen, setSubmenuOpen] = useState(false);

  useEffect(() => {
    setSubmenuOpen(false);
  }, [menu?.clientX, menu?.clientY]);

  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onPointer = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onClose();
    };
    const onScroll = () => onClose();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onPointer, true);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onPointer, true);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [menu, onClose]);

  if (!menu) return null;

  const maxX = typeof window !== "undefined" ? window.innerWidth - 200 : menu.clientX;
  const maxY = typeof window !== "undefined" ? window.innerHeight - 160 : menu.clientY;
  const left = Math.max(8, Math.min(menu.clientX, maxX));
  const top = Math.max(8, Math.min(menu.clientY, maxY));

  return (
    <div
      ref={rootRef}
      className="topology-context-menu"
      style={{ left, top }}
      role="menu"
      aria-label="Kontextmenü"
    >
      <div
        className={`topology-context-item has-submenu${submenuOpen ? " open" : ""}`}
        onMouseEnter={() => setSubmenuOpen(true)}
        onMouseLeave={() => setSubmenuOpen(false)}
      >
        <button type="button" className="topology-context-btn" role="menuitem">
          Node hinzufügen
          <span className="topology-context-caret" aria-hidden>
            ›
          </span>
        </button>
        {submenuOpen ? (
          <div className="topology-context-submenu" role="menu">
            {ADD_ITEMS.map((item) => (
              <button
                key={item.kind}
                type="button"
                className="topology-context-btn"
                role="menuitem"
                onClick={() => {
                  onAdd(item.kind);
                  onClose();
                }}
              >
                {item.label}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <button
        type="button"
        className="topology-context-btn"
        role="menuitem"
        disabled={!menu.canDelete}
        onClick={() => {
          onDelete();
          onClose();
        }}
      >
        Löschen
      </button>
      <button
        type="button"
        className="topology-context-btn"
        role="menuitem"
        disabled={!menu.canEdit}
        onClick={() => {
          onEdit();
          onClose();
        }}
      >
        Bearbeiten
      </button>
    </div>
  );
}
