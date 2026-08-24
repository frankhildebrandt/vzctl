import { loadProject } from "@/features/persistence/projectIo";
import type { Environment } from "@/domain/hypernetwork/schema";
import type { GuestService } from "@/lib/guestLogs";
import { listGuestServices } from "@/lib/guestLogs";
import type { StackVmItem } from "@/lib/stackStatus";
import { parseStackInventory } from "@/lib/stackStatus";
import { isRunning } from "@/lib/vms";
import { runVzctl } from "@/lib/vzctl";

export type LogStreamKind = "iwatch" | "journald";

export type VmLogStream = {
  source: string;
  kind: LogStreamKind;
  live?: boolean;
};

export type StackLogStream = {
  vmId: string;
  source: string;
  kind: LogStreamKind;
  label: string;
};

/** Config-/Kurzname ohne Project-Prefix (`edge/web` → `web`). */
export function stackVmShortLabel(vmId: string): string {
  const slash = vmId.lastIndexOf("/");
  return slash >= 0 ? vmId.slice(slash + 1) : vmId;
}

/** Keep only guest publishers backed by iwatch. */
export function filterIwatchServices(services: GuestService[]): GuestService[] {
  return services.filter((service) => service.kind === "iwatch");
}

export function journaldStreamsFromEnv(
  vmId: string,
  env: Environment,
): VmLogStream[] {
  const slash = vmId.indexOf("/");
  if (slash < 0) return [];
  const project = vmId.slice(0, slash);
  const vmKey = vmId.slice(slash + 1);
  if (env.spec.project !== project) return [];
  const units = env.spec.vms[vmKey]?.logs?.journald ?? [];
  return units.map((source) => ({ source, kind: "journald" as const }));
}

export function iwatchStreamsFromServices(
  services: GuestService[],
): VmLogStream[] {
  return filterIwatchServices(services)
    .map((service) => ({ source: service.name, kind: "iwatch" as const, live: true }))
    .sort((a, b) => a.source.localeCompare(b.source));
}

export function mergeVmLogStreams(
  journald: VmLogStream[],
  iwatch: VmLogStream[],
): VmLogStream[] {
  const merged = journald.map((stream) => ({ ...stream }));
  for (const stream of iwatch) {
    if (!merged.some((entry) => entry.source === stream.source)) {
      merged.push(stream);
    }
  }
  return merged.sort((a, b) => {
    if (a.kind !== b.kind) {
      return a.kind === "journald" ? -1 : 1;
    }
    return a.source.localeCompare(b.source);
  });
}

export async function listVmLogStreams(
  vmId: string,
  stackPath?: string,
  running = false,
): Promise<VmLogStream[]> {
  let journald: VmLogStream[] = [];
  if (stackPath) {
    try {
      const { env } = await loadProject(stackPath);
      journald = journaldStreamsFromEnv(vmId, env);
    } catch {
      journald = [];
    }
  }
  let iwatch: VmLogStream[] = [];
  if (running) {
    try {
      const services = await listGuestServices(vmId);
      iwatch = iwatchStreamsFromServices(services);
    } catch {
      iwatch = [];
    }
  }
  return mergeVmLogStreams(journald, iwatch);
}

function canPollVm(item: StackVmItem): boolean {
  return item.present !== false && isRunning(item.state);
}

export async function listStackLogStreams(
  stackPath: string,
): Promise<StackLogStream[]> {
  const { env } = await loadProject(stackPath);
  const statusRaw = await runVzctl(stackPath, "status");
  const inventory = parseStackInventory(statusRaw);
  const items = inventory?.items ?? [];
  const streams: StackLogStream[] = [];

  for (const item of items) {
    if (item.present === false) continue;
    for (const stream of journaldStreamsFromEnv(item.id, env)) {
      streams.push({
        vmId: item.id,
        source: stream.source,
        kind: stream.kind,
        label: `${stackVmShortLabel(item.id)}/${stream.source}`,
      });
    }
  }

  const running = items.filter(canPollVm);
  await Promise.all(
    running.map(async (item) => {
      try {
        const iwatch = await listVmLogStreams(item.id, stackPath, true);
        for (const stream of iwatch.filter((s) => s.kind === "iwatch")) {
          if (
            !streams.some(
              (entry) =>
                entry.vmId === item.id && entry.source === stream.source,
            )
          ) {
            streams.push({
              vmId: item.id,
              source: stream.source,
              kind: stream.kind,
              label: `${stackVmShortLabel(item.id)}/${stream.source}`,
            });
          }
        }
      } catch {
        /* ignore guest-service errors */
      }
    }),
  );

  return streams.sort((a, b) => {
    const byVm = a.vmId.localeCompare(b.vmId);
    if (byVm !== 0) return byVm;
    if (a.kind !== b.kind) {
      return a.kind === "journald" ? -1 : 1;
    }
    return a.source.localeCompare(b.source);
  });
}

export function isLogStreamActive(
  section: string,
  streamSource: string,
  activeSource: string | undefined,
  streams: VmLogStream[],
): boolean {
  if (section !== "logs") return false;
  if (activeSource) return activeSource === streamSource;
  return streams[0]?.source === streamSource;
}

export const logStreamKeys = {
  vm: (vmId: string, stackPath?: string) =>
    ["log-streams", "vm", vmId, stackPath ?? ""] as const,
  stack: (stackPath: string) => ["log-streams", "stack", stackPath] as const,
};
