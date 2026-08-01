import { useMemo, useState, type ReactNode } from "react";
import {
  assertEnvelopeOk,
  parseEnvelope,
  runVzctlArgv,
} from "@/lib/vzctl";

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
}: {
  result: ResultModel;
  busyLabel?: string | null;
}) {
  const [debug, setDebug] = useState(false);
  const parsed = useMemo(() => tryParse(result.raw), [result.raw]);

  if (result.kind === "idle" && !busyLabel) {
    return (
      <div className="card result-empty">
        <p className="muted">Diff oder Status laden — JSON nur über Debug.</p>
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
        <span className="result-label">{labelFor(result.kind)}</span>
        <button
          type="button"
          className={debug ? "debug-btn active" : "debug-btn"}
          onClick={() => setDebug((v) => !v)}
        >
          Debug
        </button>
      </div>

      {debug ? (
        <DebugBlock text={result.raw} error={result.kind === "error"} />
      ) : result.kind === "error" ? (
        <div className="card error-card">
          <h3>Fehler</h3>
          <p>{result.raw}</p>
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
            <span className="badge ok">ok</span>
            {parsed.command != null ? (
              <span className="path">{String(parsed.command)}</span>
            ) : null}
          </div>
          <p className="muted">
            {String(
              (parsed.summary as Record<string, unknown> | undefined)?.message ??
                "Fertig.",
            )}
          </p>
        </div>
      ) : (
        <div className="card">
          <p className="muted">Keine grafische Darstellung — Debug öffnen.</p>
        </div>
      )}
    </div>
  );
}

function DiffView({ data }: { data: Record<string, unknown> }) {
  const actions = (Array.isArray(data.actions) ? data.actions : []) as DiffAction[];
  const summary = (data.summary as Record<string, unknown> | undefined) ?? {};
  const changed = Boolean(summary.changed ?? actions.length);
  const message = String(summary.message ?? (changed ? "changes planned" : "no changes"));
  const stackId = data.stack_id != null ? String(data.stack_id) : null;

  const byKind = groupBy(actions, (a) => a.kind || "other");

  return (
    <div className="view-stack">
      <div className="card summary-card">
        <div className="summary-row">
          <span className={changed ? "badge warn" : "badge ok"}>
            {changed ? `${actions.length} Änderungen` : "In Sync"}
          </span>
          {stackId ? <span className="path">{stackId}</span> : null}
        </div>
        <p className="muted">{message}</p>
      </div>

      {actions.length === 0 ? (
        <div className="card">
          <p className="muted">Desired und Actual sind gleich.</p>
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
                  {item.breaking ? <span className="badge danger">breaking</span> : null}
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

  return (
    <div className="view-stack">
      <div className="status-grid">
        <StatusTile
          title="Stack"
          ok={String(stackData?.phase ?? "") === "running"}
          rows={[
            ["Phase", stackData?.label != null ? String(stackData.label) : "—"],
            [
              "VMs",
              stackVms
                ? `${stackVms.running ?? 0}/${stackVms.desired ?? 0} running`
                : "—",
            ],
            [
              "Stack",
              stackData?.stack_id != null ? String(stackData.stack_id) : "—",
            ],
          ]}
        />
        <StatusTile
          title="DNS"
          ok={Boolean(dnsInner?.ok ?? dns?.ok)}
          rows={[
            ["Listeners", joinList(dnsInner?.listeners)],
            ["Zones", String(dnsInner?.zones ?? "—")],
            ["Records", String(dnsInner?.records ?? "—")],
            ["Upstream", String(dnsInner?.upstream ?? "—")],
          ]}
          error={
            dnsInner?.last_error != null
              ? String(dnsInner.last_error)
              : dns?.stderr || undefined
          }
          action={
            needsDnsBindHelper(
              dnsInner?.last_error != null ? String(dnsInner.last_error) : null,
            ) ? (
              <DnsBindHelperButton />
            ) : undefined
          }
          hint={
            needsDnsBindHelper(
              dnsInner?.last_error != null ? String(dnsInner.last_error) : null,
            )
              ? "Guest-:53 braucht den DNS-Bind-Helper (Admin)."
              : undefined
          }
        />
        <StatusTile
          title="CA"
          ok={Boolean(certs?.ok && certsData?.fingerprint && certsData?.trusted !== false)}
          rows={[
            [
              "Fingerprint",
              shortFp(certsData?.fingerprint != null ? String(certsData.fingerprint) : null),
            ],
            [
              "Keychain",
              certsData?.trusted === true
                ? "trusted"
                : certsData?.trusted === false
                  ? "nicht trusted"
                  : "—",
            ],
            ["Path", certsData?.path != null ? String(certsData.path) : "—"],
          ]}
          action={
            certsData?.present === true && certsData?.trusted === false ? (
              <CaInstallButton />
            ) : undefined
          }
          hint={
            certsData?.trusted === false
              ? "Browser melden SEC_ERROR_UNKNOWN_ISSUER, bis die CA in der Keychain liegt. Firefox/Zen: enterprise_roots oder manueller Import."
              : undefined
          }
        />
        <StatusTile
          title="OIDC"
          ok={Boolean(oidcData?.running)}
          rows={[
            ["Running", oidcData?.running ? "yes" : "no"],
            ["PID", oidcData?.pid != null ? String(oidcData.pid) : "—"],
            ["Project", oidcData?.project != null ? String(oidcData.project) : "—"],
          ]}
        />
        <StatusTile
          title="Drift"
          ok={diffActions.length === 0 && Boolean(diff?.ok)}
          rows={[
            ["Actions", String(diffActions.length)],
            [
              "Stack",
              diffData?.stack_id != null ? String(diffData.stack_id) : "—",
            ],
          ]}
        />
      </div>

      {diffActions.length > 0 ? (
        <div className="card">
          <h3 className="group-title">Offene Diff-Actions</h3>
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
            <p className="muted">+{diffActions.length - 12} weitere…</p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function StatusTile({
  title,
  ok,
  rows,
  error,
  action,
  hint,
}: {
  title: string;
  ok: boolean;
  rows: Array<[string, string]>;
  error?: string;
  action?: ReactNode;
  hint?: string;
}) {
  return (
    <div className="card status-tile">
      <div className="summary-row">
        <h3>{title}</h3>
        <span className={ok ? "badge ok" : "badge warn"}>{ok ? "ok" : "check"}</span>
      </div>
      <dl className="kv">
        {rows.map(([k, v]) => (
          <div key={k} className="kv-row">
            <dt>{k}</dt>
            <dd title={v}>{v}</dd>
          </div>
        ))}
      </dl>
      {error ? <p className="tile-error">{error}</p> : null}
      {hint ? <p className="muted tile-hint">{hint}</p> : null}
      {action ? <div className="tile-action">{action}</div> : null}
    </div>
  );
}

function CaInstallButton() {
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
              const raw = await runVzctlArgv([
                "certs",
                "ca",
                "install",
                "--format",
                "json",
              ]);
              const envelope = parseEnvelope(raw);
              assertEnvelopeOk(envelope, "CA-Installation fehlgeschlagen");
              setMsg("Installiert — Status neu laden.");
            } catch (err) {
              setMsg(String(err));
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        {busy ? "Installiert…" : "CA in Keychain installieren"}
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
              const raw = await runVzctlArgv([
                "dns",
                "install-bind-helper",
                "--format",
                "json",
              ]);
              const envelope = parseEnvelope(raw);
              assertEnvelopeOk(
                envelope,
                "DNS-Bind-Helper-Installation fehlgeschlagen",
              );
              setMsg("Installiert — Status neu laden.");
            } catch (err) {
              setMsg(String(err));
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        {busy ? "Installiert…" : "Bind-Helper installieren"}
      </button>
      {msg ? <p className="muted">{msg}</p> : null}
    </div>
  );
}

function DebugBlock({ text, error }: { text: string; error?: boolean }) {
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
        {copied ? "Kopiert" : "Kopieren"}
      </button>
      <pre className={error ? "out error" : "out"}>{text}</pre>
    </div>
  );
}

function labelFor(kind: PanelKind): string {
  switch (kind) {
    case "diff":
      return "Diff";
    case "status":
      return "Status";
    case "error":
      return "Fehler";
    case "text":
      return "Ausgabe";
    default:
      return "Ergebnis";
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

function joinList(value: unknown): string {
  if (Array.isArray(value)) return value.map(String).join(", ") || "—";
  if (value == null) return "—";
  return String(value);
}

function shortFp(fp: string | null): string {
  if (!fp) return "—";
  return fp.length > 16 ? `${fp.slice(0, 10)}…${fp.slice(-6)}` : fp;
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const area = document.createElement("textarea");
      area.value = text;
      area.setAttribute("readonly", "");
      area.style.position = "fixed";
      area.style.left = "-9999px";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(area);
      return ok;
    } catch {
      return false;
    }
  }
}
