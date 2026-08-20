/** Parse a trailing `12%` meter from a job-log line. */
export function parseProgressLine(
  line: string,
): { percent: number; label: string } | null {
  const match = /^(.*?)(\d{1,3})%\s*$/.exec(line.trimEnd());
  if (!match) return null;
  const percent = Number(match[2]);
  if (!Number.isFinite(percent) || percent > 100) return null;
  return { percent, label: match[1].trim() };
}

export type JobProgress = {
  percent: number;
  label: string;
};

export function progressFromJob(job: {
  progress?: { percent?: unknown; label?: unknown } | null;
  log?: string[];
}): JobProgress | null {
  const raw = job.progress;
  if (raw && typeof raw.percent === "number" && raw.percent >= 0) {
    return {
      percent: Math.min(100, Math.round(raw.percent)),
      label: typeof raw.label === "string" ? raw.label : "",
    };
  }
  const log = job.log ?? [];
  for (let i = log.length - 1; i >= 0; i -= 1) {
    const parsed = parseProgressLine(log[i] ?? "");
    if (parsed) return parsed;
  }
  return null;
}

/** Append log lines, replacing a trailing progress meter instead of growing. */
export function mergeProgressLog(
  existing: { id: number; ts: string; level: "info" | "ok" | "warn" | "error" | "cmd"; text: string }[],
  incoming: string[],
  makeLine: (text: string) => {
    id: number;
    ts: string;
    level: "info" | "ok" | "warn" | "error" | "cmd";
    text: string;
  },
): typeof existing {
  if (incoming.length === 0) return existing;
  const next = [...existing];
  for (const text of incoming) {
    if (!text) continue;
    const progress = parseProgressLine(text);
    const last = next[next.length - 1];
    if (progress && last && parseProgressLine(last.text)) {
      next[next.length - 1] = { ...last, text };
    } else {
      next.push(makeLine(text));
    }
  }
  return next;
}
