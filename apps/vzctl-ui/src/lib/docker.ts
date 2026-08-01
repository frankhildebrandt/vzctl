import { api, encodeId } from "@/lib/api";
import {
  assertEnvelopeOk,
  parseEnvelope,
  type VzctlEnvelope,
} from "@/lib/vzctl";

export type DockerContainer = {
  id: string;
  names: string;
  image: string;
  status: string;
  state: string;
  ports: string;
  command: string;
  ip: string;
};

export type RunContainerInput = {
  project: string;
  image: string;
  name?: string;
  env?: string[];
  ports?: string[];
  cmd?: string[];
};

export const dockerKeys = {
  all: ["docker"] as const,
  containers: (project: string) => [...dockerKeys.all, "containers", project] as const,
  inspect: (project: string, id: string) =>
    [...dockerKeys.all, "inspect", project, id] as const,
};

export function projectFromVmId(vmId: string): string | null {
  const slash = vmId.indexOf("/");
  if (slash <= 0) return null;
  return vmId.slice(0, slash);
}

export function shortContainerId(id: string): string {
  return id.length > 12 ? id.slice(0, 12) : id;
}

export function isContainerRunning(state: string | undefined): boolean {
  const s = (state ?? "").toLowerCase();
  return s === "running" || s.startsWith("up");
}

function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function asContainers(value: unknown): DockerContainer[] {
  if (!Array.isArray(value)) return [];
  return value.map((row) => {
    const obj = row && typeof row === "object" ? (row as Record<string, unknown>) : {};
    return {
      id: asString(obj.id),
      names: asString(obj.names),
      image: asString(obj.image),
      status: asString(obj.status),
      state: asString(obj.state),
      ports: asString(obj.ports),
      command: asString(obj.command),
      ip: asString(obj.ip),
    };
  });
}

export async function listContainers(project: string): Promise<DockerContainer[]> {
  const envelope = parseEnvelope(
    await api.get(`/v1/projects/${encodeId(project)}/containers`),
  );
  assertEnvelopeOk(envelope, "docker ps failed");
  const summary = envelope.summary as Record<string, unknown> | undefined;
  return asContainers(summary?.containers ?? envelope.containers);
}

export async function inspectContainer(
  project: string,
  id: string,
): Promise<Record<string, unknown>> {
  const envelope = parseEnvelope(
    await api.get(`/v1/projects/${encodeId(project)}/containers/${encodeId(id)}`),
  );
  assertEnvelopeOk(envelope, `docker inspect ${id} failed`);
  const summary = envelope.summary as Record<string, unknown> | undefined;
  const inspect = summary?.inspect ?? envelope.inspect;
  if (inspect && typeof inspect === "object" && !Array.isArray(inspect)) {
    return inspect as Record<string, unknown>;
  }
  return {};
}

export async function startContainer(
  project: string,
  id: string,
): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await api.post(
      `/v1/projects/${encodeId(project)}/containers/${encodeId(id)}/start`,
    ),
  );
  assertEnvelopeOk(envelope, `docker start ${id} failed`);
  return envelope;
}

export async function stopContainer(
  project: string,
  id: string,
): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await api.post(
      `/v1/projects/${encodeId(project)}/containers/${encodeId(id)}/stop`,
    ),
  );
  assertEnvelopeOk(envelope, `docker stop ${id} failed`);
  return envelope;
}

export async function restartContainer(
  project: string,
  id: string,
): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await api.post(
      `/v1/projects/${encodeId(project)}/containers/${encodeId(id)}/restart`,
    ),
  );
  assertEnvelopeOk(envelope, `docker restart ${id} failed`);
  return envelope;
}

export async function runContainer(input: RunContainerInput): Promise<string> {
  const envelope = parseEnvelope(
    await api.post(`/v1/projects/${encodeId(input.project)}/containers`, {
      image: input.image,
      name: input.name,
      env: input.env,
      ports: input.ports,
      cmd: input.cmd,
    }),
  );
  assertEnvelopeOk(envelope, "docker run failed");
  const summary = envelope.summary as Record<string, unknown> | undefined;
  const id = summary?.container_id;
  if (typeof id === "string" && id) return id;
  throw new Error("docker run missing container_id");
}
