import { api, encodeId } from "@/lib/api";

export type NetAttachment = {
  network_id?: string;
  project?: string;
  vm_id?: string;
  network?: string;
  network_name?: string;
  ip?: string;
};

export type NetRecord = {
  id?: string;
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
  id?: string;
  project?: string;
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
      id: typeof obj.id === "string" && obj.id ? obj.id : undefined,
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
      network_id: typeof obj.network_id === "string" && obj.network_id ? obj.network_id : undefined,
      project: typeof obj.project === "string" ? obj.project : undefined,
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
  return { name, cidr,
    id: typeof obj.id === "string" && obj.id ? obj.id : undefined,
    project: typeof obj.project === "string" ? obj.project : undefined,
  };
}

export async function setDefaultNet(name: string, cidr: string): Promise<void> {
  await api.put("/v1/nets/default", { name, cidr });
}

export async function deleteNet(id: string, project?: string): Promise<void> {
  const scope = project ? `?project=${encodeURIComponent(project)}` : "";
  await api.delete(`/v1/nets/${encodeId(id)}${scope}`);
}

export function networkLabel(net: NetRecord): string {
  return net.project ? `${net.project}/${net.name}` : net.id ?? net.name;
}

/** Never send an ambiguous short name to a legacy server's delete endpoint. */
export function networkDeleteId(net: NetRecord, networks: NetRecord[]): string | null {
  const id = net.id ?? net.name;
  const matches = networks.filter((entry) => net.id
    ? (entry.id ?? entry.name) === id
    : entry.name === net.name);
  return matches.length === 1 && (matches[0].project ?? "") === (net.project ?? "") ? id : null;
}

export function networkAttachmentCount(net: NetRecord, snapshot: NetSnapshot): number | null {
  let count = 0;
  let unknown = false;
  for (const attachment of snapshot.attachments) {
    const candidates = attachment.network_id
      ? snapshot.networks.filter((entry) => (entry.id ?? entry.name) === attachment.network_id)
      : snapshot.networks.filter((entry) => entry.name === (attachment.network_name ?? attachment.network)
          && (!attachment.project || entry.project === attachment.project));
    if (!candidates.includes(net)) continue;
    if (candidates.length === 1) count += 1;
    else unknown = true;
  }
  return unknown ? null : count;
}

export function isDefaultNetwork(net: NetRecord, value: NetDefault, networks: NetRecord[]): boolean {
  if (!value) return false;
  if (value.id && net.id) return value.id === net.id && networkDeleteId(net, networks) !== null;
  // Old default responses describe the unowned global network, never a project namesake.
  return !net.project && !value.project && net.name === (value.id ?? value.name)
    && networkDeleteId(net, networks) !== null;
}
