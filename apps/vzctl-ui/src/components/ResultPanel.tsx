import { useMemo, useState, type ReactNode } from "react";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { useT } from "@/lib/i18n";
import { assertEnvelopeOk, parseEnvelope } from "@/lib/vzctl";

export type PanelKind = "diff" | "status" | "text" | "error" | "idle";

export type ResultModel = {
  kind: PanelKind;
  raw: string;
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
  const incompleteJournal =
    result.kind === "error" && isIncompleteJournalError(result.raw);

  if (result.kind === "idle" && !busyLabel) {
    return (
      <div className="card result-empty">
        <p className="muted">{t("result.empty")}</p>
      </div>
    );
  }

  if (busyLabel) {
    return (
      <div className="card result-empty">
        <p className="muted">{busyLabel}</p>
      </div>
    );
  }

  return (
    <div className={result.kind === "error" ? "result-panel is-error" : "result-panel"}>
      <div className="result-toolbar">
        <span className="result-label">{labelFor(result.kind, t)}</span>
        <button
          type="button"
          className={debug ? "debug-btn active" : "debug-btn"}
          onClick={() => setDebug((v) => !v)}
        >
          {t("result.debug")}
        </button>
      </div>

      {debug ? (
        <DebugBlock text={result.raw} error={result.kind === "error"} />
      ) : result.kind === "error" ? (
        <div className="card error-card">
          <h3>{t("result.errorTitle")}</h3>
          <p>{result.raw}</p>
          {incompleteJournal && stackPath ? (
            <IncompleteJournalActions
              path={stackPath}
              onDone={onJournalRecovered}
            />
          ) : null}
        </div>
      ) : result.kind === "diff" && parsed ? (
        <DiffView data={parsed} />
      ) : result.kind === "status" && parsed ? (
        <StatusView data={parsed} />
      ) : parsed && Array.isArray(parsed.actions) ? (
        <DiffView data={parsed} />
      ) : parsed ? (
        <div className="card summary-card">
          <div className="summary-row">
            <span className="badge ok">{t("result.status.ok")}</span>
            {parsed.command != null ? (
              <span className="path">{String(parsed.command)}</span>
            ) : null}
          </div>
          <p className="muted">
            {String(
              (parsed.summary as Record<string, unknown> | undefined)?.message ??
                t("result.summary.done"),
            )}
          </p>
        </div>
      ) : (
        <div className="card">
          <p className="muted">{t("result.noGraphicView")}</p>
        </div>
      )}
    </div>
  );
}

function isIncompleteJournalError(raw: string): boolean {
  return (
    raw.includes("incomplete journal") ||
    raw.includes("--resume") ||
    raw.includes("--abort")
  );
}

