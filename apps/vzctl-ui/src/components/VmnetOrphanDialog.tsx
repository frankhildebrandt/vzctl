import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useT } from "@/lib/i18n";

export type VmnetOrphanChoice = "reboot" | "cidr";

export function VmnetOrphanDialog({
  open,
  orphanedCidr,
  suggestedCidr,
  busy = false,
  error = null,
  onChoose,
  onCancel,
}: {
  open: boolean;
  orphanedCidr: string;
  suggestedCidr: string;
  busy?: boolean;
  error?: string | null;
  onChoose: (choice: VmnetOrphanChoice) => void;
  onCancel: () => void;
}) {
  const t = useT();
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  const [pending, setPending] = useState<VmnetOrphanChoice | null>(null);

  useEffect(() => {
    if (!open) {
      setPending(null);
      return;
    }
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    primaryRef.current?.focus();

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        event.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", onKey);
    };
  }, [open, busy, onCancel]);

  if (!open || typeof document === "undefined") return null;

  function choose(choice: VmnetOrphanChoice) {
    if (busy) return;
    setPending(choice);
    onChoose(choice);
  }

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
        className="confirm-dialog vmnet-orphan-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <h3 id={titleId}>{t("orphan.title")}</h3>
        <p>{t("orphan.body", { cidr: orphanedCidr })}</p>
        <ul className="vmnet-orphan-options">
          <li>
            <strong>{t("orphan.optionRebootTitle")}</strong> — {t("orphan.optionRebootHint")}
          </li>
          <li>
            <strong>{t("orphan.optionCidrTitle")}</strong> —{" "}
            {t("orphan.optionCidrHint", { cidr: suggestedCidr })}
          </li>
        </ul>
        {error ? <p className="form-error confirm-error">{error}</p> : null}
        <div className="row confirm-actions vmnet-orphan-actions">
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              onCancel();
            }}
          >
            {t("dialog.cancel")}
          </button>
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              choose("reboot");
            }}
          >
            {busy && pending === "reboot" ? t("orphan.rebootBusy") : t("orphan.reboot")}
          </button>
          <button
            ref={primaryRef}
            type="button"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              choose("cidr");
            }}
          >
            {busy && pending === "cidr"
              ? t("orphan.cidrSwitchBusy")
              : t("orphan.cidrSwitch", { cidr: suggestedCidr })}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
