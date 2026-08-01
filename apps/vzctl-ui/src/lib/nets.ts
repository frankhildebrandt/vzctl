import { api, encodeId } from "@/lib/api";

export type NetAttachment = {
  vm_id?: string;
  network?: string;
  network_name?: string;
  ip?: string;
};

export type NetRecord = {
  name: string;
  cidr?: string;
  mode?: string;
  nat_egress?: boolean;
  backend?: string;
  runtime_state?: string;
  project?: string;
  stack?: string;
};

export type NetSnapshot = {
  networks: NetRecord[];
  attachments: NetAttachment[];
};

export type NetDefault = {
  name: string;
  cidr: string;
} | null;

export const netKeys = {
  all: ["nets"] as const,
  list: () => [...netKeys.all, "list"] as const,
  default: () => [...netKeys.all, "default"] as const,
};

function asNetworks(value: unknown): NetRecord[] {
  if (!Array.isArray(value)) return [];
  return value.map((row) => {
    const obj = (row && typeof row === "object" ? row : {}) as Record<string, unknown>;
    return {
      name: typeof obj.name === "string" ? obj.name : "",
      cidr: typeof obj.cidr === "string" ? obj.cidr : undefined,
      mode: typeof obj.mode === "string" ? obj.mode : undefined,
      nat_egress: typeof obj.nat_egress === "boolean" ? obj.nat_egress : undefined,
      backend: typeof obj.backend === "string" ? obj.backend : undefined,
      runtime_state:
        typeof obj.runtime_state === "string" ? obj.runtime_state : undefined,
      project: typeof obj.project === "string" ? obj.project : undefined,
      stack: typeof obj.stack === "string" ? obj.stack : undefined,
    };
  }).filter((n) => n.name);
}

function asAttachments(value: unknown): NetAttachment[] {
  if (!Array.isArray(value)) return [];
  return value.map((row) => {
    const obj = (row && typeof row === "object" ? row : {}) as Record<string, unknown>;
    return {
      vm_id: typeof obj.vm_id === "string" ? obj.vm_id : undefined,
      network: typeof obj.network === "string" ? obj.network : undefined,
      network_name:
        typeof obj.network_name === "string" ? obj.network_name : undefined,
      ip: typeof obj.ip === "string" ? obj.ip : undefined,
    };
  });
}

export async function listNets(): Promise<NetSnapshot> {
  const data = await api.get<Record<string, unknown>>("/v1/nets");
  return {
    networks: asNetworks(data.networks ?? data),
    attachments: asAttachments(data.attachments),
  };
}

export async function getDefaultNet(): Promise<NetDefault> {
  const data = await api.get<unknown>("/v1/nets/default");
  if (!data || data === null) return null;
  if (typeof data !== "object") return null;
  const obj = data as Record<string, unknown>;
  const name = typeof obj.name === "string" ? obj.name : null;
  const cidr = typeof obj.cidr === "string" ? obj.cidr : null;
  if (!name || !cidr) return null;
  return { name, cidr };
}

export async function setDefaultNet(name: string, cidr: string): Promise<void> {
  await api.put("/v1/nets/default", { name, cidr });
}

export async function deleteNet(name: string): Promise<void> {
  await api.delete(`/v1/nets/${encodeId(name)}`);
}
