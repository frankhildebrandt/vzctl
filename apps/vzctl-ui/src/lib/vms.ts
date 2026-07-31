import { assertEnvelopeOk, parseEnvelope, runVzctlArgv, type VzctlEnvelope } from "@/lib/vzctl";

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
  dataDiskGib: number;
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
  if (typeof row.id !== "string") return null;
  const resources = asResources(row.resources);
  return {
    id: row.id,
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
  const raw = await runVzctlArgv(["vm", "list"]);
  const envelope = parseEnvelope(raw);
  const vms = Array.isArray(envelope.vms) ? envelope.vms : [];
  return vms
    .map(parseListItem)
    .filter((entry): entry is VmListItem => entry !== null);
}

export async function inspectVm(id: string): Promise<VmInspect> {
  const raw = await runVzctlArgv(["vm", "inspect", id]);
  const envelope = parseEnvelope(raw);
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
  const resources = vm.resources ?? null;

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
    resources,
  };
}

export async function startVm(id: string): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(await runVzctlArgv(["vm", "start", id]));
  assertEnvelopeOk(envelope, `vm start ${id} failed`);
  return envelope;
}

export async function stopVm(id: string): Promise<VzctlEnvelope> {
  const envelope = parseEnvelope(await runVzctlArgv(["vm", "stop", id]));
  assertEnvelopeOk(envelope, `vm stop ${id} failed`);
  return envelope;
}

export async function deleteVm(
  id: string,
  force = false,
): Promise<VzctlEnvelope> {
  const args = ["vm", "delete", id];
  if (force) args.push("--force");
  const envelope = parseEnvelope(await runVzctlArgv(args));
  assertEnvelopeOk(envelope, `vm delete ${id} failed`);
  return envelope;
}

export async function createVm(input: CreateVmInput): Promise<VzctlEnvelope> {
  const args = [
    "vm",
    "create",
    input.id,
    "--from",
    input.from,
    "--data-disk",
    String(input.dataDiskGib),
  ];
  if (input.cpus != null) {
    args.push("--cpus", String(input.cpus));
  }
  if (input.memory) {
    args.push("--memory", input.memory);
  }
  if (input.network) {
    args.push("--network", input.network);
  }
  if (input.project) {
    args.push("--project", input.project);
  }
  if (input.rootPassword) {
    args.push("--root-password", input.rootPassword);
  }
  for (const role of input.roles ?? []) {
    args.push("--role", role);
  }
  for (const mount of input.mounts ?? []) {
    let flag = `source=${mount.source},target=${mount.target}`;
    if (mount.tag) flag = `tag=${mount.tag},${flag}`;
    if (mount.readOnly) flag += ",ro";
    args.push("--mount", flag);
  }
  const envelope = parseEnvelope(await runVzctlArgv(args));
  assertEnvelopeOk(envelope, `vm create ${input.id} failed`);
  return envelope;
}

/** Encode slash-namespaced VM IDs for `/vms/$vmId` route params. */
export function encodeVmIdParam(id: string): string {
  return encodeURIComponent(id);
}

/** Decode route param back to runtime VM ID (`project/vm` or flat). */
export function decodeVmIdParam(param: string): string {
  try {
    return decodeURIComponent(param);
  } catch {
    return param;
  }
}

/** Resolved runtime id after create (may be `{project}/{id}` when --project was set). */
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
  const args = ["vm", "modify", input.id];
  if (input.cpus != null) args.push("--cpus", String(input.cpus));
  if (input.memory) args.push("--memory", input.memory);
  const envelope = parseEnvelope(await runVzctlArgv(args));
  assertEnvelopeOk(envelope, `vm modify ${input.id} failed`);
  return envelope;
}

export async function mountVm(input: MountVmInput): Promise<VzctlEnvelope> {
  const args = [
    "vm",
    "mount",
    input.id,
    "--source",
    input.source,
    "--target",
    input.target,
  ];
  if (input.tag) args.push("--tag", input.tag);
  if (input.readOnly) args.push("--ro");
  const envelope = parseEnvelope(await runVzctlArgv(args));
  assertEnvelopeOk(envelope, `vm mount ${input.id} failed`);
  return envelope;
}

export async function unmountVm(
  id: string,
  opts: { target?: string; tag?: string },
): Promise<VzctlEnvelope> {
  const args = ["vm", "unmount", id];
  if (opts.target) args.push("--target", opts.target);
  if (opts.tag) args.push("--tag", opts.tag);
  const envelope = parseEnvelope(await runVzctlArgv(args));
  assertEnvelopeOk(envelope, `vm unmount ${id} failed`);
  return envelope;
}

export async function listMounts(id: string): Promise<VmMount[]> {
  const envelope = parseEnvelope(await runVzctlArgv(["vm", "mounts", id]));
  return asMounts(envelope.mounts);
}

export function isRunning(state: string | undefined): boolean {
  return state === "running" || state === "starting";
}

export const IMAGE_ALIAS_HINTS = [
  "ubuntu",
  "debian",
  "alpine",
  "arch",
  "fedora",
  "rocky",
  "alma",
  "opensuse",
  "coreos",
  "flatcar",
] as const;
