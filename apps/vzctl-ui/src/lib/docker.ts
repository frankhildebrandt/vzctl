import {
  assertEnvelopeOk,
  parseEnvelope,
  runVzctlArgv,
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

/** Project from `{project}/{vm}` runtime id; null when flat (no docker context). */
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
  const raw = await runVzctlArgv([
    "docker",
    "ps",
    "--project",
    project,
    "--all",
  ]);
  const envelope = parseEnvelope(raw);
  assertEnvelopeOk(envelope, "docker ps failed");
  const summary = envelope.summary as Record<string, unknown> | undefined;
  return asContainers(summary?.containers);
}

export async function inspectContainer(
  project: string,
  id: string,
): Promise<Record<string, unknown>> {
  const raw = await runVzctlArgv([
    "docker",
    "inspect",
    "--project",
    project,
    id,
  ]);
  const envelope = parseEnvelope(raw);
  assertEnvelopeOk(envelope, `docker inspect ${id} failed`);
  const summary = envelope.summary as Record<string, unknown> | undefined;
  const inspect = summary?.inspect;
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
    await runVzctlArgv(["docker", "start", "--project", project, id]),
  );
  assertEnvelopeOk(envelope, `docker start ${id} failed`);
  return envelope;
}

export async function stopContainer(
  project: string,
  id: string,
): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await runVzctlArgv(["docker", "stop", "--project", project, id]),
  );
  assertEnvelopeOk(envelope, `docker stop ${id} failed`);
  return envelope;
}

export async function restartContainer(
  project: string,
  id: string,
): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await runVzctlArgv(["docker", "restart", "--project", project, id]),
  );
  assertEnvelopeOk(envelope, `docker restart ${id} failed`);
  return envelope;
}

export async function runContainer(input: RunContainerInput): Promise<string> {
  const args = ["docker", "run", "--project", input.project, "--image", input.image];
  if (input.name) {
    args.push("--name", input.name);
  }
  for (const env of input.env ?? []) {
    const trimmed = env.trim();
    if (trimmed) args.push("-e", trimmed);
  }
  for (const port of input.ports ?? []) {
    const trimmed = port.trim();
    if (trimmed) args.push("-p", trimmed);
  }
  if (input.cmd && input.cmd.length > 0) {
    args.push(...input.cmd);
  }
  const envelope = parseEnvelope(await runVzctlArgv(args));
  assertEnvelopeOk(envelope, "docker run failed");
  const summary = envelope.summary as Record<string, unknown> | undefined;
  const id = summary?.container_id;
  if (typeof id === "string" && id) return id;
  throw new Error("docker run missing container_id");
}
