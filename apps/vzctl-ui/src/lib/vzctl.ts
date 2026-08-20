import { api, encodeId } from "@/lib/api";

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
  "ensure_docker_project_mount",
  "ensure_ca",
  "ensure_oidc",
  "ensure_ingress",
  "ensure_ca_rollout",
  "ensure_oidc_inject",
  "ensure_docker_context",
  "ensure_containers",
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
  force?: boolean;
  purge?: boolean;
  resume?: boolean;
  abort?: boolean;
};

export type VzctlEnvelope = {
  apiVersion?: string;
  command?: string;
  status?: string;
  exit_code?: number;
  summary?: Record<string, unknown>;
  [key: string]: unknown;
};

export function parseEnvelope(raw: string | unknown): VzctlEnvelope {
  if (typeof raw !== "string") {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      throw new Error("vzctl envelope is not an object");
    }
    return raw as VzctlEnvelope;
  }
  const trimmed = raw.trim();
  let value: unknown;
  try {
    value = JSON.parse(trimmed) as unknown;
  } catch (first) {
    const start = trimmed.indexOf("{");
    const end = trimmed.lastIndexOf("}");
    if (start < 0 || end <= start) throw first;
    value = JSON.parse(trimmed.slice(start, end + 1)) as unknown;
  }
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

export type JobResponse = {
  jobId: string;
  kind?: string;
  status: string;
  result?: unknown;
  error?: string;
  log?: string[];
  progress?: { percent?: number; label?: string };
};

export type WaitForJobOptions = {
  onUpdate?: (job: JobResponse) => void;
};

export async function ensureStackId(path: string): Promise<string> {
  const listed = await api.get<{ stacks: Array<{ id: string; path: string }> }>("/v1/stacks");
  const found = (listed.stacks ?? []).find((s) => s.path === path);
  if (found) return found.id;
  const created = await api.post<{ id: string }>("/v1/stacks", { path });
  return created.id;
}

