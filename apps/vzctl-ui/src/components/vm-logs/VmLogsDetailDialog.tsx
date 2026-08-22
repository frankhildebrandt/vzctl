import { useEffect, useState } from "react";
import { Button, Dialog } from "@/components/ui";
import { fetchGuestLogLine, type LogsQuery } from "@/lib/guestLogs";
import { useT } from "@/lib/i18n";

type Props = {
  open: boolean;
  vmId: string;
  source: string;
  index: number;
  query: LogsQuery;
  onClose: () => void;
};

export function VmLogsDetailDialog({
  open,
  vmId,
  source,
  index,
  query,
  onClose,
}: Props) {
  const t = useT();
  const [body, setBody] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || index < 0) return;
    let cancelled = false;
    setError(null);
    void (async () => {
      try {
        const data = await fetchGuestLogLine(vmId, source, index, query);
        if (cancelled) return;
        const pretty = Object.entries(data.pretty ?? {})
          .map(([key, value]) => `${key}:\n${value}`)
          .join("\n\n");
        const line = data.line;
        setBody(
          `source: ${line.source ?? ""}\n` +
            `time: ${line.ts ?? ""}\n` +
            `index: ${line.index ?? ""}\n\n` +
            `${line.text}\n\n${pretty}`,
        );
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, vmId, source, index, query]);

  return (
    <Dialog
      open={open}
      title={t("vmLogs.detail")}
      onCancel={onClose}
      role="dialog"
      actions={
        <Button tone="secondary" onClick={onClose}>
          {t("dialog.close")}
        </Button>
      }
    >
      {error ? <p>{error}</p> : <pre className="vm-logs-detail-body">{body}</pre>}
    </Dialog>
  );
}
