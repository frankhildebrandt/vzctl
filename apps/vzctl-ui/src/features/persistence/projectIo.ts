import { isDemoMode } from "@/lib/demo";
import { ApiError, api, encodeId } from "@/lib/api";
import {
  configPath,
  diagramPath,
  parseDiagramState,
  parseEnvironmentYaml,
  serializeDiagramState,
  serializeEnvironmentYaml,
} from "@/application/services/yaml";
import type { Environment } from "@/domain/hypernetwork/schema";
import type { DiagramState } from "@/domain/diagram/types";
import { scaffoldFiles } from "@/application/services/scaffold";
import { ensureStackId } from "@/lib/vzctl";

const MEMORY_KEY = "vzctl.ui.topology.memory.v1";

type MemoryEntry = { env: Environment; diagram: DiagramState };

function readMemory(): Map<string, MemoryEntry> {
  if (typeof sessionStorage === "undefined") return new Map();
  try {
    const raw = sessionStorage.getItem(MEMORY_KEY);
    if (!raw) return new Map();
    const obj = JSON.parse(raw) as Record<string, MemoryEntry>;
    return new Map(Object.entries(obj));
  } catch {
    return new Map();
  }
}

function writeMemory(map: Map<string, MemoryEntry>): void {
  if (typeof sessionStorage === "undefined") return;
  const obj = Object.fromEntries(map.entries());
  sessionStorage.setItem(MEMORY_KEY, JSON.stringify(obj));
}

export async function loadProject(projectDir: string): Promise<{
  env: Environment;
  diagram: DiagramState;
}> {
  if (isDemoMode()) {
    const mem = readMemory().get(projectDir);
    if (mem) return mem;
    const files = scaffoldFiles({ name: "demo", cidr: "10.80.0.0/24" });
    return { env: files.env, diagram: files.diagram };
  }
  const stackId = await ensureStackId(projectDir);
  const yaml = await api.getText(`/v1/stacks/${encodeId(stackId)}/config`);
  const env = parseEnvironmentYaml(yaml);
  let diagramRaw: string | null = null;
  try {
    diagramRaw = await api.getText(`/v1/stacks/${encodeId(stackId)}/diagram`);
  } catch (error) {
    // A missing optional sidecar is fine; outages/permission failures must not
    // become an empty diagram that the next save would overwrite.
    // Older supervisors returned 500/internal for this exact missing-file case.
    const legacyMissing = error instanceof ApiError && error.status === 500
      && error.code === "internal" && error.message === `not found: ${diagramPath(projectDir)}`;
    if (!(error instanceof ApiError) || (error.status !== 404 && !legacyMissing)) throw error;
  }
  return { env, diagram: parseDiagramState(diagramRaw) };
}

export async function saveProject(
  projectDir: string,
  env: Environment,
  diagram: DiagramState,
): Promise<void> {
  if (isDemoMode()) {
    const map = readMemory();
    map.set(projectDir, { env, diagram });
    writeMemory(map);
    return;
  }
  const stackId = await ensureStackId(projectDir);
  await api.putText(
    `/v1/stacks/${encodeId(stackId)}/config`,
    serializeEnvironmentYaml(env),
    "text/yaml; charset=utf-8",
  );
  await api.putText(
    `/v1/stacks/${encodeId(stackId)}/diagram`,
    serializeDiagramState(diagram),
    "application/json; charset=utf-8",
  );
}

export async function createProject(
  projectDir: string,
  name: string,
): Promise<{ env: Environment; diagram: DiagramState }> {
  await api.post("/v1/stacks", {
    path: projectDir,
    name,
    create: true,
  });
  const files = scaffoldFiles({ name });
  // Ensure scaffold content is written (create:true may only write minimal yaml).
  await saveProject(projectDir, files.env, files.diagram);
  return { env: files.env, diagram: files.diagram };
}

export async function loadProjectFlexible(projectDir: string): Promise<{
  env: Environment;
  diagram: DiagramState;
}> {
  return loadProject(projectDir);
}

export async function saveProjectFlexible(
  projectDir: string,
  env: Environment,
  diagram: DiagramState,
): Promise<void> {
  // Only explicit demo mode may save in memory (handled by saveProject).
  // A failed real write must reach the editor, which keeps the dirty state.
  return saveProject(projectDir, env, diagram);
}

export async function createProjectFlexible(
  projectDir: string,
  name: string,
): Promise<{ env: Environment; diagram: DiagramState }> {
  if (isDemoMode()) {
    const files = scaffoldFiles({ name });
    await saveProject(projectDir, files.env, files.diagram);
    return { env: files.env, diagram: files.diagram };
  }
  return createProject(projectDir, name);
}

// Keep path helpers available for orphan recovery / diagnostics.
export { configPath, diagramPath };