export async function waitForJob(
  jobId: string,
  options: WaitForJobOptions = {},
): Promise<VzctlEnvelope> {
  for (let i = 0; i < 3_600; i += 1) {
    const job = await api.get<JobResponse>(`/v1/jobs/${encodeId(jobId)}`);
    options.onUpdate?.(job);
    if (job.status === "succeeded") {
      return parseEnvelope(job.result ?? { status: "ok", exit_code: 0 });
    }
    if (job.status === "failed") {
      if (job.result) {
        const envelope = parseEnvelope(job.result);
        assertEnvelopeOk(envelope, job.error ?? "job failed");
      }
      throw new Error(job.error ?? "job failed");
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`job ${jobId} timed out`);
}

export async function runVzctl(
  path: string,
  command: VzctlCommand,
  options: RunVzctlOptions = {},
): Promise<string> {
  try {
    return await runVzctlOnce(path, command, options);
  } catch (err) {
    const message = String(err);

    if (
      isResolverPermissionError(message) &&
      (command === "apply" || command === "up" || command === "down") &&
      !options.resume &&
      !options.abort
    ) {
      await ensureHostResolver(path, command, options.purge === true);
      try {
        await runVzctlOnce(path, "apply", { abort: true });
      } catch {
        // no incomplete journal — fine
      }
      return await runVzctlOnce(path, command, options);
    }

    const canAutoAbort =
      (command === "apply" || command === "up") &&
      !options.resume &&
      !options.abort &&
      isIncompleteJournalError(message);
    if (!canAutoAbort) throw err;

    // Match pre-REST Tauri recovery: clear failed/incomplete journal, then retry.
    await runVzctlOnce(path, "apply", { abort: true });
    return await runVzctlOnce(path, command, options);
  }
}

function isIncompleteJournalError(raw: string): boolean {
  return (
    raw.includes("incomplete journal") ||
    (raw.includes("--resume") && raw.includes("--abort"))
  );
}

function isResolverPermissionError(raw: string): boolean {
  return (
    raw.includes("Permission denied") ||
    raw.includes("run this command with sudo") ||
    raw.includes("os error 13") ||
    (raw.includes("/etc/resolver") &&
      (raw.includes("write ") || raw.includes("create directory") || raw.includes("remove ")))
  );
}

/** Install/uninstall macOS `/etc/resolver` via REST (Tauri elevates when needed). */
async function ensureHostResolver(
  path: string,
  command: VzctlCommand,
  purge: boolean,
): Promise<void> {
  if (command === "down" && purge) {
    const envelope = parseEnvelope(
      await api.delete(`/v1/dns/resolver?config=${encodeURIComponent(path)}`),
    );
    assertEnvelopeOk(envelope, "dns uninstall-resolver failed");
    return;
  }
  if (command === "down") {
    return;
  }
  const envelope = parseEnvelope(
    await api.post("/v1/dns/resolver", { config: path }),
  );
  assertEnvelopeOk(envelope, "dns install-resolver failed");
}

async function runVzctlOnce(
  path: string,
  command: VzctlCommand,
  options: RunVzctlOptions = {},
): Promise<string> {
  const stackId = await ensureStackId(path);
  const encoded = encodeId(stackId);

  if (command === "status") {
    const status = await api.get(`/v1/stacks/${encoded}/status`);
    return JSON.stringify(status, null, 2);
  }
  if (command === "diff") {
    const diff = await api.get(`/v1/stacks/${encoded}/diff`);
    return JSON.stringify(diff, null, 2);
  }
  if (command === "validate") {
    const result = await api.post(`/v1/stacks/${encoded}/validate`);
    return JSON.stringify(result, null, 2);
  }

  const body: Record<string, boolean> = {};
  if (options.force) body.force = true;
  if (options.purge) body.purge = true;
  if (options.resume) body.resume = true;
  if (options.abort) body.abort = true;
  const accepted = await api.post<{ jobId: string }>(
    `/v1/stacks/${encoded}/${command}`,
    body,
  );
  const envelope = await waitForJob(accepted.jobId);
  assertEnvelopeOk(envelope, `${command} failed`);
  return JSON.stringify(envelope, null, 2);
}

/** @deprecated Prefer domain helpers; kept for gradual migration. */
export async function runVzctlArgv(args: string[]): Promise<string> {
  // Map common argv patterns to REST for leftover call sites.
  const [cmd, sub, ...rest] = args;
  if (cmd === "doctor") {
    return JSON.stringify(await api.get("/v1/doctor"), null, 2);
  }
  if (cmd === "vm" && sub === "list") {
    return JSON.stringify(await api.get("/v1/vms"), null, 2);
  }
  if (cmd === "vm" && sub === "inspect" && rest[0]) {
    return JSON.stringify(await api.get(`/v1/vms/${encodeId(rest[0])}`), null, 2);
  }
  if (cmd === "image" && sub === "list") {
    return JSON.stringify(await api.get("/v1/images"), null, 2);
  }
  if (cmd === "dns" && sub === "status") {
    return JSON.stringify(await api.get("/v1/dns/status"), null, 2);
  }
  if (cmd === "dns" && sub === "install-bind-helper") {
    return JSON.stringify(await api.post("/v1/dns/bind-helper"), null, 2);
  }
  if (cmd === "dns" && (sub === "install-resolver" || sub === "uninstall-resolver")) {
    const configIdx = rest.indexOf("--config");
    const projectIdx = rest.indexOf("--project");
    const body: Record<string, string> = {};
    if (configIdx >= 0 && rest[configIdx + 1]) body.config = rest[configIdx + 1];
    if (projectIdx >= 0 && rest[projectIdx + 1]) body.project = rest[projectIdx + 1];
    if (sub === "uninstall-resolver") {
      const qs = new URLSearchParams(body).toString();
      return JSON.stringify(
        await api.delete(qs ? `/v1/dns/resolver?${qs}` : "/v1/dns/resolver"),
        null,
        2,
      );
    }
    return JSON.stringify(await api.post("/v1/dns/resolver", body), null, 2);
  }
  if (cmd === "certs" && sub === "fingerprint") {
    return JSON.stringify(await api.get("/v1/certs/fingerprint"), null, 2);
  }
  if (cmd === "certs" && sub === "ca" && rest[0] === "init") {
    return JSON.stringify(await api.post("/v1/certs/ca/init"), null, 2);
  }
  if (cmd === "certs" && sub === "ca" && rest[0] === "install") {
    return JSON.stringify(await api.post("/v1/certs/ca/install"), null, 2);
  }
  throw new Error(`runVzctlArgv unsupported after REST migration: ${args.join(" ")}`);
}

export async function subscribeEvents(): Promise<void> {
  const { isDemoMode } = await import("@/lib/demo");
  if (isDemoMode()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("subscribe_events");
}

export async function pickEnvironment(): Promise<string | null> {
  const { isDemoMode, DEMO_PROJECT_PATH } = await import("@/lib/demo");
  if (isDemoMode()) return DEMO_PROJECT_PATH;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const { getT } = await import("@/lib/i18n");
  const selected = await open({
    directory: true,
    multiple: false,
    title: getT()("dialog.openEnvironment"),
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
