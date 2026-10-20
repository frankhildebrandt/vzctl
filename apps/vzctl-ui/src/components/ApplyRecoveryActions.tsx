import { useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ActionRow, Button, Muted } from "@/components/ui";
import { useT } from "@/lib/i18n";
import { runVzctl } from "@/lib/vzctl";
import { parseApplyRecovery, type ApplyRecovery } from "@/lib/applyRecovery";

export function ApplyRecoveryActions({ raw = "", info: providedInfo, path, onDone, disabled = false }: { raw?: string; info?: ApplyRecovery; path: string; onDone?: () => void; disabled?: boolean }) {
  const t = useT();
  const [busy, setBusy] = useState<"resume" | "abort" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [confirmAbort, setConfirmAbort] = useState(false);
  const info = providedInfo ?? parseApplyRecovery(raw);
  if (!info) return null;

  async function recover(mode: "resume" | "abort") {
    setBusy(mode);
    setMessage(null);
    try {
      await runVzctl(path, "apply", { [mode]: true });
      setMessage(t(mode === "resume" ? "result.journalResumeMsg" : "result.journalAbortMsg"));
      setConfirmAbort(false);
      onDone?.();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(null);
    }
  }

  return <div className="doctor-actions" style={{ marginTop: "0.75rem" }}>
    <dl>
      {info.holder ? <><dt>{t("recovery.holder")}</dt><dd>{info.holder}</dd></> : null}
      {info.pid !== undefined ? <><dt>PID</dt><dd>{info.pid}</dd></> : null}
      {info.expiresAt ? <><dt>{t("recovery.expires")}</dt><dd>{info.expiresAt}</dd></> : null}
      {info.journalId ? <><dt>Journal</dt><dd>{info.journalId}</dd></> : null}
      {info.step ? <><dt>{t("recovery.step")}</dt><dd>{info.step}</dd></> : null}
    </dl>
    <Muted>{t(info.kind === "lease" ? "recovery.leaseHint" : info.kind === "config-changed" ? "recovery.changedHint" : "result.incompleteJournalHint")}</Muted>
    {info.kind !== "lease" ? <ActionRow gap="md">
      {info.kind === "journal" ? <Button disabled={disabled || busy !== null} onClick={() => void recover("resume")}>
        {t(busy === "resume" ? "result.resumeBusy" : "result.resume")}
      </Button> : null}
      <Button tone="secondary" disabled={disabled || busy !== null} onClick={() => setConfirmAbort(true)}>
        {t(busy === "abort" ? "result.abortBusy" : "result.abort")}
      </Button>
    </ActionRow> : null}
    {message ? <p role="status">{message}</p> : null}
    <ConfirmDialog open={confirmAbort} title={t("recovery.abortTitle")} message={t("recovery.abortMessage")}
      confirmLabel={t("result.abort")} busy={busy === "abort"} onCancel={() => setConfirmAbort(false)}
      onConfirm={() => void recover("abort")} />
  </div>;
}
