export type VzctlCommand = "diff" | "up" | "apply" | "down" | "status" | "validate";

export type VzctlEvent = {
  v: number;
  ts: string;
  type: string;
  data: Record<string, unknown>;
};

export const APPLY_STEPS = [
  "validate",
  "acquire_lease",
  "ensure_nets",
  "ensure_dns",
  "ensure_images",
  "ensure_vms",
  "attach_nets",
  "start_helpers",
  "await_agents",
  "ensure_ca",
  "ensure_oidc",
  "ensure_ingress",
  "ensure_ca_rollout",
  "ensure_oidc_inject",
  "ensure_docker_context",
  "ensure_ports",
  "apply_routes_policies",
  "release_lease",
] as const;

export const DOWN_STEPS = [
  "purge_ingress",
  "purge_oidc",
  "stop_helpers",
  "detach_nets",
  "destroy_managed",
  "purge_docker_context",
  "purge_ports",
  "dns_cleanup",
  "release_lease",
] as const;

export type RunVzctlOptions = {
  /** Pass --force (needed for breaking recreate; UI has no TTY confirm). */
  force?: boolean;
  /** Pass --purge with down (destroy managed VMs/nets/…; keep project files). */
  purge?: boolean;
};

export async function runVzctl(
  path: string,
  command: VzctlCommand,
  options: RunVzctlOptions = {},
): Promise<string> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("run_vzctl", {
    path,
    command,
    force: options.force ?? false,
    purge: options.purge ?? false,
  });
}

export async function runVzctlArgv(args: string[]): Promise<string> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("run_vzctl_argv", { args });
}

export type VzctlEnvelope = {
  apiVersion?: string;
  command?: string;
  status?: string;
  exit_code?: number;
  summary?: Record<string, unknown>;
  [key: string]: unknown;
};

export function parseEnvelope(raw: string): VzctlEnvelope {
  const value = JSON.parse(raw) as unknown;
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("vzctl envelope is not an object");
  }
  return value as VzctlEnvelope;
}

export function assertEnvelopeOk(envelope: VzctlEnvelope, fallback = "vzctl failed"): void {
  const exit = envelope.exit_code;
  if (envelope.status === "fail" || (typeof exit === "number" && exit !== 0)) {
    const summary =
      envelope.summary && typeof envelope.summary === "object"
        ? (envelope.summary as Record<string, unknown>).message
        : undefined;
    const message =
      (typeof summary === "string" && summary) ||
      (typeof envelope.message === "string" && envelope.message) ||
      fallback;
    throw new Error(message);
  }
}

export async function subscribeEvents(): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("subscribe_events");
}

export async function pickEnvironment(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Open vzctl Environment",
  });
  if (!selected) return null;
  return typeof selected === "string" ? selected : String(selected);
}

export function basename(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export const queryKeys = {
  status: (path: string) => ["vzctl", "status", path] as const,
  diff: (path: string) => ["vzctl", "diff", path] as const,
};
