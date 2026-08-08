import { api, encodeId } from "@/lib/api";
import {
  assertEnvelopeOk,
  parseEnvelope,
  type VzctlEnvelope,
} from "@/lib/vzctl";

export type VmNetwork = {
  name?: string;
  ip?: string;
  cidr?: string;
};

export type VmListItem = {
  id: string;
  state: string;
  pid: number | null;
  bundle?: string;
  "managed-by"?: string | null;
  roles: string[];
  ips: string[];
  networks: VmNetwork[];
  updated_at?: string;
  resources?: VmResources;
};

export type VmResources = {
  cpus: number;
  memory_mib: number;
};

export type VmMount = {
  name: string;
  source: string;
  target: string;
  read_only: boolean;
};

export type VmInspect = {
  envelope: VzctlEnvelope;
  vm: VmListItem;
  identity: Record<string, unknown> | null;
  disks: Record<string, unknown> | null;
  networks: VmNetwork[];
  agent: Record<string, unknown> | null;
  logs: { serial?: string } | null;
  warnings: string[];
  resources: VmResources | null;
};

export type CreateVmInput = {
  id: string;
  from: string;
  diskGib: number;
  cpus?: number;
  memory?: string;
  network?: string;
  roles?: string[];
  rootPassword?: string;
  project?: string;
  mounts?: Array<{
    source: string;
    target: string;
    tag?: string;
    readOnly?: boolean;
  }>;
};

export type ModifyVmInput = {
  id: string;
  cpus?: number;
  memory?: string;
};

export type MountVmInput = {
  id: string;
  source: string;
  target: string;
  tag?: string;
  readOnly?: boolean;
};

export const vmKeys = {
  all: ["vms"] as const,
  list: () => [...vmKeys.all, "list"] as const,
  detail: (id: string) => [...vmKeys.all, "detail", id] as const,
  mounts: (id: string) => [...vmKeys.all, "mounts", id] as const,
};

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

function asNetworks(value: unknown): VmNetwork[] {
  if (!Array.isArray(value)) return [];
  return value.map((entry) => {
    const row = entry as Record<string, unknown>;
    return {
      name: typeof row.name === "string" ? row.name : undefined,
      ip: typeof row.ip === "string" ? row.ip : undefined,
      cidr: typeof row.cidr === "string" ? row.cidr : undefined,
    };
  });
}

function asMounts(value: unknown): VmMount[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((entry) => {
      const row = entry as Record<string, unknown>;
      if (
        typeof row.name !== "string" ||
        typeof row.source !== "string" ||
        typeof row.target !== "string"
      ) {
        return null;
      }
      return {
        name: row.name,
        source: row.source,
        target: row.target,
        read_only: row.read_only === true,
      };
    })
    .filter((entry): entry is VmMount => entry !== null);
}

function asResources(value: unknown): VmResources | null {
  if (!value || typeof value !== "object") return null;
  const row = value as Record<string, unknown>;
  const cpus = typeof row.cpus === "number" ? row.cpus : Number(row.cpus);
  const memory =
    typeof row.memory_mib === "number" ? row.memory_mib : Number(row.memory_mib);
  if (!Number.isFinite(cpus) || !Number.isFinite(memory)) return null;
  return { cpus, memory_mib: memory };
}

function parseListItem(value: unknown): VmListItem | null {
  if (!value || typeof value !== "object") return null;
  const row = value as Record<string, unknown>;
  const id =
    typeof row.id === "string"
      ? row.id
      : typeof row.vm_id === "string"
        ? row.vm_id
        : null;
  if (!id) return null;
  const resources = asResources(row.resources);
  return {
    id,
    state: typeof row.state === "string" ? row.state : "unknown",
    pid: typeof row.pid === "number" ? row.pid : null,
    bundle: typeof row.bundle === "string" ? row.bundle : undefined,
    "managed-by":
      typeof row["managed-by"] === "string" ? row["managed-by"] : null,
    roles: asStringArray(row.roles),
    ips: asStringArray(row.ips),
    networks: asNetworks(row.networks),
    updated_at:
      typeof row.updated_at === "string" ? row.updated_at : undefined,
    resources: resources ?? undefined,
  };
}

export async function listVms(): Promise<VmListItem[]> {
  const data = await api.get<unknown>("/v1/vms");
  const envelope = parseEnvelope(data);
  if (Array.isArray(envelope.vms)) {
    return envelope.vms
      .map(parseListItem)
      .filter((entry): entry is VmListItem => entry !== null);
  }
  if (Array.isArray(data)) {
    return data
      .map(parseListItem)
      .filter((entry): entry is VmListItem => entry !== null);
  }
  return [];
}

