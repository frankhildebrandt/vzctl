import { useState } from "react";
import { copyText } from "@/lib/clipboard";
import { localeToBcp47 } from "@/lib/i18n/detect";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { useSettingsStore } from "@/store/settingsStore";
import {
  formatAllErrorsForClipboard,
  formatErrorForClipboard,
  useErrorStore,
  type ReportedError,
} from "@/store/errorStore";

function formatTime(ts: number, locale: string): string {
  try {
    return new Date(ts).toLocaleString(locale);
  } catch {
    return String(ts);
  }
}

function sourceLabel(
  source: ReportedError["source"],
  t: ReturnType<typeof useT>,
): string {
  return t(`errors.source.${source}` as MessageKey);
}

function detailsText(details: unknown): string {
  if (details == null) return "";
  if (typeof details === "string") return details;
  try {
    return JSON.stringify(details, null, 2);
  } catch {
    return String(details);
  }
}

export function ErrorsPage() {
  const t = useT();
  const errors = useErrorStore((s) => s.errors);
  const clear = useErrorStore((s) => s.clear);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [copiedAll, setCopiedAll] = useState(false);

  const flash = (id: string | null) => {
    setCopiedId(id);
    window.setTimeout(() => setCopiedId(null), 1500);
  };

  const onCopyOne = (entry: ReportedError) => {
    void copyText(formatErrorForClipboard(entry)).then((ok) => {
      if (ok) flash(entry.id);
    });
  };

  const onCopyAll = () => {
    void copyText(formatAllErrorsForClipboard(errors)).then((ok) => {
      if (!ok) return;
      setCopiedAll(true);
      window.setTimeout(() => setCopiedAll(false), 1500);
    });
  };

  return (
    <section>
      <header className="detail-heading" style={{ marginBottom: "1rem" }}>
        <h2 className="section-title">{t("errors.title")}</h2>
        <p className="muted">{t("errors.subtitle")}</p>
      </header>

      <div className="card summary-card">
        <div className="summary-row">
          <span className="muted">
            {errors.length === 0
              ? t("errors.none")
              : t("errors.count", { n: errors.length })}
          </span>
          <div className="errors-toolbar">
            <button
              type="button"
              className="secondary"
              disabled={errors.length === 0}
              onClick={onCopyAll}
            >
              {copiedAll ? t("common.copied") : t("errors.copyAll")}
            </button>
            <button
              type="button"
              className="secondary"
              disabled={errors.length === 0}
              onClick={() => clear()}
            >
              {t("errors.clear")}
            </button>
          </div>
        </div>
      </div>

      {errors.length === 0 ? (
        <div className="card">
          <p className="muted" style={{ margin: 0 }}>
            {t("errors.empty")}
          </p>
        </div>
      ) : (
        <div className="errors-list">
          {errors.map((entry) => (
            <ErrorEntryCard
              key={entry.id}
              entry={entry}
              copied={copiedId === entry.id}
              onCopy={() => onCopyOne(entry)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function ErrorEntryCard({
  entry,
  copied,
  onCopy,
}: {
  entry: ReportedError;
  copied: boolean;
  onCopy: () => void;
}) {
  const t = useT();
  const locale = useSettingsStore((s) => s.locale);
  const details = detailsText(entry.details);

  return (
    <article className="card error-card error-entry">
      <div className="error-entry-head">
        <div className="error-entry-meta">
          <span className="badge danger">{sourceLabel(entry.source, t)}</span>
          <time className="muted" dateTime={new Date(entry.ts).toISOString()}>
            {formatTime(entry.ts, localeToBcp47(locale))}
          </time>
          {entry.code ? <code className="error-entry-code">{entry.code}</code> : null}
          {entry.status != null && entry.status > 0 ? (
            <span className="muted">
              {t("errors.http", { status: entry.status })}
            </span>
          ) : null}
        </div>
        <button type="button" className="secondary out-copy-inline" onClick={onCopy}>
          {copied ? t("common.copied") : t("common.copy")}
        </button>
      </div>
      <p className="error-entry-message">{entry.message}</p>
      <dl className="kv error-entry-kv">
        {entry.method || entry.path ? (
          <div className="kv-row">
            <dt>{t("errors.field.request")}</dt>
            <dd>
              <code>
                {entry.method ?? "?"} {entry.path ?? "?"}
              </code>
            </dd>
          </div>
        ) : null}
        {entry.route ? (
          <div className="kv-row">
            <dt>{t("errors.field.route")}</dt>
            <dd>
              <code>{entry.route}</code>
            </dd>
          </div>
        ) : null}
        {entry.queryKey ? (
          <div className="kv-row">
            <dt>{t("errors.field.queryKey")}</dt>
            <dd>
              <code>{entry.queryKey}</code>
            </dd>
          </div>
        ) : null}
        {entry.mutationKey ? (
          <div className="kv-row">
            <dt>{t("errors.field.mutationKey")}</dt>
            <dd>
              <code>{entry.mutationKey}</code>
            </dd>
          </div>
        ) : null}
      </dl>
      {details ? (
        <pre className="error-entry-details">{details}</pre>
      ) : null}
      {entry.stack ? (
        <details className="error-entry-stack">
          <summary>{t("errors.field.stack")}</summary>
          <pre>{entry.stack}</pre>
        </details>
      ) : null}
    </article>
  );
}
