import { ApplyRecoveryActions } from "@/components/ApplyRecoveryActions";
import { Alert, Muted } from "@/components/ui";
import type { ApplyRecovery } from "@/lib/applyRecovery";
import { useT } from "@/lib/i18n";

const record = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
const text = (value: unknown, max: number) => typeof value === "string" && value.length > 0 && value.length <= max ? value : undefined;

/** Persisted CLI/UI operation state, independent of this window's mutation history. */
export function ApplyJournalPanel({ raw, path, onDone, disabled = false }: {
  raw?: string; path: string; onDone?: () => void; disabled?: boolean;
}) {
  const t = useT();
  let section: Record<string, unknown> | null;
  try { section = record(record(record(JSON.parse(raw ?? "{}"))?.sections)?.apply); }
  catch { section = null; }
  // Compatibility with supervisors that do not yet expose persisted operations.
  if (!section) return null;
  const data = record(section.data);
  if (section.ok !== true || !data) return <Alert title={t("journal.unavailable")}><p>{t("journal.unavailableHint")}</p></Alert>;
  if (data.journal === null) return null;
  const journal = record(data.journal);
  const id = text(journal?.id, 256);
  const step = text(journal?.step, 128);
  const status = text(journal?.status, 16);
  if (!journal || !id || !step || !status || !["failed", "pending", "running"].includes(status)) {
    return <Alert title={t("journal.unavailable")}><p>{t("journal.unavailableHint")}</p></Alert>;
  }
  const lease = record(data.lease);
  const holder = text(lease?.holder, 256);
  const expiresAt = text(lease?.expires_at, 128);
  const expires = expiresAt ? Date.parse(expiresAt) : NaN;
  // An unparseable lease is not permission to resume. The server rechecks it at
  // every explicit mutation; a status snapshot never grants ownership.
  const blocked = data.lease != null && (!holder || !Number.isFinite(expires) || expires > Date.now());
  const pid = holder ? /:(\d+)$/.exec(holder) : null;
  const info: ApplyRecovery = { kind: blocked ? "lease" : "journal", journalId: id, step,
    ...(holder ? { holder } : {}), ...(expiresAt ? { expiresAt } : {}),
    ...(pid ? { pid: Number(pid[1]) } : {}) };
  const error = text(journal.error, 4096);
  const mode = text(journal.mode, 16);
  const updated = text(journal.updated_at, 128);
  return <Alert title={t(status === "failed" ? "journal.failed" : "journal.unfinished")}>
    <p>{error ?? t("journal.noError")}</p>
    <Muted>{[mode && ["apply", "up", "down"].includes(mode) ? mode : null, updated].filter(Boolean).join(" · ")}</Muted>
    <ApplyRecoveryActions key={id} info={info} path={path} onDone={onDone} disabled={disabled} />
  </Alert>;
}
