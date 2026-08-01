import { invoke } from "@tauri-apps/api/core";
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

async function isTauri(): Promise<boolean> {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function readText(path: string): Promise<string> {
  if (await isTauri()) {
    return invoke<string>("read_text_file", { path });
  }
  throw new Error("Dateizugriff nur in der Tauri-App");
}

async function writeText(path: string, contents: string): Promise<void> {
  if (await isTauri()) {
    await invoke("write_text_file", { path, contents });
    return;
  }
  throw new Error("Dateizugriff nur in der Tauri-App");
}

async function ensureDir(path: string): Promise<void> {
  if (await isTauri()) {
    await invoke("ensure_dir", { path });
    return;
  }
  throw new Error("Dateizugriff nur in der Tauri-App");
}

async function exists(path: string): Promise<boolean> {
  if (await isTauri()) {
    return invoke<boolean>("path_exists", { path });
  }
  return false;
}

export async function loadProject(projectDir: string): Promise<{
  env: Environment;
  diagram: DiagramState;
}> {
  const yaml = await readText(configPath(projectDir));
  const env = parseEnvironmentYaml(yaml);
  let diagramRaw: string | null = null;
  try {
    if (await exists(diagramPath(projectDir))) {
      diagramRaw = await readText(diagramPath(projectDir));
    }
  } catch {
    diagramRaw = null;
  }
  return { env, diagram: parseDiagramState(diagramRaw) };
}

export async function saveProject(
  projectDir: string,
  env: Environment,
  diagram: DiagramState,
): Promise<void> {
  await writeText(configPath(projectDir), serializeEnvironmentYaml(env));
  const diagDir = diagramPath(projectDir).replace(/[/\\][^/\\]+$/, "");
  await ensureDir(diagDir);
  await writeText(diagramPath(projectDir), serializeDiagramState(diagram));
}

export async function createProject(
  projectDir: string,
  name: string,
): Promise<{ env: Environment; diagram: DiagramState }> {
  await ensureDir(projectDir);
  const cfg = configPath(projectDir);
  if (await exists(cfg)) {
    throw new Error("Verzeichnis enthält bereits hypernetwork.config.yaml");
  }
  const files = scaffoldFiles({ name });
  await saveProject(projectDir, files.env, files.diagram);
  return { env: files.env, diagram: files.diagram };
}

/** Browser/dev fallback: sessionStorage so navigation keeps state. */
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

export async function loadProjectFlexible(projectDir: string): Promise<{
  env: Environment;
  diagram: DiagramState;
}> {
  try {
    return await loadProject(projectDir);
  } catch (err) {
    const mem = readMemory().get(projectDir);
    if (mem) return mem;
    throw err;
  }
}

export async function saveProjectFlexible(
  projectDir: string,
  env: Environment,
  diagram: DiagramState,
): Promise<void> {
  try {
    await saveProject(projectDir, env, diagram);
  } catch {
    const map = readMemory();
    map.set(projectDir, { env, diagram });
    writeMemory(map);
  }
}

export async function createProjectFlexible(
  projectDir: string,
  name: string,
): Promise<{ env: Environment; diagram: DiagramState }> {
  try {
    return await createProject(projectDir, name);
  } catch (err) {
    if (String(err).includes("nur in der Tauri-App")) {
      const files = scaffoldFiles({ name });
      const map = readMemory();
      map.set(projectDir, { env: files.env, diagram: files.diagram });
      writeMemory(map);
      return { env: files.env, diagram: files.diagram };
    }
    throw err;
  }
}
