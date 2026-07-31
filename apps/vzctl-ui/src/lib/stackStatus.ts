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

const LABELS: Record<StackPhase, string> = {
  down: "Down",
  starting: "Starting",
  stopping: "Stopping",
  reconciling: "Up (Reconciling)",
  running: "Up (Running)",
  partial: "Up (Partial)",
  failed: "Failed",
  unknown: "Unknown",
};

export function phaseLabel(phase: StackPhase): string {
  return LABELS[phase] ?? LABELS.unknown;
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
      label: data.label || phaseLabel(phase),
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
}): { phase: StackPhase; label: string; inventory: StackInventory | null } {
  const { inventory, applyActive, applyMode, applyFailed } = input;

  if (applyActive) {
    const mode = applyMode ?? "apply";
    if (mode === "down") {
      return { phase: "stopping", label: phaseLabel("stopping"), inventory };
    }
    const running = inventory?.vms?.running ?? 0;
    if (mode === "up" && running === 0) {
      return { phase: "starting", label: phaseLabel("starting"), inventory };
    }
    return {
      phase: "reconciling",
      label: phaseLabel("reconciling"),
      inventory,
    };
  }

  if (applyFailed) {
    return { phase: "failed", label: phaseLabel("failed"), inventory };
  }

  if (!inventory) {
    return { phase: "unknown", label: phaseLabel("unknown"), inventory: null };
  }

  const phase = normalizePhase(inventory.phase);
  return { phase, label: inventory.label || phaseLabel(phase), inventory };
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
