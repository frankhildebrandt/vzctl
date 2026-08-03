import {
  useEffect,
  useId,
  useRef,
  type HTMLAttributes,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { cx } from "./cx";

export type DialogProps = {
  open: boolean;
  title: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
  busy?: boolean;
  onCancel: () => void;
  /** Extra class on the dialog panel. */
  className?: string;
  /** Extra class on the actions row. */
  actionsClassName?: string;
  role?: "alertdialog" | "dialog";
  /** Element to focus when opened. Defaults to first focusable in panel. */
  initialFocusRef?: RefObject<HTMLElement | null>;
  panelProps?: HTMLAttributes<HTMLDivElement>;
};

/**
 * Shared modal shell: portal, backdrop, escape, body scroll lock, focus trap.
 * ConfirmDialog / VmnetOrphanDialog compose on top.
 */
export function Dialog({
  open,
  title,
  children,
  actions,
  busy = false,
  onCancel,
  className,
  actionsClassName,
  role = "alertdialog",
  initialFocusRef,
  panelProps,
}: DialogProps) {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    const focusTarget =
      initialFocusRef?.current ??
      panelRef.current?.querySelector<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      );
    focusTarget?.focus();

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        event.preventDefault();
        onCancel();
        return;
      }
      if (event.key !== "Tab" || !panelRef.current) return;
      const focusable = panelRef.current.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    window.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", onKey);
    };
  }, [open, busy, onCancel, initialFocusRef]);

  if (!open || typeof document === "undefined") return null;

  return createPortal(
    <div
      className="confirm-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
    >
      <div
        ref={panelRef}
        className={cx("confirm-dialog", className)}
        role={role}
        aria-modal="true"
        aria-labelledby={titleId}
        {...panelProps}
      >
        <h3 id={titleId}>{title}</h3>
        {children}
        {actions != null ? (
          <div className={cx("row", "confirm-actions", actionsClassName)}>
            {actions}
          </div>
        ) : null}
      </div>
    </div>,
    document.body,
  );
}
