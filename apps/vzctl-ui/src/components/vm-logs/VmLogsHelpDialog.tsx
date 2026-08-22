import { Button, Dialog } from "@/components/ui";
import { useT } from "@/lib/i18n";

type Props = {
  open: boolean;
  onClose: () => void;
};

export function VmLogsHelpDialog({ open, onClose }: Props) {
  const t = useT();

  return (
    <Dialog
      open={open}
      title={t("vmLogs.help")}
      onCancel={onClose}
      role="dialog"
      actions={
        <Button tone="secondary" onClick={onClose}>
          {t("dialog.close")}
        </Button>
      }
    >
      <pre className="vm-logs-help-body">{t("vmLogs.helpText")}</pre>
    </Dialog>
  );
}
