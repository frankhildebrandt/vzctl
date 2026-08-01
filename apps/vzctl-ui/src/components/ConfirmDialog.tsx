import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { useT } from "@/lib/i18n";

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel,
  busy = false,
  tone = "danger",
  error = null,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  busy?: boolean;
  tone?: "danger" | "default";
  error?: string | null;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const t = useT();
  const resolvedCancel = cancelLabel ?? t("dialog.cancel");
  const resolvedConfirm =
    confirmLabel ??
    (tone === "danger" ? t("dialog.delete") : t("dialog.confirmDefault"));
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    confirmRef.current?.focus();

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
  }, [open, busy, onCancel]);

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
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <h3 id={titleId}>{title}</h3>
        <p>{message}</p>
        {error ? <p className="form-error confirm-error">{error}</p> : null}
        <div className="row confirm-actions">
          <button
            ref={cancelRef}
            type="button"
            className="secondary"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onCancel();
            }}
          >
            {resolvedCancel}
          </button>
          <button
            ref={confirmRef}
            type="button"
            className={tone === "danger" ? "danger" : undefined}
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onConfirm();
            }}
          >
            {busy ? t("dialog.busy", { label: resolvedConfirm }) : resolvedConfirm}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
