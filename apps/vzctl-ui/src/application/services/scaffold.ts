import type { Environment } from "@/domain/hypernetwork/schema";
import { emptyDiagramState, type DiagramState } from "@/domain/diagram/types";
import {
  serializeDiagramState,
  serializeEnvironmentYaml,
} from "@/application/services/yaml";

export type ScaffoldOptions = {
  name: string;
  /** Optional first network CIDR. */
  cidr?: string;
};

export function scaffoldEnvironment(options: ScaffoldOptions): Environment {
  const name = options.name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 63);
  if (!name) {
    throw new Error("Projektname darf nicht leer sein");
  }
  const cidr = options.cidr ?? "10.80.0.0/24";
  return {
    apiVersion: "hypernetwork/v1",
    kind: "Environment",
    metadata: { name },
    spec: {
      project: name,
      domain: `${name}.vz.test`,
      dns: {
        enabled: true,
        hostResolver: true,
        hostListen: "127.0.0.1:15353",
        forward: { enabled: true, upstream: "system" },
      },
      images: {
        "ubuntu-base": { from: "ubuntu-latest", role: "base" },
      },
      networks: {
        lan: { cidr, mode: "shared", dhcp: false, natEgress: true },
      },
      routes: [],
      policies: [],
      ports: [],
      volumes: {},
      vms: {},
    },
  };
}

export function scaffoldDiagramForEnv(env: Environment): DiagramState {
  const diagram = emptyDiagramState();
  let x = 80;
  const yNet = 80;
  for (const name of Object.keys(env.spec.networks)) {
    diagram.nodes[`net:${name}`] = { x, y: yNet, width: 200, height: 88 };
    x += 260;
  }
  let vx = 80;
  const yVm = 280;
  for (const name of Object.keys(env.spec.vms)) {
    diagram.nodes[`vm:${name}`] = { x: vx, y: yVm, width: 220, height: 120 };
    vx += 260;
  }
  return diagram;
}

export function scaffoldFiles(options: ScaffoldOptions): {
  env: Environment;
  diagram: DiagramState;
  yaml: string;
  diagramJson: string;
} {
  const env = scaffoldEnvironment(options);
  const diagram = scaffoldDiagramForEnv(env);
  return {
    env,
    diagram,
    yaml: serializeEnvironmentYaml(env),
    diagramJson: serializeDiagramState(diagram),
  };
}
