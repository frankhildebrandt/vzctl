import { useEffect, useMemo, useRef, useState } from "react";
import {
  APPLY_STEPS,
  DOWN_STEPS,
  subscribeEvents,
  type VzctlEvent,
} from "@/lib/vzctl";

type StepStatus = "pending" | "running" | "done" | "failed";

export type ConsoleLine = {
  id: number;
  ts: string;
  level: "info" | "ok" | "warn" | "error" | "cmd";
  text: string;
};

type ProgressState = {
  active: boolean;
  mode: string | null;
  invocationId: string | null;
  steps: Record<string, StepStatus>;
  error: string | null;
  finished: boolean;
  lines: ConsoleLine[];
  nextId: number;
  lastVmState: Record<string, string>;
};

const idle: ProgressState = {
  active: false,
  mode: null,
  invocationId: null,
  steps: {},
  error: null,
  finished: false,
  lines: [],
  nextId: 1,
  lastVmState: {},
};

function nowStamp(): string {
  return new Date().toLocaleTimeString("de-DE", { hour12: false });
}

function append(
  prev: ProgressState,
  level: ConsoleLine["level"],
  text: string,
  ts?: string,
): ProgressState {
  const line: ConsoleLine = {
    id: prev.nextId,
    ts: ts ?? nowStamp(),
    level,
    text,
  };
  const lines = [...prev.lines, line];
  // Keep console bounded.
  const trimmed = lines.length > 500 ? lines.slice(lines.length - 500) : lines;
  return { ...prev, lines: trimmed, nextId: prev.nextId + 1 };
}

export function useApplyProgress(enabled: boolean) {
  const [state, setState] = useState<ProgressState>(idle);

  useEffect(() => {
    if (!enabled) return;
    let unlistenEvent: (() => void) | undefined;
    let unlistenConsole: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      try {
        await subscribeEvents();
        const { listen } = await import("@tauri-apps/api/event");
        unlistenEvent = await listen<VzctlEvent>("vzctl-event", (event) => {
          if (cancelled) return;
          setState((prev) => reduceEvent(prev, event.payload));
        });
        unlistenConsole = await listen<{
          stream?: string;
          line?: string;
          text?: string;
        }>("vzctl-console", (event) => {
          if (cancelled) return;
          const payload = event.payload;
          const text = String(payload.line ?? payload.text ?? "").trimEnd();
          if (!text) return;
          const stream = String(payload.stream ?? "out");
          const level: ConsoleLine["level"] =
            stream === "err"
              ? "warn"
              : stream === "cmd"
                ? "cmd"
                : stream === "fail"
                  ? "error"
                  : "info";
          setState((prev) => {
            if (!prev.active && !prev.finished) return prev;
            return append(prev, level, text);
          });
        });
      } catch {
        // Supervisor may be down; deploy can still run without live events.
      }
    })();

    return () => {
      cancelled = true;
      unlistenEvent?.();
      unlistenConsole?.();
    };
  }, [enabled]);

  function begin(mode: string) {
    const catalog = mode === "down" ? DOWN_STEPS : APPLY_STEPS;
    const steps = Object.fromEntries(
      catalog.map((step) => [step, "pending" as StepStatus]),
    );
    let next: ProgressState = {
      active: true,
      mode,
      invocationId: null,
      steps,
      error: null,
      finished: false,
      lines: [],
      nextId: 1,
      lastVmState: {},
    };
    next = append(next, "cmd", `$ vzctl ${mode}`);
    next = append(next, "info", "warte auf apply-Events…");
    setState(next);
  }

  function end(ok = true) {
    setState((prev) => {
      if (!prev.active && prev.finished) return prev;
      let next = { ...prev, active: false, finished: true };
      if (ok && !prev.error) {
        next = append(next, "ok", `${prev.mode ?? "apply"} fertig`);
      } else if (!ok && !prev.error) {
        next = append(next, "error", `${prev.mode ?? "apply"} fehlgeschlagen`);
      }
      return next;
    });
  }

  function reset() {
    setState(idle);
  }

  const ordered = useMemo(() => {
    const catalog =
      state.mode === "down" ? [...DOWN_STEPS] : [...APPLY_STEPS];
    return catalog.map((name) => ({
      name,
      status: state.steps[name] ?? ("pending" as StepStatus),
    }));
  }, [state.mode, state.steps]);

  const doneCount = ordered.filter((s) => s.status === "done").length;
  const total = ordered.length || 1;
  const percent = Math.round((doneCount / total) * 100);
  const current =
    ordered.find((s) => s.status === "running")?.name ??
    ordered.find((s) => s.status === "failed")?.name ??
    null;

  return { state, ordered, percent, current, begin, end, reset };
}

