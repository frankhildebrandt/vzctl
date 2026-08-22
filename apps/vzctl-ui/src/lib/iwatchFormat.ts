import type { IwatchLine } from "@/lib/guestLogs";

export type HiddenFields = Record<string, true>;

export type FieldPair = {
  key: string;
  value: string;
};

export type VisibleColumns = {
  ts: string;
  source: string | null;
  body: string;
  levelClass: string;
};

/** Map a log level string to a CSS class name. */
export function levelClass(level?: string): string {
  const value = (level ?? "").toLowerCase();
  if (value.includes("err") || value.includes("fatal") || value.includes("panic")) {
    return "error";
  }
  if (value.includes("warn")) return "warn";
  if (value.includes("info")) return "info";
  if (value.includes("debug") || value.includes("trace")) return "debug";
  return "";
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function formatLogfmtValue(value: string): string {
  if (value === "" || /[\s"=]/.test(value)) return JSON.stringify(value);
  return value;
}

function structuredHiddenKeys(hiddenFields: HiddenFields): string[] {
  return Object.keys(hiddenFields).filter(
    (key) => key !== "raw" && key !== "source",
  );
}

function isEmptyObject(value: unknown): boolean {
  return (
    Boolean(value) &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.keys(value as object).length === 0
  );
}

function omitHiddenJSON(
  value: unknown,
  prefix: string,
  hidden: Set<string>,
): unknown {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return value;
  }
  const out: Record<string, unknown> = {};
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key.toLowerCase()}` : key.toLowerCase();
    if (hidden.has(path) || hidden.has(key.toLowerCase())) continue;
    const next = omitHiddenJSON(child, path, hidden);
    if (isEmptyObject(next)) continue;
    out[key] = next;
  }
  return out;
}

function stripHiddenFields(text: string, hiddenKeys: string[]): string {
  const hidden = new Set(hiddenKeys.map((key) => key.toLowerCase()));
  const trimmed = text.trim();
  if (trimmed.startsWith("{") && trimmed.endsWith("}")) {
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      return JSON.stringify(omitHiddenJSON(parsed, "", hidden));
    } catch {
      // Fall through to logfmt stripping for invalid JSON objects.
    }
  }
  return text
    .replace(
      /(^|\s)([A-Za-z0-9_.-]+)=(?:"(?:\\.|[^"\\])*"|[^\s)]+)/g,
      (match, prefix: string, key: string) =>
        hidden.has(key.toLowerCase()) ? prefix : match,
    )
    .replace(/[ \t]{2,}/g, " ");
}

/** Return visible structured field pairs for a log line. */
export function visibleFieldPairs(
  line: IwatchLine,
  observedFields: string[],
  hiddenFields: HiddenFields,
): FieldPair[] {
  const fields = line.fields ?? {};
  const parts: FieldPair[] = [];
  for (const key of observedFields) {
    if (hiddenFields[key] || fields[key] == null || fields[key] === "") continue;
    parts.push({ key, value: fields[key] });
  }
  return parts;
}

/** Format the log body HTML for display. */
export function formatBodyHtml(
  line: IwatchLine,
  observedFields: string[],
  hiddenFields: HiddenFields,
): string {
  const pairs = visibleFieldPairs(line, observedFields, hiddenFields);
  if (pairs.length > 0 && hiddenFields.raw) {
    return pairs
      .map((pair) => {
        const key = escapeHtml(pair.key);
        const value = escapeHtml(formatLogfmtValue(pair.value));
        return `<span class="vm-logs-k">${key}</span>=<span class="vm-logs-v">${value}</span>`;
      })
      .join(" ");
  }
  const text = line.text ?? "";
  if (!hiddenFields.raw) {
    const hiddenKeys = structuredHiddenKeys(hiddenFields);
    if (hiddenKeys.length === 0) return escapeHtml(text);
    return escapeHtml(stripHiddenFields(text, hiddenKeys));
  }
  return escapeHtml(text);
}

/** Build visible column values for a log line row. */
export function visibleColumns(
  line: IwatchLine,
  observedFields: string[],
  hiddenFields: HiddenFields,
): VisibleColumns {
  const hidden = hiddenFields;
  return {
    ts: (line.ts ?? "").slice(11, 19),
    source: hidden.source ? null : (line.source ?? ""),
    body: formatBodyHtml(line, observedFields, hiddenFields),
    levelClass: levelClass(line.level),
  };
}
