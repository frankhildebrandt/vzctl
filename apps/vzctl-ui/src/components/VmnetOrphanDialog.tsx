import { useEffect, useRef, useState } from "react";
import { Button, Dialog, FieldError } from "@/components/ui";
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
  const primaryRef = useRef<HTMLButtonElement>(null);
  const [pending, setPending] = useState<VmnetOrphanChoice | null>(null);

  useEffect(() => {
    if (!open) setPending(null);
  }, [open]);

  function choose(choice: VmnetOrphanChoice) {
    if (busy) return;
    setPending(choice);
    onChoose(choice);
  }

  return (
    <Dialog
      open={open}
      title={t("orphan.title")}
      busy={busy}
      onCancel={onCancel}
      className="vmnet-orphan-dialog"
      actionsClassName="vmnet-orphan-actions"
      initialFocusRef={primaryRef}
      actions={
        <>
          <Button
            tone="secondary"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              onCancel();
            }}
          >
            {t("dialog.cancel")}
          </Button>
          <Button
            tone="secondary"
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              choose("reboot");
            }}
          >
            {busy && pending === "reboot" ? t("orphan.rebootBusy") : t("orphan.reboot")}
          </Button>
          <Button
            ref={primaryRef}
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              choose("cidr");
            }}
          >
            {busy && pending === "cidr"
              ? t("orphan.cidrSwitchBusy")
              : t("orphan.cidrSwitch", { cidr: suggestedCidr })}
          </Button>
        </>
      }
    >
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
      <FieldError message={error} className="confirm-error" />
    </Dialog>
  );
}