function reduceEvent(prev: ProgressState, event: VzctlEvent): ProgressState {
  const type = event.type;
  const data = event.data ?? {};
  const ts = event.ts
    ? new Date(event.ts).toLocaleTimeString("de-DE", { hour12: false })
    : nowStamp();

  // Only mirror live apply into the UI while a local up/apply/down is running.
  // Otherwise vm.state spam replaces the project dashboard with a stuck log.
  if (!prev.active) return prev;

  if (type === "apply.started") {
    const mode = String(data.mode ?? prev.mode ?? "apply");
    const catalog = mode === "down" ? DOWN_STEPS : APPLY_STEPS;
    let next: ProgressState = {
      active: true,
      mode,
      invocationId: String(data.invocation_id ?? ""),
      steps: Object.fromEntries(
        catalog.map((step) => [step, "pending" as StepStatus]),
      ),
      error: null,
      finished: false,
      lines: prev.lines,
      nextId: prev.nextId,
      lastVmState: prev.lastVmState,
    };
    return append(
      next,
      "info",
      `apply.started mode=${mode} id=${data.invocation_id ?? "—"}`,
      ts,
    );
  }

  if (type === "apply.step") {
    const step = String(data.step ?? "");
    const statusRaw = String(data.status ?? "running");
    const status: StepStatus =
      statusRaw === "done"
        ? "done"
        : statusRaw === "failed"
          ? "failed"
          : "running";
    if (!step) return prev;
    const steps = { ...prev.steps };
    const catalog: readonly string[] =
      prev.mode === "down" ? DOWN_STEPS : APPLY_STEPS;
    const idx = catalog.indexOf(step);
    if (idx > 0) {
      for (let i = 0; i < idx; i += 1) {
        const name = catalog[i];
        if (steps[name] === "pending" || steps[name] === "running") {
          steps[name] = "done";
        }
      }
    }
    steps[step] = status;
    const mark =
      status === "done" ? "✓" : status === "failed" ? "✗" : "→";
    const level: ConsoleLine["level"] =
      status === "done" ? "ok" : status === "failed" ? "error" : "info";
    const detail =
      status === "failed" && data.error
        ? `  ${String(data.error)}`
        : status === "running"
          ? " …"
          : "";
    let next: ProgressState = {
      ...prev,
      active: true,
      invocationId:
        data.invocation_id != null
          ? String(data.invocation_id)
          : prev.invocationId,
      steps,
      error:
        status === "failed"
          ? String(data.error ?? "step failed")
          : prev.error,
    };
    return append(next, level, `${mark} ${step}${detail}`, ts);
  }

  if (type === "apply.finished") {
    const steps = { ...prev.steps };
    for (const key of Object.keys(steps)) {
      if (steps[key] === "pending" || steps[key] === "running") {
        steps[key] = "done";
      }
    }
    let next: ProgressState = {
      ...prev,
      active: true,
      steps,
      finished: true,
      error: null,
    };
    return append(next, "ok", "apply.finished", ts);
  }

  if (type === "apply.failed") {
    const step = data.step != null ? String(data.step) : null;
    const steps = { ...prev.steps };
    if (step) steps[step] = "failed";
    const err = String(data.error ?? "apply failed");
    let next: ProgressState = {
      ...prev,
      active: true,
      steps,
      finished: true,
      error: err,
    };
    return append(
      next,
      "error",
      `✗ apply.failed${step ? ` @ ${step}` : ""}: ${err}`,
      ts,
    );
  }

  if (type === "vm.state") {
    const vm = String(data.vm_id ?? data.name ?? "?");
    const vmState = String(data.state ?? "?");
    if (prev.lastVmState[vm] === vmState) return prev;
    const next = {
      ...prev,
      lastVmState: { ...prev.lastVmState, [vm]: vmState },
    };
    return append(next, "info", `vm ${vm} → ${vmState}`, ts);
  }

  return prev;
}

export function ApplyProgress({
  ordered,
  percent,
  mode,
  error,
  visible,
}: {
  ordered: Array<{ name: string; status: StepStatus }>;
  percent: number;
  mode: string | null;
  error: string | null;
  visible: boolean;
}) {
  if (!visible) return null;

  return (
    <div className="card progress-card">
      <div className="progress-head">
        <h2>{mode ? `${mode}` : "apply"} — Fortschritt</h2>
        <span className="muted">{percent}%</span>
      </div>
      <div className="progress-bar" aria-hidden>
        <div className="progress-fill" style={{ width: `${percent}%` }} />
      </div>
      <ol className="progress-steps">
        {ordered.map((step) => (
          <li key={step.name} className={`progress-step ${step.status}`}>
            <span className="progress-dot" aria-hidden />
            <span className="progress-name">{step.name}</span>
            <span className="progress-status">{label(step.status)}</span>
          </li>
        ))}
      </ol>
      {error ? <p className="progress-error">{error}</p> : null}
    </div>
  );
}

export function ConsoleLog({
  lines,
  visible,
  title = "Log",
  onDismiss,
}: {
  lines: ConsoleLine[];
  visible: boolean;
  title?: string;
  onDismiss?: () => void;
}) {
  const scroller = useRef<HTMLPreElement>(null);
  const stickRef = useRef(true);

  useEffect(() => {
    const el = scroller.current;
    if (!el || !stickRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [lines]);

  if (!visible) return null;

  return (
    <div className="card console-card">
      <div className="progress-head">
        <h2>{title}</h2>
        <div className="console-head-actions">
          <span className="muted">{lines.length} Zeilen</span>
          {onDismiss ? (
            <button type="button" className="debug-btn" onClick={onDismiss}>
              Schließen
            </button>
          ) : null}
        </div>
      </div>
      <pre
        ref={scroller}
        className="text-console"
        aria-live="polite"
        onScroll={(event) => {
          const el = event.currentTarget;
          stickRef.current =
            el.scrollHeight - el.scrollTop - el.clientHeight < 48;
        }}
      >
        {lines.length === 0 ? (
          <span className="console-line info">warte auf Output…</span>
        ) : (
          lines.map((line) => (
            <span key={line.id} className={`console-line ${line.level}`}>
              <span className="console-ts">{line.ts}</span>
              <span className="console-text">{line.text}</span>
            </span>
          ))
        )}
      </pre>
    </div>
  );
}

function label(status: StepStatus): string {
  switch (status) {
    case "running":
      return "läuft";
    case "done":
      return "ok";
    case "failed":
      return "fehler";
    default:
      return "";
  }
}