function IncompleteJournalActions({
  path,
  onDone,
}: {
  path: string;
  onDone?: () => void;
}) {
  const t = useT();
  const [busy, setBusy] = useState<"restart" | "resume" | "abort" | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  async function run(mode: "restart" | "resume" | "abort") {
    setBusy(mode);
    setMsg(null);
    try {
      const { runVzctl } = await import("@/lib/vzctl");
      if (mode === "restart") {
        await runVzctl(path, "apply", { abort: true });
        await runVzctl(path, "apply", { force: true });
        setMsg(t("result.journalRestartMsg"));
      } else if (mode === "resume") {
        await runVzctl(path, "apply", { resume: true });
        setMsg(t("result.journalResumeMsg"));
      } else {
        await runVzctl(path, "apply", { abort: true });
        setMsg(t("result.journalAbortMsg"));
      }
      onDone?.();
    } catch (err) {
      setMsg(String(err));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="doctor-actions" style={{ marginTop: "0.75rem" }}>
      <p className="muted">{t("result.incompleteJournalHint")}</p>
      <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
        <button
          type="button"
          disabled={busy != null}
          onClick={() => void run("restart")}
        >
          {busy === "restart" ? t("result.restartBusy") : t("result.restart")}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy != null}
          onClick={() => void run("resume")}
        >
          {busy === "resume" ? t("result.resumeBusy") : t("result.resume")}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy != null}
          onClick={() => void run("abort")}
        >
          {busy === "abort" ? t("result.abortBusy") : t("result.abort")}
        </button>
      </div>
      {msg ? <p className="muted">{msg}</p> : null}
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
      <div className="card summary-card">
        <div className="summary-row">
          <span className={changed ? "badge warn" : "badge ok"}>
            {changed
              ? t("result.diff.changes", { n: actions.length })
              : t("result.diff.inSync")}
          </span>
          {stackId ? <span className="path">{stackId}</span> : null}
        </div>
        <p className="muted">{message}</p>
      </div>

      {actions.length === 0 ? (
        <div className="card">
          <p className="muted">{t("result.diff.equal")}</p>
        </div>
      ) : (
        Object.entries(byKind).map(([kind, items]) => (
          <div key={kind} className="card">
            <h3 className="group-title">
              <span className="kind-pill">{kind}</span>
              <span className="muted">{items.length}</span>
            </h3>
            <ul className="action-list">
              {items.map((item) => (
                <li key={`${item.kind}:${item.name}:${item.action}`} className="action-row">
                  <span className={`action-chip action-${item.action}`}>
                    {item.action}
                  </span>
                  <div className="action-body">
                    <strong>{item.name}</strong>
                    {item.reason ? <span className="muted">{item.reason}</span> : null}
                  </div>
                  {item.breaking ? (
                    <span className="badge danger">{t("result.status.breaking")}</span>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
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

  const dnsData = asRecord(dns?.data);
  const dnsInner = asRecord(dnsData?.dns) ?? dnsData;
  const certsData = asRecord(asRecord(certs?.data)?.data) ?? asRecord(certs?.data);
  const oidcData = asRecord(asRecord(oidc?.data)?.data) ?? asRecord(oidc?.data);
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
      ok: String(stackData?.phase ?? "") === "running",
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
      ok: Boolean(oidcData?.running),
      facts: [
        oidcData?.running
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
      <div className="card status-board">
        <ul className="status-board-list">
          {rows.map((row) => (
            <StatusRow key={row.title} {...row} />
          ))}
        </ul>
      </div>

      {diffActions.length > 0 ? (
        <div className="card">
          <h3 className="group-title">{t("result.status.openDiffActions")}</h3>
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
            <p className="muted">
              {t("result.status.moreActions", { n: diffActions.length - 12 })}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

type StatusRowModel = {
  title: string;
  ok: boolean;
  facts: Array<string | null | undefined>;
  error?: string;
  action?: ReactNode;
  hint?: string;
};

function StatusRow({ title, ok, facts, error, action, hint }: StatusRowModel) {
  const t = useT();
  const emDash = t("common.emDash");
  const visibleFacts = facts.filter(
    (fact): fact is string =>
      typeof fact === "string" && fact.length > 0 && fact !== emDash,
  );

  return (
    <li className={`status-board-row${ok ? "" : " is-warn"}`}>
      <div className="status-board-main">
        <span className="status-board-title">{title}</span>
        <span className={ok ? "badge ok" : "badge warn"}>
          {ok ? t("result.status.ok") : t("result.status.check")}
        </span>
        <p className="status-board-facts" title={visibleFacts.join(" · ")}>
          {visibleFacts.length > 0 ? visibleFacts.join(" · ") : emDash}
        </p>
      </div>
      {error ? <p className="status-board-error">{error}</p> : null}
      {hint ? <p className="muted status-board-hint">{hint}</p> : null}
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
      <button
        type="button"
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
      </button>
      {msg ? <p className="muted">{msg}</p> : null}
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
      <button
        type="button"
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
      </button>
      {msg ? <p className="muted">{msg}</p> : null}
    </div>
  );
}

function DebugBlock({ text, error }: { text: string; error?: boolean }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  return (
    <div className={error ? "out-wrap error" : "out-wrap"}>
      <button
        type="button"
        className="out-copy"
        onClick={() => {
          void copyText(text).then((ok) => {
            if (!ok) return;
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          });
        }}
      >
        {copied ? t("common.copied") : t("common.copy")}
      </button>
      <pre className={error ? "out error" : "out"}>{text}</pre>
    </div>
  );
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
