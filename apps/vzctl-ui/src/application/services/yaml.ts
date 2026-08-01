import { parse, stringify } from "yaml";
import {
  EnvironmentSchema,
  type Environment,
} from "@/domain/hypernetwork/schema";
import {
  DiagramStateSchema,
  emptyDiagramState,
  type DiagramState,
} from "@/domain/diagram/types";

export function parseEnvironmentYaml(raw: string): Environment {
  const data = parse(raw) as unknown;
  const result = EnvironmentSchema.safeParse(data);
  if (!result.success) {
    const msg = result.error.issues
      .slice(0, 5)
      .map((i) => `${i.path.join(".")}: ${i.message}`)
      .join("; ");
    throw new Error(`Ungültige hypernetwork.config.yaml: ${msg}`);
  }
  return result.data;
}

/** Stable key order for readable diffs. */
export function serializeEnvironmentYaml(env: Environment): string {
  const ordered = {
    apiVersion: env.apiVersion,
    kind: env.kind,
    metadata: { name: env.metadata.name },
    spec: {
      project: env.spec.project,
      domain: env.spec.domain,
      dns: env.spec.dns,
      images: env.spec.images,
      networks: env.spec.networks,
      routes: env.spec.routes,
      policies: env.spec.policies,
      ...(Object.keys(env.spec.volumes).length > 0
        ? { volumes: env.spec.volumes }
        : {}),
      ...(env.spec.ports.length > 0 ? { ports: env.spec.ports } : {}),
      ...(env.spec.certs !== undefined ? { certs: env.spec.certs } : {}),
      ...(env.spec.ingress !== undefined ? { ingress: env.spec.ingress } : {}),
      ...(env.spec.oidc !== undefined ? { oidc: env.spec.oidc } : {}),
      vms: env.spec.vms,
    },
  };
  return stringify(ordered, {
    lineWidth: 100,
  });
}

export function parseDiagramState(raw: string | null): DiagramState {
  if (!raw) return emptyDiagramState();
  try {
    const data = JSON.parse(raw) as unknown;
    const result = DiagramStateSchema.safeParse(data);
    if (!result.success) return emptyDiagramState();
    return result.data;
  } catch {
    return emptyDiagramState();
  }
}

export function serializeDiagramState(state: DiagramState): string {
  return `${JSON.stringify(state, null, 2)}\n`;
}

export const CONFIG_FILENAME = "hypernetwork.config.yaml";
export const DIAGRAM_RELATIVE_PATH = ".vzctl/diagram.json";

export function configPath(projectDir: string): string {
  return joinPath(projectDir, CONFIG_FILENAME);
}

export function diagramPath(projectDir: string): string {
  return joinPath(projectDir, DIAGRAM_RELATIVE_PATH);
}

function joinPath(base: string, rel: string): string {
  const sep = base.includes("\\") && !base.includes("/") ? "\\" : "/";
  return `${base.replace(/[/\\]+$/, "")}${sep}${rel.replace(/^[/\\]+/, "")}`;
}
