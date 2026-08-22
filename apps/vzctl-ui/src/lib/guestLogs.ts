import { api, encodeId } from "@/lib/api";

export type GuestService = {
  name: string;
  kind: string;
  url?: string;
  pid?: number;
};

export type IwatchLine = {
  index?: number;
  session?: number;
  source?: string;
  text: string;
  fields?: Record<string, string>;
  ts?: string;
  level?: string;
};

export type IwatchStatus = {
  process?: string;
  bufferLen?: number;
  bufferCap?: number;
  commandTitle?: string;
  lastUrl?: string;
  observedFields?: string[];
  groupField?: string;
  groupValues?: string[];
};

export type IwatchLineDetail = {
  line: IwatchLine;
  pretty?: Record<string, string>;
};

export type IwatchShare = {
  text?: string;
};

export type GuestLogAction =
  | "/api/restart"
  | "/api/truncate"
  | "/api/separator"
  | "/api/open-url";

export type LogsQuery = {
  q?: string;
  minLevel?: string;
  groupField?: string;
  groupValue?: string;
  filters?: Record<string, string>;
  after?: number;
  before?: number;
  limit?: number;
  tail?: number;
};

export const IWATCH_LEVELS = [
  "all",
  "trace",
  "debug",
  "info",
  "warn",
  "error",
  "fatal",
] as const;

export type IwatchLevel = (typeof IWATCH_LEVELS)[number];

/** Build iwatch `/api/logs` query string (`q`, `minLevel`, `group*`, `filter.*`). */
export function buildLogsQuery(input: LogsQuery): string {
  const params = new URLSearchParams();
  if (input.q) params.set("q", input.q);
  if (input.minLevel && input.minLevel !== "all") {
    params.set("minLevel", input.minLevel);
  }
  if (input.groupField) params.set("groupField", input.groupField);
  if (input.groupValue) params.set("groupValue", input.groupValue);
  if (input.after != null) params.set("after", String(input.after));
  if (input.before != null) params.set("before", String(input.before));
  if (input.limit != null) params.set("limit", String(input.limit));
  if (input.tail != null) params.set("tail", String(input.tail));
  if (input.filters) {
    for (const [key, value] of Object.entries(input.filters)) {
      if (value) params.set(`filter.${key}`, value);
    }
  }
  const encoded = params.toString();
  return encoded ? `?${encoded}` : "";
}

export function guestServiceApiPath(
  vmId: string,
  name: string,
  apiPath: string,
  query: LogsQuery = {},
): string {
  const suffix = apiPath.startsWith("/") ? apiPath : `/${apiPath}`;
  return `/v1/vms/${encodeId(vmId)}/guest-services/${encodeId(name)}${suffix}${buildLogsQuery(query)}`;
}

export async function listGuestServices(vmId: string): Promise<GuestService[]> {
  const raw = (await api.get(`/v1/vms/${encodeId(vmId)}/guest-services`)) as {
    services?: GuestService[];
  };
  return Array.isArray(raw.services) ? raw.services : [];
}

export async function fetchGuestLogs(
  vmId: string,
  name: string,
  query: LogsQuery,
): Promise<IwatchLine[]> {
  const payload = (await api.get(
    guestServiceApiPath(vmId, name, "/api/logs", query),
  )) as { lines?: IwatchLine[] } | IwatchLine[];
  if (Array.isArray(payload)) return payload;
  return Array.isArray(payload.lines) ? payload.lines : [];
}

export async function fetchGuestLogStatus(
  vmId: string,
  name: string,
): Promise<IwatchStatus> {
  return (await api.get(
    guestServiceApiPath(vmId, name, "/api/status"),
  )) as IwatchStatus;
}

export async function fetchGuestLogLine(
  vmId: string,
  name: string,
  index: number,
  query: LogsQuery,
): Promise<IwatchLineDetail> {
  return (await api.get(
    guestServiceApiPath(vmId, name, `/api/logs/${index}`, query),
  )) as IwatchLineDetail;
}

export async function fetchGuestLogShare(
  vmId: string,
  name: string,
  index: number,
  query: LogsQuery,
  context?: number,
): Promise<IwatchShare> {
  const base = guestServiceApiPath(vmId, name, `/api/share/${index}`, query);
  const path =
    context != null && context > 0
      ? `${base}${base.includes("?") ? "&" : "?"}context=${context}`
      : base;
  return (await api.get(path)) as IwatchShare;
}

/** POST an iwatch control endpoint for the published source. */
export async function postGuestLogAction(
  vmId: string,
  name: string,
  action: GuestLogAction,
): Promise<void> {
  await api.post(guestServiceApiPath(vmId, name, action));
}

/** POST iwatch `/api/restart` for the published source (watched process, not the VM). */
export async function restartGuestProcess(
  vmId: string,
  name: string,
): Promise<void> {
  await postGuestLogAction(vmId, name, "/api/restart");
}
