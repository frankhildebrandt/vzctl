import { ApplyRecoveryActions } from "@/components/ApplyRecoveryActions";
import { useMemo, useState, type ReactNode } from "react";
import { StackHealthSummary } from "@/components/StackHealthSummary";
import { HEALTH_LABEL_KEYS, parseStackHealth } from "@/lib/stackHealth";
import { api } from "@/lib/api";
import {
  Alert,
  Badge,
  Button,
  Card,
  CodeBlock,
  EmptyState,
  LoadingState,
  Muted,
  SummaryCard,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import { assertEnvelopeOk, parseEnvelope } from "@/lib/vzctl";

export type PanelKind = "diff" | "status" | "text" | "error" | "idle";

export type ResultModel = {
  kind: PanelKind;
  raw: string;
  /** Preserve mutation failures across background status polling. */
  sticky?: boolean;
};

type DiffAction = {
  action: string;
  kind: string;
  name: string;
  breaking?: boolean;
  reason?: string;
};

type StatusSection = {
  ok: boolean;
  exit_code?: number | null;
  data?: Record<string, unknown> | null;
  stderr?: string;
};

export function ResultPanel({
  result,
  busyLabel,
  stackPath,
  onJournalRecovered,
}: {
  result: ResultModel;
  busyLabel?: string | null;
  stackPath?: string | null;
  onJournalRecovered?: () => void;
}) {
  const t = useT();
  const [debug, setDebug] = useState(false);
  const parsed = useMemo(() => tryParse(result.raw), [result.raw]);


  if (result.kind === "idle" && !busyLabel) {
    return (
      <EmptyState card className="result-empty" message={t("result.empty")} />
    );
  }

  if (busyLabel) {
    return (
      <LoadingState card className="result-empty" message={busyLabel} />
    );
  }

  return (
    <div className={result.kind === "error" ? "result-panel is-error" : "result-panel"}>
      <div className="result-toolbar">
        <span className="result-label">{labelFor(result.kind, t)}</span>
        <Button
          tone="secondary"
          className={debug ? "debug-btn active" : "debug-btn"}
          onClick={() => setDebug((v) => !v)}
        >
          {t("result.debug")}
        </Button>
      </div>

      {debug ? (
        <DebugBlock text={result.raw} error={result.kind === "error"} />
      ) : result.kind === "error" ? (
        <Alert title={t("result.errorTitle")}>
          <p>{result.raw}</p>
          {stackPath ? (
            <ApplyRecoveryActions
              raw={result.raw}
              path={stackPath}
              onDone={onJournalRecovered}
            />
          ) : null}
        </Alert>
      ) : result.kind === "diff" && parsed ? (
        <DiffView data={parsed} />
      ) : result.kind === "status" && parsed ? (
        <StatusView data={parsed} />
      ) : parsed && Array.isArray(parsed.actions) ? (
        <DiffView data={parsed} />
      ) : parsed ? (
        <SummaryCard
          badge={<Badge tone="ok">{t("result.status.ok")}</Badge>}
          meta={
            parsed.command != null ? (
              <span className="path">{String(parsed.command)}</span>
            ) : null
          }
        >
          <Muted>
            {String(
              (parsed.summary as Record<string, unknown> | undefined)?.message ??
                t("result.summary.done"),
            )}
          </Muted>
        </SummaryCard>
      ) : (
        <EmptyState message={t("result.noGraphicView")} />
      )}
    </div>
  );
}

function DiffView({ data }: { data: Record<string, unknown> }) {
  const t = useT();
  const actions = (Array.isArray(data.actions) ? data.actions : []) as DiffAction[];
  const summary = (data.summary as Record<string, unknown> | undefined) ?? {};
  const changed = Boolean(summary.changed ?? actions.length);
  const message =
    summary.message != null
      ? String(summary.message)
      : changed
        ? t("result.diff.changes", { n: actions.length })
        : t("result.diff.equal");
  const stackId = data.stack_id != null ? String(data.stack_id) : null;

  const byKind = groupBy(actions, (a) => a.kind || "other");

  return (
    <div className="view-stack">
      <SummaryCard
        badge={
          <Badge tone={changed ? "warn" : "ok"}>
            {changed
              ? t("result.diff.changes", { n: actions.length })
              : t("result.diff.inSync")}
          </Badge>
        }
        meta={stackId ? <span className="path">{stackId}</span> : null}
      >
        <Muted>{message}</Muted>
      </SummaryCard>

      {actions.length === 0 ? (
        <EmptyState message={t("result.diff.equal")} />
      ) : (
        Object.entries(byKind).map(([kind, items]) => (
          <Card key={kind}>
            <h3 className="group-title">
              <span className="kind-pill">{kind}</span>
              <Muted as="span">{items.length}</Muted>
            </h3>
            <ul className="action-list">
              {items.map((item) => (
                <li key={`${item.kind}:${item.name}:${item.action}`} className="action-row">
                  <span className={`action-chip action-${item.action}`}>
                    {item.action}
                  </span>
                  <div className="action-body">
                    <strong>{item.name}</strong>
                    {item.reason ? <Muted as="span">{item.reason}</Muted> : null}
                  </div>
                  {item.breaking ? (
                    <Badge tone="danger">{t("result.status.breaking")}</Badge>
                  ) : null}
                </li>
              ))}
            </ul>
          </Card>
        ))
      )}
    </div>
  );
}

function StatusView({ data }: { data: Record<string, unknown> }) {
  const t = useT();
  const emDash = t("common.emDash");
  const sections = (data.sections as Record<string, StatusSection> | undefined) ?? {};
  const dns = sections.dns;
  const certs = sections.certs;
  const oidc = sections.oidc;
  const diff = sections.diff;
  const stackSection = sections.stack;
  const health = parseStackHealth(data);

  const dnsData = asRecord(dns?.data);
  const dnsInner = asRecord(dnsData?.dns) ?? dnsData;
  const certsData = asRecord(asRecord(certs?.data)?.data) ?? asRecord(certs?.data);
  const oidcData = asRecord(asRecord(oidc?.data)?.data) ?? asRecord(oidc?.data);
  const oidcDisabled = oidcData?.enabled === false || oidcData?.status === "disabled";
  const diffData = asRecord(diff?.data);
  const diffActions = (Array.isArray(diffData?.actions) ? diffData?.actions : []) as DiffAction[];
  const stackData = asRecord(stackSection?.data);
  const stackVms = asRecord(stackData?.vms);
  const dnsNeedsHelper = needsDnsBindHelper(
    dnsInner?.last_error != null ? String(dnsInner.last_error) : null,
  );

  const rows: StatusRowModel[] = [
    {
      title: t("result.status.stack"),
      ok: String(stackData?.phase ?? "") === "running" && health.state === "ok",
      statusLabel: t(HEALTH_LABEL_KEYS[health.state]),
      neutral: health.state === "stopped",
      facts: [
        stackData?.label != null ? String(stackData.label) : null,
        stackVms
          ? t("result.status.vms", {
              running: Number(stackVms.running ?? 0),
              desired: Number(stackVms.desired ?? 0),
            })
          : null,
        stackData?.stack_id != null ? String(stackData.stack_id) : null,
      ],
    },
    {
      title: t("result.status.dns"),
      ok: Boolean(dnsInner?.ok ?? dns?.ok),
      facts: [
        joinList(dnsInner?.listeners, emDash),
        dnsInner?.zones != null
          ? t("result.status.zones", { n: Number(dnsInner.zones) })
          : null,
        dnsInner?.records != null
          ? t("result.status.records", { n: Number(dnsInner.records) })
          : null,
        dnsInner?.upstream != null
          ? t("result.status.upstream", { upstream: String(dnsInner.upstream) })
          : null,
      ],
      error:
        dnsInner?.last_error != null
          ? String(dnsInner.last_error)
          : dns?.stderr || undefined,
      hint: dnsNeedsHelper ? t("result.status.dnsHint") : undefined,
      action: dnsNeedsHelper ? <DnsBindHelperButton /> : undefined,
    },
    {
      title: t("result.status.ca"),
      ok: Boolean(certs?.ok && certsData?.fingerprint && certsData?.trusted !== false),
      facts: [
        shortFp(
          certsData?.fingerprint != null ? String(certsData.fingerprint) : null,
          emDash,
        ),
        certsData?.trusted === true
          ? t("result.status.trusted")
          : certsData?.trusted === false
            ? t("result.status.notTrusted")
            : null,
        certsData?.path != null ? String(certsData.path) : null,
      ],
      hint: certsData?.trusted === false ? t("result.status.caHint") : undefined,
      action:
        certsData?.present === true && certsData?.trusted === false ? (
          <CaInstallButton />
        ) : undefined,
    },
    {
      title: t("result.status.oidc"),
      ok: oidcDisabled || Boolean(oidcData?.running),
      statusLabel: oidcDisabled ? t("result.status.disabled") : undefined,
      facts: [
        oidcDisabled ? t("result.status.disabled") : oidcData?.running
          ? t("result.status.running")
          : t("result.status.stopped"),
        oidcData?.pid != null
          ? t("result.status.pid", { pid: String(oidcData.pid) })
          : null,
        oidcData?.project != null ? String(oidcData.project) : null,
      ],
    },
    {
      title: t("result.status.drift"),
      ok: diffActions.length === 0 && Boolean(diff?.ok),
      facts: [
        t("result.status.actions", { n: diffActions.length }),
        diffData?.stack_id != null ? String(diffData.stack_id) : null,
      ],
    },
  ];

  return (
    <div className="view-stack">
      <StackHealthSummary health={health} />
      <Card className="status-board">
        <ul className="status-board-list">
          {rows.map((row) => (
            <StatusRow key={row.title} {...row} />
          ))}
        </ul>
      </Card>

      {diffActions.length > 0 ? (
        <Card title={t("result.status.openDiffActions")} titleAs="h3">
          <ul className="action-list compact">
            {diffActions.slice(0, 12).map((item) => (
              <li key={`${item.kind}:${item.name}:${item.action}`} className="action-row">
                <span className={`action-chip action-${item.action}`}>{item.action}</span>
                <div className="action-body">
                  <strong>
                    {item.kind}/{item.name}
                  </strong>
                </div>
              </li>
            ))}
          </ul>
          {diffActions.length > 12 ? (
            <Muted>
              {t("result.status.moreActions", { n: diffActions.length - 12 })}
            </Muted>
          ) : null}
        </Card>
      ) : null}
    </div>
  );
}

type StatusRowModel = {
  statusLabel?: string;
  neutral?: boolean;
  title: string;
  ok: boolean;
  facts: Array<string | null | undefined>;
  error?: string;
  action?: ReactNode;
  hint?: string;
};

function StatusRow({ title, ok, facts, error, action, hint, statusLabel, neutral }: StatusRowModel) {
  const t = useT();
  const emDash = t("common.emDash");
  const visibleFacts = facts.filter(
    (fact): fact is string =>
      typeof fact === "string" && fact.length > 0 && fact !== emDash,
  );

  return (
    <li className={`status-board-row${ok || neutral ? "" : " is-warn"}`}>
      <div className="status-board-main">
        <span className="status-board-title">{title}</span>
        <Badge tone={neutral ? "neutral" : ok ? "ok" : "warn"}>
          {statusLabel ?? (ok ? t("result.status.ok") : t("result.status.check"))}
        </Badge>
        <p className="status-board-facts" title={visibleFacts.join(" · ")}>
          {visibleFacts.length > 0 ? visibleFacts.join(" · ") : emDash}
        </p>
      </div>
      {error ? <p className="status-board-error">{error}</p> : null}
      {hint ? <Muted className="status-board-hint">{hint}</Muted> : null}
      {action ? <div className="status-board-action">{action}</div> : null}
    </li>
  );
}

function CaInstallButton() {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  return (
    <div className="doctor-actions">
      <Button
        disabled={busy}
        onClick={() => {
          setBusy(true);
          setMsg(null);
          void (async () => {
            try {
              const envelope = parseEnvelope(await api.post("/v1/certs/ca/install"));
              assertEnvelopeOk(envelope, t("doctor.caInstallFail"));
              setMsg(t("result.status.caInstallOk"));
            } catch (err) {
              setMsg(String(err));
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        {busy ? t("result.status.caInstallBusy") : t("result.status.caInstall")}
      </Button>
      {msg ? <Muted>{msg}</Muted> : null}
    </div>
  );
}

function needsDnsBindHelper(lastError: string | null | undefined): boolean {
  if (!lastError) return false;
  return (
    lastError.includes("Permission denied") ||
    lastError.includes("dns-bind") ||
    (lastError.includes("bind") && lastError.includes(":53"))
  );
}

function DnsBindHelperButton() {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  return (
    <div className="doctor-actions">
      <Button
        disabled={busy}
        onClick={() => {
          setBusy(true);
          setMsg(null);
          void (async () => {
            try {
              const envelope = parseEnvelope(await api.post("/v1/dns/bind-helper"));
              assertEnvelopeOk(envelope, t("doctor.bindInstallFail"));
              setMsg(t("result.status.bindInstallOk"));
            } catch (err) {
              setMsg(String(err));
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        {busy ? t("result.status.bindInstallBusy") : t("result.status.bindInstall")}
      </Button>
      {msg ? <Muted>{msg}</Muted> : null}
    </div>
  );
}

function DebugBlock({ text, error }: { text: string; error?: boolean }) {
  return <CodeBlock value={text} copyable tone={error ? "error" : "default"} />;
}

function labelFor(
  kind: PanelKind,
  t: ReturnType<typeof useT>,
): string {
  switch (kind) {
    case "diff":
      return t("result.label.diff");
    case "status":
      return t("result.label.status");
    case "error":
      return t("result.label.error");
    case "text":
      return t("result.label.text");
    default:
      return t("result.label.idle");
  }
}

function tryParse(raw: string): Record<string, unknown> | null {
  try {
    const value = JSON.parse(raw) as unknown;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      return value as Record<string, unknown>;
    }
  } catch {
    // ignore
  }
  return null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  return null;
}

function groupBy<T>(items: T[], key: (item: T) => string): Record<string, T[]> {
  const out: Record<string, T[]> = {};
  for (const item of items) {
    const k = key(item);
    (out[k] ??= []).push(item);
  }
  return out;
}

function joinList(value: unknown, emDash: string): string {
  if (Array.isArray(value)) return value.map(String).join(", ") || emDash;
  if (value == null) return emDash;
  return String(value);
}

function shortFp(fp: string | null, emDash: string): string {
  if (!fp) return emDash;
  return fp.length > 16 ? `${fp.slice(0, 10)}…${fp.slice(-6)}` : fp;
}
