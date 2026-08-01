import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";

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
        <h3 id={titleId}>vmnet-CIDR verwaist</h3>
        <p>
          <code>{orphanedCidr}</code> ist nach einem unclean Exit von{" "}
          <code>vz-net</code> auf dem Host blockiert (Status 1001). Bis zum Reboot
          oder einer neuen CIDR schlägt der Netz-Reserve fehl.
        </p>
        <ul className="vmnet-orphan-options">
          <li>
            <strong>Host neu starten</strong> — gibt verwaiste Reservierungen
            frei; danach Supervisor starten und erneut up.
          </li>
          <li>
            <strong>CIDR wechseln</strong> — Config auf{" "}
            <code>{suggestedCidr}</code> umschreiben (IPs behalten Host-Offset)
            und up erneut versuchen.
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
            Abbrechen
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
            {busy && pending === "reboot" ? "Neustart…" : "Host neu starten"}
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
              ? "CIDR wechseln…"
              : `CIDR → ${suggestedCidr}`}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
