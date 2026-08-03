import { useRef } from "react";
import { Button, Dialog, FieldError } from "@/components/ui";
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
  const confirmRef = useRef<HTMLButtonElement>(null);

  return (
    <Dialog
      open={open}
      title={title}
      busy={busy}
      onCancel={onCancel}
      initialFocusRef={confirmRef}
      actions={
        <>
          <Button
            tone="secondary"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onCancel();
            }}
          >
            {resolvedCancel}
          </Button>
          <Button
            ref={confirmRef}
            tone={tone === "danger" ? "danger" : "primary"}
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onConfirm();
            }}
          >
            {busy ? t("dialog.busy", { label: resolvedConfirm }) : resolvedConfirm}
          </Button>
        </>
      }
    >
      <p>{message}</p>
      <FieldError message={error} className="confirm-error" />
    </Dialog>
  );
}
