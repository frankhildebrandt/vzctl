import { api, encodeId } from "@/lib/api";

export type SystemdUnitType = "service" | "timer" | "socket";

export type SystemdStatus = {
  available: boolean;
  version?: string;
};

export type SystemdUnit = {
  name: string;
  type: SystemdUnitType;
  load: string;
  active: string;
  sub: string;
  description: string;
};

export type SystemdUnitDetail = SystemdUnit & {
  unit_file?: string;
  fragment?: string;
};

export const systemdKeys = {
  all: ["systemd"] as const,
  status: (vmId: string) => [...systemdKeys.all, "status", vmId] as const,
  units: (vmId: string, type: SystemdUnitType) =>
    [...systemdKeys.all, "units", vmId, type] as const,
  detail: (vmId: string, unit: string) =>
    [...systemdKeys.all, "detail", vmId, unit] as const,
};

function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function asUnitType(value: unknown): SystemdUnitType {
  if (value === "timer" || value === "socket") return value;
  return "service";
}

function asUnitDetail(value: unknown): SystemdUnitDetail | null {
  if (!value || typeof value !== "object") return null;
  const obj = value as Record<string, unknown>;
  const name = asString(obj.name);
  if (!name) return null;
  return {
    name,
    type: asUnitType(obj.type),
    load: asString(obj.load),
    active: asString(obj.active),
    sub: asString(obj.sub),
    description: asString(obj.description),
    unit_file: asString(obj.unit_file) || undefined,
    fragment: asString(obj.fragment) || undefined,
  };
}

function asUnits(value: unknown): SystemdUnit[] {
  if (!Array.isArray(value)) return [];
  return value.map((row) => {
    const obj = row && typeof row === "object" ? (row as Record<string, unknown>) : {};
    return {
      name: asString(obj.name),
      type: asUnitType(obj.type),
      load: asString(obj.load),
      active: asString(obj.active),
      sub: asString(obj.sub),
      description: asString(obj.description),
    };
  });
}

export async function fetchSystemdStatus(vmId: string): Promise<SystemdStatus> {
  const raw = (await api.get(`/v1/vms/${encodeId(vmId)}/systemd`)) as Record<
    string,
    unknown
  >;
  return {
    available: raw.available === true,
    version: asString(raw.version) || undefined,
  };
}

export async function listSystemdUnits(
  vmId: string,
  type: SystemdUnitType,
): Promise<SystemdUnit[]> {
  const raw = (await api.get(
    `/v1/vms/${encodeId(vmId)}/systemd/units?type=${encodeURIComponent(type)}`,
  )) as Record<string, unknown>;
  return asUnits(raw.units);
}

export async function fetchSystemdUnitDetail(
  vmId: string,
  unit: string,
): Promise<SystemdUnitDetail> {
  const raw = (await api.get(
    `/v1/vms/${encodeId(vmId)}/systemd/units/${encodeId(unit)}`,
  )) as Record<string, unknown>;
  const detail = asUnitDetail(raw.unit);
  if (!detail) {
    throw new Error(`invalid systemd unit detail for ${unit}`);
  }
  return detail;
}

export async function startSystemdUnit(vmId: string, unit: string): Promise<void> {
  await api.post(
    `/v1/vms/${encodeId(vmId)}/systemd/units/${encodeId(unit)}/start`,
  );
}

export async function stopSystemdUnit(vmId: string, unit: string): Promise<void> {
  await api.post(
    `/v1/vms/${encodeId(vmId)}/systemd/units/${encodeId(unit)}/stop`,
  );
}

export async function restartSystemdUnit(
  vmId: string,
  unit: string,
): Promise<void> {
  await api.post(
    `/v1/vms/${encodeId(vmId)}/systemd/units/${encodeId(unit)}/restart`,
  );
}

export function isUnitActive(unit: SystemdUnit): boolean {
  const active = unit.active.toLowerCase();
  const sub = unit.sub.toLowerCase();
  if (active !== "active") return false;
  return sub === "running" || sub === "listening" || sub === "waiting";
}

export type UnitVisualState =
  | "running"
  | "exited"
  | "inactive"
  | "activating"
  | "failed"
  | "other";

export function unitVisualState(unit: SystemdUnit): UnitVisualState {
  const active = unit.active.toLowerCase();
  const sub = unit.sub.toLowerCase();
  if (active === "failed" || sub === "failed") return "failed";
  if (
    sub === "activating" ||
    sub === "deactivating" ||
    sub === "reloading" ||
    sub.endsWith("-pre") ||
    sub.startsWith("start-") ||
    sub.startsWith("stop-")
  ) {
    return "activating";
  }
  if (isUnitActive(unit)) return "running";
  if (active === "active") return "exited";
  if (active === "inactive") return "inactive";
  return "other";
}

export function splitUnitName(name: string): { base: string; suffix: string } {
  const dot = name.indexOf(".");
  if (dot <= 0) return { base: name, suffix: "" };
  return { base: name.slice(0, dot), suffix: name.slice(dot) };
}

export function unitStatusLabel(unit: SystemdUnit): string {
  if (unit.sub) return `${unit.active}/${unit.sub}`;
  return unit.active || unit.load || "unknown";
}
