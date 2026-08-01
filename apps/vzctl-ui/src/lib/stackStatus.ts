import type { MessageKey } from "@/lib/i18n";
import type { TFunction } from "@/lib/i18n/useT";

export type StackPhase =
  | "down"
  | "starting"
  | "stopping"
  | "reconciling"
  | "running"
  | "partial"
  | "failed"
  | "unknown";

export type StackVmItem = {
  id: string;
  name?: string;
  state: string;
  present?: boolean;
};

export type StackInventory = {
  phase: StackPhase;
  label: string;
  stack_id?: string | null;
  project?: string | null;
  vms?: {
    desired: number;
    running: number;
    starting: number;
    stopping: number;
    stopped: number;
    missing: number;
    other?: number;
  };
  items?: StackVmItem[];
};

const PHASE_KEYS: Record<StackPhase, MessageKey> = {
  down: "stack.phase.down",
  starting: "stack.phase.starting",
  stopping: "stack.phase.stopping",
  reconciling: "stack.phase.reconciling",
  running: "stack.phase.running",
  partial: "stack.phase.partial",
  failed: "stack.phase.failed",
  unknown: "stack.phase.unknown",
};

export function phaseLabel(phase: StackPhase, t: TFunction): string {
  return t(PHASE_KEYS[phase] ?? PHASE_KEYS.unknown);
}

export function parseStackInventory(
  statusRaw: string | null | undefined,
): StackInventory | null {
  if (!statusRaw) return null;
  try {
    const parsed = JSON.parse(statusRaw) as {
      sections?: { stack?: { data?: StackInventory } };
    };
    const data = parsed.sections?.stack?.data;
    if (!data || typeof data !== "object") return null;
    const phase = normalizePhase(data.phase);
    return {
      ...data,
      phase,
      label: data.label ?? "",
    };
  } catch {
    return null;
  }
}

export function deriveStackStatus(input: {
  inventory: StackInventory | null;
  applyActive: boolean;
  applyMode: string | null;
  applyFailed: boolean;
  t: TFunction;
}): { phase: StackPhase; label: string; inventory: StackInventory | null } {
  const { inventory, applyActive, applyMode, applyFailed, t } = input;

  if (applyActive) {
    const mode = applyMode ?? "apply";
    if (mode === "down") {
      return { phase: "stopping", label: phaseLabel("stopping", t), inventory };
    }
    const running = inventory?.vms?.running ?? 0;
    if (mode === "up" && running === 0) {
      return { phase: "starting", label: phaseLabel("starting", t), inventory };
    }
    return {
      phase: "reconciling",
      label: phaseLabel("reconciling", t),
      inventory,
    };
  }

  if (applyFailed) {
    return { phase: "failed", label: phaseLabel("failed", t), inventory };
  }

  if (!inventory) {
    return { phase: "unknown", label: phaseLabel("unknown", t), inventory: null };
  }

  const phase = normalizePhase(inventory.phase);
  return { phase, label: phaseLabel(phase, t), inventory };
}

function normalizePhase(value: unknown): StackPhase {
  switch (String(value ?? "")) {
    case "down":
    case "starting":
    case "stopping":
    case "reconciling":
    case "running":
    case "partial":
    case "failed":
      return String(value) as StackPhase;
    default:
      return "unknown";
  }
}
