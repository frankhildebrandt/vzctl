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
  observedFields?: string[];
  groupField?: string;
  groupValues?: string[];
};

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

/** Build iwatch `/api/logs` query string (`q`, `minLevel`, `group*`, `filter.*`). */
export function buildLogsQuery(input: LogsQuery): string {
  const params = new URLSearchParams();
  if (input.q) params.set("q", input.q);
  if (input.minLevel) params.set("minLevel", input.minLevel);
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

/** POST iwatch `/api/restart` for the published source (watched process, not the VM). */
export async function restartGuestProcess(
  vmId: string,
  name: string,
): Promise<void> {
  await api.post(guestServiceApiPath(vmId, name, "/api/restart"));
}
