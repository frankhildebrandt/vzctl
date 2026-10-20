import type { MessageKey } from "@/lib/i18n";

export type StackHealthState = "ok" | "degraded" | "critical" | "unknown" | "stopped";
export type StackHealthWarning = { message: string; vmId?: string; hint?: string };
export type StackHealth = { state: StackHealthState; warnings: StackHealthWarning[]; error?: string; stoppedVms?: number };

export const HEALTH_LABEL_KEYS: Record<StackHealthState, MessageKey> = {
  ok: "stack.health.ok",
  stopped: "stack.health.stopped",
  degraded: "stack.health.degraded",
  critical: "stack.health.critical",
  unknown: "stack.health.unknown",
};

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : undefined;
}

/** Process inventory does not imply health. Old or failed bundles are unknown. */
export function parseStackHealth(raw: unknown): StackHealth {
  let bundle: unknown = raw;
  if (typeof raw === "string") {
    try { bundle = JSON.parse(raw); } catch { return { state: "unknown", warnings: [] }; }
  }
  const section = record(record(record(bundle)?.sections)?.health);
  const data = record(section?.data);
  const summary = record(data?.summary);
  const health = summary?.health;
  // stack.status may exit nonzero for a valid degraded/critical report.
  const state = health === "degraded" || health === "critical"
    ? health : section?.ok === true && health === "ok" ? summary?.checks === "skipped" ? "stopped" : "ok" : "unknown";
  const warnings = (Array.isArray(data?.warnings) ? data.warnings : []).flatMap((item): StackHealthWarning[] => {
    const warning = record(item);
    if (typeof warning?.message !== "string") return [];
    return [{ message: warning.message,
      ...(typeof warning.vm_id === "string" ? { vmId: warning.vm_id } : {}),
      ...(typeof warning.hint === "string" ? { hint: warning.hint } : {}),
    }];
  });
  return { state, warnings,
    ...(typeof summary?.stopped_vms === "number" && Number.isSafeInteger(summary.stopped_vms) && summary.stopped_vms > 0
      ? { stoppedVms: summary.stopped_vms } : {}), ...(state === "unknown" && typeof section?.stderr === "string" && section.stderr
    ? { error: section.stderr } : {}) };
}