export async function inspectVm(id: string): Promise<VmInspect> {
  const envelope = parseEnvelope(await api.get(`/v1/vms/${encodeId(id)}`));
  const vm =
    parseListItem(envelope.vm) ??
    ({
      id,
      state: "unknown",
      pid: null,
      roles: [],
      ips: [],
      networks: [],
    } satisfies VmListItem);

  return {
    envelope,
    vm,
    identity:
      envelope.identity && typeof envelope.identity === "object"
        ? (envelope.identity as Record<string, unknown>)
        : null,
    disks:
      envelope.disks && typeof envelope.disks === "object"
        ? (envelope.disks as Record<string, unknown>)
        : null,
    networks: asNetworks(envelope.networks),
    agent:
      envelope.agent && typeof envelope.agent === "object"
        ? (envelope.agent as Record<string, unknown>)
        : null,
    logs:
      envelope.logs && typeof envelope.logs === "object"
        ? (envelope.logs as { serial?: string })
        : null,
    warnings: asStringArray(envelope.warnings),
    resources: vm.resources ?? null,
  };
}

export async function startVm(id: string): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(await api.post(`/v1/vms/${encodeId(id)}/start`));
  assertEnvelopeOk(envelope, `vm start ${id} failed`);
  return envelope;
}

export async function stopVm(id: string): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(await api.post(`/v1/vms/${encodeId(id)}/stop`));
  assertEnvelopeOk(envelope, `vm stop ${id} failed`);
  return envelope;
}

export async function deleteVm(id: string, force = false): Promise<VzctlEnvelope> {
  const qs = force ? "?force=1" : "";
  const envelope = parseEnvelope(await api.delete(`/v1/vms/${encodeId(id)}${qs}`));
  assertEnvelopeOk(envelope, `vm delete ${id} failed`);
  return envelope;
}

export async function createVm(input: CreateVmInput): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await api.post("/v1/vms", {
      id: input.id,
      from: input.from,
      diskGib: input.diskGib,
      cpus: input.cpus,
      memory: input.memory,
      network: input.network,
      project: input.project,
      rootPassword: input.rootPassword,
      roles: input.roles,
      mounts: input.mounts,
    }),
  );
  assertEnvelopeOk(envelope, `vm create ${input.id} failed`);
  return envelope;
}

export function encodeVmIdParam(id: string): string {
  return encodeURIComponent(id);
}

export function decodeVmIdParam(param: string): string {
  try {
    return decodeURIComponent(param);
  } catch {
    return param;
  }
}

export function createdVmId(envelope: VzctlEnvelope, fallback: string): string {
  const summaryId = envelope.summary?.vm_id;
  if (typeof summaryId === "string" && summaryId) return summaryId;
  const vm = envelope.vm;
  if (vm && typeof vm === "object" && !Array.isArray(vm)) {
    const id = (vm as Record<string, unknown>).id;
    if (typeof id === "string" && id) return id;
  }
  return fallback;
}

export async function modifyVm(input: ModifyVmInput): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await api.patch(`/v1/vms/${encodeId(input.id)}`, {
      cpus: input.cpus,
      memory: input.memory,
    }),
  );
  assertEnvelopeOk(envelope, `vm modify ${input.id} failed`);
  return envelope;
}

export async function mountVm(input: MountVmInput): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(
    await api.post(`/v1/vms/${encodeId(input.id)}/mounts`, {
      source: input.source,
      target: input.target,
      tag: input.tag,
      readOnly: input.readOnly,
    }),
  );
  assertEnvelopeOk(envelope, `vm mount ${input.id} failed`);
  return envelope;
}

export async function unmountVm(
  id: string,
  opts: { target?: string; tag?: string },
): Promise<VzctlEnvelope> {
  if (!opts.tag) throw new Error("unmount requires tag");
  const envelope = parseEnvelope(
    await api.delete(`/v1/vms/${encodeId(id)}/mounts/${encodeId(opts.tag)}`),
  );
  assertEnvelopeOk(envelope, `vm unmount ${id} failed`);
  return envelope;
}

export async function listMounts(id: string): Promise<VmMount[]> {
  const envelope = parseEnvelope(await api.get(`/v1/vms/${encodeId(id)}/mounts`));
  return asMounts(envelope.mounts);
}

export function isRunning(state: string | undefined): boolean {
  return state === "running" || state === "starting";
}
