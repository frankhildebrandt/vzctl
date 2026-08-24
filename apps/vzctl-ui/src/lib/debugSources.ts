import { api, encodeId } from "@/lib/api";

export type DebugSourceTypeSummary = {
  default: string;
  via: string;
  entries: string[];
};

export type DebugSourcesListResponse = {
  command: string;
  status: string;
  exit_code: number;
  sources?: {
    dataSources?: Record<string, DebugSourceTypeSummary>;
  };
};

export type DebugSqlResult = {
  command: string;
  status: string;
  exit_code: number;
  summary?: { message?: string; rowCount?: number };
  result?: {
    columns: string[];
    rows: string[][];
    rowCount: number;
    truncated?: boolean;
  };
  exec?: { stderr?: string; stdout?: string };
};

export type DebugDoctorResponse = {
  command: string;
  status: string;
  exit_code: number;
  doctor?: {
    ok: boolean;
    checks: Array<{
      type: string;
      status: string;
      via?: string;
      vmId?: string;
      tool?: string;
      message?: string;
    }>;
  };
};

export function debugSourcesPath(stackId: string): string {
  return `/v1/stacks/${encodeId(stackId)}/debug/sources`;
}

export function debugDoctorPath(stackId: string): string {
  return `/v1/stacks/${encodeId(stackId)}/debug/doctor`;
}

export async function fetchDebugSources(stackId: string) {
  return api.get<DebugSourcesListResponse>(debugSourcesPath(stackId));
}

export async function fetchDebugDoctor(stackId: string) {
  return api.get<DebugDoctorResponse>(debugDoctorPath(stackId));
}

export async function runPostgresQuery(
  stackId: string,
  body: { source?: string; sql: string; maxRows?: number; timeoutMs?: number },
) {
  return api.post<DebugSqlResult>(
    `/v1/stacks/${encodeId(stackId)}/debug/postgres/query`,
    body,
  );
}

export async function runMysqlQuery(
  stackId: string,
  body: { source?: string; sql: string; maxRows?: number; timeoutMs?: number },
) {
  return api.post<DebugSqlResult>(
    `/v1/stacks/${encodeId(stackId)}/debug/mysql/query`,
    body,
  );
}

export async function fetchNatsStreams(stackId: string, source?: string) {
  const query = source ? `?source=${encodeURIComponent(source)}` : "";
  return api.get(`/v1/stacks/${encodeId(stackId)}/debug/nats/streams${query}`);
}

export async function runNatsSub(
  stackId: string,
  body: {
    source?: string;
    subject: string;
    count?: number;
    timeoutMs?: number;
  },
) {
  return api.post(
    `/v1/stacks/${encodeId(stackId)}/debug/nats/sub`,
    body,
  );
}

export async function runNatsKvGet(
  stackId: string,
  body: { source?: string; bucket: string; key: string },
) {
  return api.post(
    `/v1/stacks/${encodeId(stackId)}/debug/nats/kv/get`,
    body,
  );
}

export async function runNatsKvWatch(
  stackId: string,
  body: {
    source?: string;
    bucket: string;
    key?: string;
    timeoutMs?: number;
  },
) {
  return api.post(
    `/v1/stacks/${encodeId(stackId)}/debug/nats/kv/watch`,
    body,
  );
}
