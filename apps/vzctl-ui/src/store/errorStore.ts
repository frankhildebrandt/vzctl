import { create } from "zustand";
import { ApiError } from "@/lib/api";
import { getT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";

export type ErrorSource = "query" | "mutation" | "api" | "ui";

export type ReportedError = {
  id: string;
  ts: number;
  message: string;
  source: ErrorSource;
  status?: number;
  code?: string;
  details?: unknown;
  method?: string;
  path?: string;
  route?: string;
  queryKey?: string;
  mutationKey?: string;
  stack?: string;
};

export type ReportErrorMeta = {
  source?: ErrorSource;
  method?: string;
  path?: string;
  queryKey?: unknown;
  mutationKey?: unknown;
};

const MAX_ERRORS = 100;
const DEDUPE_MS = 2000;

type ErrorStore = {
  errors: ReportedError[];
  report: (err: unknown, meta?: ReportErrorMeta) => ReportedError | null;
  clear: () => void;
};

let seq = 0;

function nextId(): string {
  seq += 1;
  return `err-${Date.now()}-${seq}`;
}

function keyString(key: unknown): string | undefined {
  if (key == null) return undefined;
  try {
    return JSON.stringify(key);
  } catch {
    return String(key);
  }
}

function currentRoute(): string | undefined {
  if (typeof window === "undefined") return undefined;
  return `${window.location.pathname}${window.location.search}`;
}

function buildEntry(err: unknown, meta?: ReportErrorMeta): ReportedError {
  const source = meta?.source ?? "ui";
  const route = currentRoute();
  const queryKey = keyString(meta?.queryKey);
  const mutationKey = keyString(meta?.mutationKey);

  if (err instanceof ApiError) {
    return {
      id: nextId(),
      ts: Date.now(),
      message: err.message,
      source,
      status: err.status,
      code: err.code,
      details: err.details,
      method: meta?.method ?? err.request?.method,
      path: meta?.path ?? err.request?.path,
      route,
      queryKey,
      mutationKey,
      stack: err.stack,
    };
  }

  if (err instanceof Error) {
    return {
      id: nextId(),
      ts: Date.now(),
      message: err.message || String(err),
      source,
      method: meta?.method,
      path: meta?.path,
      route,
      queryKey,
      mutationKey,
      stack: err.stack,
    };
  }

  return {
    id: nextId(),
    ts: Date.now(),
    message: String(err),
    source,
    method: meta?.method,
    path: meta?.path,
    route,
    queryKey,
    mutationKey,
  };
}

function isDupe(a: ReportedError, b: ReportedError): boolean {
  return (
    a.message === b.message &&
    a.code === b.code &&
    a.status === b.status &&
    a.path === b.path &&
    a.source === b.source
  );
}

export const useErrorStore = create<ErrorStore>((set, get) => ({
  errors: [],

  report: (err, meta) => {
    const entry = buildEntry(err, meta);
    const latest = get().errors[0];
    if (latest && isDupe(latest, entry) && entry.ts - latest.ts < DEDUPE_MS) {
      return null;
    }
    set((s) => ({
      errors: [entry, ...s.errors].slice(0, MAX_ERRORS),
    }));
    return entry;
  },

  clear: () => set({ errors: [] }),
}));

/** Report an error into the session history. Safe to call from anywhere. */
export function reportError(
  err: unknown,
  meta?: ReportErrorMeta,
): ReportedError | null {
  return useErrorStore.getState().report(err, meta);
}

/** Format one error for clipboard paste (support / debug). */
export function formatErrorForClipboard(entry: ReportedError): string {
  const t = getT();
  const lines: string[] = [
    `${t("errors.clipboard.time")}: ${new Date(entry.ts).toISOString()}`,
    `${t("errors.clipboard.source")}: ${clipboardSourceLabel(entry.source, t)}`,
    `${t("errors.clipboard.message")}: ${entry.message}`,
  ];
  if (entry.code != null) {
    lines.push(`${t("errors.clipboard.code")}: ${entry.code}`);
  }
  if (entry.status != null) {
    lines.push(`${t("errors.clipboard.status")}: ${entry.status}`);
  }
  if (entry.method || entry.path) {
    lines.push(
      `${t("errors.clipboard.request")}: ${entry.method ?? "?"} ${entry.path ?? "?"}`,
    );
  }
  if (entry.route) lines.push(`${t("errors.clipboard.route")}: ${entry.route}`);
  if (entry.queryKey) {
    lines.push(`${t("errors.clipboard.queryKey")}: ${entry.queryKey}`);
  }
  if (entry.mutationKey) {
    lines.push(`${t("errors.clipboard.mutationKey")}: ${entry.mutationKey}`);
  }
  if (entry.details !== undefined) {
    lines.push(
      `${t("errors.clipboard.details")}: ${
        typeof entry.details === "string"
          ? entry.details
          : JSON.stringify(entry.details, null, 2)
      }`,
    );
  }
  if (entry.stack) {
    lines.push(`${t("errors.clipboard.stack")}:`);
    lines.push(entry.stack);
  }
  return lines.join("\n");
}

function clipboardSourceLabel(
  source: ErrorSource,
  t: ReturnType<typeof getT>,
): string {
  const key = `errors.source.${source}` as MessageKey;
  return t(key);
}

/** Format all errors for clipboard (newest first). */
export function formatAllErrorsForClipboard(entries: ReportedError[]): string {
  if (entries.length === 0) return "";
  const t = getT();
  return entries
    .map(
      (e, i) =>
        `${t("errors.clipboard.separator", { i: i + 1, n: entries.length })}\n${formatErrorForClipboard(e)}`,
    )
    .join("\n\n");
}
