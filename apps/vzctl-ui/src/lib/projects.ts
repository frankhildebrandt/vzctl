import { basename } from "@/lib/vzctl";

const STORAGE_KEY = "vzctl.ui.projects.v1";

export type Project = {
  path: string;
  name: string;
  openedAt: number;
};

export const projectKeys = {
  all: ["projects"] as const,
};

export function listProjects(): Project[] {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(isProject)
      .sort((a, b) => b.openedAt - a.openedAt);
  } catch {
    return [];
  }
}

export function rememberProject(path: string): Project[] {
  const now = Date.now();
  const existing = listProjects().filter((project) => project.path !== path);
  const next: Project[] = [
    { path, name: basename(path), openedAt: now },
    ...existing,
  ];
  save(next);
  return next;
}

export function forgetProject(path: string): Project[] {
  const next = listProjects().filter((project) => project.path !== path);
  save(next);
  return next;
}

export function getProject(path: string): Project | undefined {
  return listProjects().find((project) => project.path === path);
}

function save(projects: Project[]) {
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

export function formatOpenedAt(openedAt: number): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(openedAt));
  } catch {
    return new Date(openedAt).toLocaleString();
  }
}
