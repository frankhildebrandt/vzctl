import {
  Badge,
  Button,
  Card,
  CodeBlock,
  CopyButton,
  DescriptionList,
  EmptyState,
  Muted,
  PageHeader,
  SummaryCard,
  type DescriptionItem,
} from "@/components/ui";
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

  return (
    <section>
      <PageHeader
        layout="detail"
        title={t("errors.title")}
        subtitle={t("errors.subtitle")}
      />

      <SummaryCard
        meta={
          <Muted as="span">
            {errors.length === 0
              ? t("errors.none")
              : t("errors.count", { n: errors.length })}
          </Muted>
        }
        actions={
          <div className="errors-toolbar">
            <CopyButton
              value={formatAllErrorsForClipboard(errors)}
              disabled={errors.length === 0}
              label={t("errors.copyAll")}
            />
            <Button
              tone="secondary"
              disabled={errors.length === 0}
              onClick={() => clear()}
            >
              {t("errors.clear")}
            </Button>
          </div>
        }
      />

      {errors.length === 0 ? (
        <EmptyState message={t("errors.empty")} />
      ) : (
        <div className="errors-list">
          {errors.map((entry) => (
            <ErrorEntryCard key={entry.id} entry={entry} />
          ))}
        </div>
      )}
    </section>
  );
}

function ErrorEntryCard({
  entry,
}: {
  entry: ReportedError;
}) {
  const t = useT();
  const locale = useSettingsStore((s) => s.locale);
  const details = detailsText(entry.details);
  const fields: DescriptionItem[] = [];
  if (entry.method || entry.path) {
    fields.push({
      label: t("errors.field.request"),
      value: (
        <code>
          {entry.method ?? "?"} {entry.path ?? "?"}
        </code>
      ),
    });
  }
  if (entry.route) {
    fields.push({
      label: t("errors.field.route"),
      value: <code>{entry.route}</code>,
    });
  }
  if (entry.queryKey) {
    fields.push({
      label: t("errors.field.queryKey"),
      value: <code>{entry.queryKey}</code>,
    });
  }
  if (entry.mutationKey) {
    fields.push({
      label: t("errors.field.mutationKey"),
      value: <code>{entry.mutationKey}</code>,
    });
  }

  return (
    <Card as="article" tone="error" className="error-entry">
      <div className="error-entry-head">
        <div className="error-entry-meta">
          <Badge tone="danger">{sourceLabel(entry.source, t)}</Badge>
          <Muted as="span">
            {formatTime(entry.ts, localeToBcp47(locale))}
          </Muted>
          {entry.code ? <code className="error-entry-code">{entry.code}</code> : null}
          {entry.status != null && entry.status > 0 ? (
            <Muted as="span">
              {t("errors.http", { status: entry.status })}
            </Muted>
          ) : null}
        </div>
        <CopyButton
          value={formatErrorForClipboard(entry)}
          tone="inline"
        />
      </div>
      <p className="error-entry-message">{entry.message}</p>
      <DescriptionList stacked className="error-entry-kv" items={fields} />
      {details ? (
        <CodeBlock value={details} className="error-entry-details" tone="error" />
      ) : null}
      {entry.stack ? (
        <details className="error-entry-stack">
          <summary>{t("errors.field.stack")}</summary>
          <pre>{entry.stack}</pre>
        </details>
      ) : null}
    </Card>
  );
}
