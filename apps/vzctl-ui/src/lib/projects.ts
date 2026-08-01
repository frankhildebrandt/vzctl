import { api, encodeId } from "@/lib/api";
import { localeToBcp47 } from "@/lib/i18n/detect";
import type { LocaleId } from "@/lib/i18n/types";
import { useSettingsStore } from "@/store/settingsStore";
import { basename } from "@/lib/vzctl";

const STORAGE_KEY = "vzctl.ui.projects.v1";

export type Project = {
  path: string;
  name: string;
  openedAt: number;
  id?: string;
};

export const projectKeys = {
  all: ["projects"] as const,
};

function readLocal(): Project[] {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isProject).sort((a, b) => b.openedAt - a.openedAt);
  } catch {
    return [];
  }
}

function writeLocal(projects: Project[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(projects));
}

function isProject(value: unknown): value is Project {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.path === "string" &&
    record.path.length > 0 &&
    typeof record.name === "string" &&
    typeof record.openedAt === "number"
  );
}

function openedAtMs(value: string | number | undefined): number {
  if (typeof value === "number") return value;
  if (typeof value === "string") {
    const parsed = Date.parse(value);
    if (!Number.isNaN(parsed)) return parsed;
  }
  return Date.now();
}

export async function listProjects(): Promise<Project[]> {
  try {
    const data = await api.get<{
      stacks: Array<{ id: string; path: string; name: string; openedAt: string }>;
    }>("/v1/stacks");
    const projects = (data.stacks ?? [])
      .map((row) => ({
        id: row.id,
        path: row.path,
        name: row.name || basename(row.path),
        openedAt: openedAtMs(row.openedAt),
      }))
      .sort((a, b) => b.openedAt - a.openedAt);
    writeLocal(projects);
    return projects;
  } catch {
    return readLocal();
  }
}

export async function rememberProject(path: string, name?: string): Promise<Project[]> {
  const displayName = name ?? basename(path);
  try {
    await api.post("/v1/stacks", { path, name: displayName });
  } catch {
    // Keep local MRU even if supervisor is down.
  }
  const now = Date.now();
  const existing = readLocal().filter((project) => project.path !== path);
  writeLocal([{ path, name: displayName, openedAt: now }, ...existing]);
  return listProjects();
}

export async function forgetProject(path: string): Promise<Project[]> {
  const local = readLocal();
  const match = local.find((p) => p.path === path);
  if (match?.id) {
    try {
      await api.delete(`/v1/stacks/${encodeId(match.id)}`);
    } catch {
      // ignore
    }
  } else {
    try {
      const remote = await listProjects();
      const found = remote.find((p) => p.path === path);
      if (found?.id) await api.delete(`/v1/stacks/${encodeId(found.id)}`);
    } catch {
      // ignore
    }
  }
  writeLocal(local.filter((project) => project.path !== path));
  return listProjects();
}

/** Sync lookup from local MRU cache (for breadcrumbs / titles). */
export function getProject(path: string): Project | undefined {
  return readLocal().find((project) => project.path === path);
}

export function formatOpenedAt(
  openedAt: number,
  locale: LocaleId = useSettingsStore.getState().locale,
): string {
  try {
    return new Intl.DateTimeFormat(localeToBcp47(locale), {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(openedAt));
  } catch {
    return new Date(openedAt).toLocaleString(localeToBcp47(locale));
  }
}
