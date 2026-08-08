import { scaffoldFiles } from "@/application/services/scaffold";
import { rememberProject } from "@/lib/projects";

export const DEMO_FLAG_KEY = "vzctl.ui.demo.v1";
export const DEMO_PROJECT_PATH = "/demo/edge-dmz";
export const DEMO_PROJECT_NAME = "edge-dmz";

const MEMORY_KEY = "vzctl.ui.topology.memory.v1";

export function isDemoMode(): boolean {
  if (typeof window === "undefined") return false;
  try {
    if (sessionStorage.getItem(DEMO_FLAG_KEY) === "1") return true;
  } catch {
    // ignore
  }
  const params = new URLSearchParams(window.location.search);
  if (params.get("demo") === "1") return true;
  const path = window.location.pathname;
  return path === "/demo" || path.startsWith("/demo/");
}

export function enableDemoMode(): void {
  try {
    sessionStorage.setItem(DEMO_FLAG_KEY, "1");
  } catch {
    // ignore
  }
  seedDemoProject();
}

export function disableDemoMode(): void {
  try {
    sessionStorage.removeItem(DEMO_FLAG_KEY);
  } catch {
    // ignore
  }
}

function seedDemoProject(): void {
  void rememberProject(DEMO_PROJECT_PATH);

  const files = scaffoldFiles({ name: DEMO_PROJECT_NAME, cidr: "10.80.0.0/24" });
  // Enrich scaffold so Topology/Ops look closer to edge-dmz.
  files.env.spec.networks = {
    dmz: {
      cidr: "10.80.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: true,
      backend: "vmnet",
    },
    lan: {
      cidr: "10.90.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: true,
      backend: "vmnet",
    },
    containers: {
      cidr: "10.95.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: false,
      backend: "docker",
    },
  };
  files.env.spec.vms = {
    router: {
      from: "ubuntu-base",
      clone: "linked",
      disk: "4G",
      networks: [
        { name: "dmz", ip: "10.80.0.2" },
        { name: "lan", ip: "10.90.0.2" },
      ],
      dependsOn: [],
      roles: ["router"],
      requires: [],
      ports: [],
      mounts: [],
    },
    web: {
      from: "ubuntu-base",
      clone: "linked",
      disk: "40G",
      networks: [{ name: "dmz", ip: "10.80.0.10" }],
      dependsOn: ["router"],
      roles: [],
      requires: ["oidc"],
      ports: [],
      mounts: [],
    },
    docker: {
      from: "ubuntu-base",
      clone: "linked",
      disk: "100G",
      networks: [
        { name: "lan", ip: "10.90.0.10" },
        { name: "containers", ip: "10.95.0.2" },
      ],
      dependsOn: ["router"],
      roles: ["docker", "router"],
      requires: [],
      ports: [],
      mounts: [],
    },
    host: {
      from: "ubuntu-base",
      clone: "linked",
      disk: "20G",
      cpus: 2,
      memory: "2048MiB",
      networks: [{ name: "lan", ip: "10.90.0.11" }],
      dependsOn: [],
      roles: [],
      requires: [],
      ports: [],
      mounts: [],
    },
  };
  files.env.spec.ingress = {
    enabled: true,
    bind: "127.0.0.1",
    hostAliases: true,
    redirectHttp: true,
    routes: [
      {
        host: "web.svc.edge-dmz.vz.test",
        to: "web:80",
        requires: ["oidc"],
      },
      {
        host: "auth.svc.edge-dmz.vz.test",
        to: "oidc:5556",
      },
    ],
  };
  files.env.spec.oidc = {
    enabled: true,
    mode: "oidc-simple",
    issuer: "https://auth.svc.edge-dmz.vz.test",
    listen: "127.0.0.1:5556",
    clients: "auto",
    users: [
      { username: "alice", email: "alice@dev.local", role: "admin" },
      { username: "bob", email: "bob@dev.local" },
      { username: "charlie", email: "charlie@dev.local" },
    ],
  };
  files.env.spec.certs = { enabled: true, onRotate: "reinject" };

  // Simple diagram positions for the three nets + four VMs.
  let x = 80;
  for (const name of Object.keys(files.env.spec.networks)) {
    files.diagram.nodes[`net:${name}`] = {
      x,
      y: 80,
      width: 200,
      height: 88,
    };
    x += 240;
  }
  x = 80;
  for (const name of Object.keys(files.env.spec.vms)) {
    files.diagram.nodes[`vm:${name}`] = {
      x,
      y: 240,
      width: 180,
      height: 96,
    };
    x += 220;
  }

  try {
    const raw = sessionStorage.getItem(MEMORY_KEY);
    const map = raw
      ? (JSON.parse(raw) as Record<string, unknown>)
      : ({} as Record<string, unknown>);
    map[DEMO_PROJECT_PATH] = { env: files.env, diagram: files.diagram };
    sessionStorage.setItem(MEMORY_KEY, JSON.stringify(map));
  } catch {
    // ignore
  }
}
