import type { LogsQuery } from "@/lib/guestLogs";

export const TEXT_FILTER_DEBOUNCE_MS = 300;

export type GuestLogFilters = {
  q: string;
  minLevel: string;
  groupField: string;
  groupValue: string;
  fieldFilters: Record<string, string>;
};

/** Stable key for select-based filters (immediate reconnect). */
export function serializeSelectFilterKey(filters: GuestLogFilters): string {
  return JSON.stringify({
    minLevel: filters.minLevel || "",
    groupField: filters.groupField || "",
    groupValue: filters.groupValue || "",
  });
}

function serializeFieldFilters(fieldFilters: Record<string, string>): string {
  const entries = Object.entries(fieldFilters)
    .filter(([, value]) => value)
    .sort(([a], [b]) => a.localeCompare(b));
  return JSON.stringify(entries);
}

/** Stable key for debounced text filters (`q`, `filter.*`). */
export function serializeTextFilterKey(filters: GuestLogFilters): string {
  return JSON.stringify({
    q: filters.q || "",
    fieldFilters: serializeFieldFilters(filters.fieldFilters),
  });
}

/** Full filter query key for reconnect effects (select + text). */
export function serializeFilterQueryKey(filters: GuestLogFilters): string {
  return `${serializeSelectFilterKey(filters)}\0${serializeTextFilterKey(filters)}`;
}

export function buildLogsStreamQuery(
  filters: GuestLogFilters,
  extra?: Partial<LogsQuery>,
): LogsQuery {
  return {
    q: filters.q || undefined,
    minLevel: filters.minLevel || undefined,
    groupField: filters.groupField || undefined,
    groupValue: filters.groupValue || undefined,
    filters: filters.fieldFilters,
    ...extra,
  };
}
