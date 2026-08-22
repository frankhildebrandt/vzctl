import { useEffect, useState } from "react";
import { Button, Dialog } from "@/components/ui";
import { fetchGuestLogShare, type LogsQuery } from "@/lib/guestLogs";
import { useT } from "@/lib/i18n";

type Props = {
  open: boolean;
  vmId: string;
  source: string;
  index: number;
  query: LogsQuery;
  context?: number;
  onClose: () => void;
};

export function VmLogsShareDialog({
  open,
  vmId,
  source,
  index,
  query,
  context,
  onClose,
}: Props) {
  const t = useT();
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || index < 0) return;
    let cancelled = false;
    setError(null);
    void (async () => {
      try {
        const data = await fetchGuestLogShare(vmId, source, index, query, context);
        if (!cancelled) setText(data.text ?? "");
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, vmId, source, index, query, context]);

  async function copyShare() {
    if (!text) return;
    await navigator.clipboard.writeText(text);
  }

  function downloadShare() {
    const blob = new Blob([text], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "iwatch-share.txt";
    link.click();
    URL.revokeObjectURL(url);
  }

  return (
    <Dialog
      open={open}
      title={t("vmLogs.share")}
      onCancel={onClose}
      role="dialog"
      actions={
        <>
          <Button tone="secondary" onClick={() => void copyShare()}>
            {t("vmLogs.copy")}
          </Button>
          <Button tone="secondary" onClick={downloadShare}>
            {t("vmLogs.download")}
          </Button>
          <Button tone="secondary" onClick={onClose}>
            {t("dialog.close")}
          </Button>
        </>
      }
    >
      {error ? <p>{error}</p> : <pre className="vm-logs-detail-body">{text}</pre>}
    </Dialog>
  );
}
