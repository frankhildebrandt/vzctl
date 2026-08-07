import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import {
  parseEnvironmentYaml,
  serializeEnvironmentYaml,
} from "@/application/services/yaml";
import {
  cidrsOverlap,
  ipInCidr,
  parseCidr,
  validateEnvironment,
} from "@/application/validation/topology";
import { validateConnection } from "@/application/validation/connection";
import { scaffoldEnvironment } from "@/application/services/scaffold";
import {
  applyAttachNic,
  applyAssignPrimaryNetwork,
  applyCreateNetwork,
  applyCreateVm,
  applyDeleteNodes,
  applyRenameNetwork,
  applyRenameVm,
  applyReassignNic,
  emptySnapshot,
} from "./_helpers";

const root = join(dirname(fileURLToPath(import.meta.url)), "../../../..");

describe("cidr / ip", () => {
  it("parses cidr", () => {
    expect(parseCidr("10.80.0.0/24")).toMatchObject({ prefix: 24 });
  });

  it("checks membership", () => {
    expect(ipInCidr("10.80.0.10", "10.80.0.0/24")).toBe(true);
    expect(ipInCidr("10.90.0.10", "10.80.0.0/24")).toBe(false);
  });

  it("detects overlap", () => {
    expect(cidrsOverlap("10.80.0.0/24", "10.80.0.0/16")).toBe(true);
    expect(cidrsOverlap("10.80.0.0/24", "10.90.0.0/24")).toBe(false);
  });
});

describe("yaml round-trip", () => {
  it("parses edge-dmz example", () => {
    const raw = readFileSync(
      join(root, "examples/edge-dmz/hypernetwork.config.yaml"),
      "utf8",
    );
    const env = parseEnvironmentYaml(raw);
    expect(env.metadata.name).toBe("edge-dmz");
    expect(env.spec.networks.dmz?.cidr).toBe("10.80.1.0/24");
    expect(env.spec.networks.containers?.backend).toBe("docker");
    expect(env.spec.vms.docker?.roles).toEqual(
      expect.arrayContaining(["docker", "router"]),
    );
    expect(env.spec.vms.router?.roles).toContain("router");
    const yaml = serializeEnvironmentYaml(env);
    const again = parseEnvironmentYaml(yaml);
    expect(again.spec.networks.containers?.backend).toBe("docker");
  });

  it("round-trips scaffold", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    const yaml = serializeEnvironmentYaml(env);
    const again = parseEnvironmentYaml(yaml);
    expect(again.spec.project).toBe("lab");
    expect(again.spec.networks.lan).toBeTruthy();
    expect(again.spec.resilience?.network.restartVMsOnStuckEgress).toBe(false);
    expect(yaml).toContain("resilience:");
  });

  it("rejects unsafe egress probe URLs", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    env.spec.resilience!.network.egressProbe.url = "https://secret@example.com/";
    expect(() => parseEnvironmentYaml(serializeEnvironmentYaml(env))).toThrow(
      "keine Zugangsdaten",
    );
  });
});

describe("connection validation", () => {
  it("allows vm↔network", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    env.spec.vms.web = {
      from: "ubuntu-base",
      clone: "linked",
      dataDisk: "20G",
      networks: [{ name: "lan", ip: "10.80.0.10" }],
      dependsOn: [],
      roles: [],
      requires: [],
      ports: [],
      mounts: [],
    };
    const result = validateConnection({
      source: { nodeId: "vm:web", portId: "nic:web:new" },
      target: { nodeId: "net:lan", portId: "attach:lan" },
      existingAttachments: new Set(),
      existingRoutes: new Set(),
      env,
    });
    expect(result.ok).toBe(true);
  });

  it("rejects duplicate attachment", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    env.spec.vms.web = {
      from: "ubuntu-base",
      clone: "linked",
      dataDisk: "20G",
      networks: [{ name: "lan", ip: "10.80.0.10" }],
      dependsOn: [],
      roles: [],
      requires: [],
      ports: [],
      mounts: [],
    };
    const result = validateConnection({
      source: { nodeId: "vm:web", portId: "nic:web:new" },
      target: { nodeId: "net:lan", portId: "attach:lan" },
      existingAttachments: new Set(["web|lan"]),
      existingRoutes: new Set(),
      env,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects non-attach ports", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    env.spec.vms.web = {
      from: "ubuntu-base",
      clone: "linked",
      dataDisk: "20G",
      networks: [{ name: "lan", ip: "10.80.0.10" }],
      dependsOn: [],
      roles: [],
      requires: [],
      ports: [],
      mounts: [],
    };
    const result = validateConnection({
      source: { nodeId: "vm:web", portId: "nic:web:new" },
      target: { nodeId: "net:lan", portId: "uplink:lan" },
      existingAttachments: new Set(),
      existingRoutes: new Set(),
      env,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects self-connect", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    const result = validateConnection({
      source: { nodeId: "net:lan", portId: "a" },
      target: { nodeId: "net:lan", portId: "b" },
      existingAttachments: new Set(),
      existingRoutes: new Set(),
      env,
    });
    expect(result.ok).toBe(false);
  });
});

describe("topology validation", () => {
  it("flags IP outside cidr", () => {
    const env = scaffoldEnvironment({ name: "lab" });
    env.spec.vms.web = {
      from: "ubuntu-base",
      clone: "linked",
      dataDisk: "20G",
      networks: [{ name: "lan", ip: "192.168.1.10" }],
      dependsOn: [],
      roles: [],
      requires: [],
      ports: [],
      mounts: [],
    };
    const issues = validateEnvironment(env);
    expect(issues.some((i) => i.code === "IP_OUT_OF_CIDR")).toBe(true);
  });
});

describe("commands", () => {
  it("creates network and vm, attaches, deletes with cascade", () => {
    let snap = emptySnapshot();
    snap = applyCreateNetwork(snap, "dmz", "10.80.0.0/24", "shared", {
      x: 0,
      y: 0,
    });
    snap = applyCreateVm(snap, "web", {
      networkName: "dmz",
      position: { x: 40, y: 200 },
    });
    snap = applyCreateNetwork(snap, "lan", "10.90.0.0/24", "shared", {
      x: 300,
      y: 0,
    });
    snap = applyAttachNic(snap, "web", "lan");
    expect(snap.env.spec.vms.web?.networks).toHaveLength(2);
    snap = applyDeleteNodes(snap, ["net:lan"]);
    expect(snap.env.spec.networks.lan).toBeUndefined();
    expect(snap.env.spec.vms.web?.networks.every((n) => n.name !== "lan")).toBe(
      true,
    );
  });

  it("renames vm and network with references", () => {
    let snap = emptySnapshot();
    snap = applyCreateNetwork(snap, "dmz", "10.80.0.0/24", "shared", {
      x: 0,
      y: 0,
    });
    snap = applyCreateVm(snap, "web", {
      networkName: "dmz",
      roles: ["router"],
      position: { x: 0, y: 100 },
    });
    snap = applyRenameVm(snap, "web", "edge");
    expect(snap.env.spec.vms.edge).toBeTruthy();
    expect(snap.env.spec.vms.web).toBeUndefined();
    expect(snap.diagram.nodes["vm:edge"]).toBeTruthy();

    snap = applyRenameNetwork(snap, "dmz", "public");
    expect(snap.env.spec.networks.public).toBeTruthy();
    expect(snap.env.spec.vms.edge?.networks[0]?.name).toBe("public");
    expect(snap.diagram.nodes["net:public"]).toBeTruthy();
  });

  it("assigns primary network via container (replace / reorder)", () => {
    let snap = emptySnapshot();
    snap = applyCreateNetwork(snap, "dmz", "10.80.0.0/24", "shared", {
      x: 0,
      y: 0,
    });
    snap = applyCreateNetwork(snap, "lan", "10.90.0.0/24", "shared", {
      x: 400,
      y: 0,
    });
    snap = applyCreateVm(snap, "web", {
      networkName: "dmz",
      position: { x: 40, y: 80 },
    });
    // Single NIC → replace primary
    snap = applyAssignPrimaryNetwork(snap, "web", "lan");
    expect(snap.env.spec.vms.web?.networks).toHaveLength(1);
    expect(snap.env.spec.vms.web?.networks[0]?.name).toBe("lan");

    snap = applyAssignPrimaryNetwork(snap, "web", "dmz", { x: 120, y: 140 });
    expect(snap.env.spec.vms.web?.networks[0]?.name).toBe("dmz");
    expect(snap.diagram.nodes["vm:web"]).toMatchObject({ x: 120, y: 140 });

    snap = applyAttachNic(snap, "web", "lan");
    expect(snap.env.spec.vms.web?.networks.map((n) => n.name)).toEqual([
      "dmz",
      "lan",
    ]);
    // Already attached → reorder to front
    snap = applyAssignPrimaryNetwork(snap, "web", "lan");
    expect(snap.env.spec.vms.web?.networks.map((n) => n.name)).toEqual([
      "lan",
      "dmz",
    ]);
  });

  it("reassigns secondary nic via port reconnect", () => {
    let snap = emptySnapshot();
    snap = applyCreateNetwork(snap, "dmz", "10.80.0.0/24", "shared", {
      x: 0,
      y: 0,
    });
    snap = applyCreateNetwork(snap, "lan", "10.90.0.0/24", "shared", {
      x: 400,
      y: 0,
    });
    snap = applyCreateNetwork(snap, "mgmt", "10.100.0.0/24", "shared", {
      x: 800,
      y: 0,
    });
    snap = applyCreateVm(snap, "web", {
      networkName: "dmz",
      position: { x: 40, y: 80 },
    });
    snap = applyAttachNic(snap, "web", "lan");
    snap = applyReassignNic(snap, "web", "lan", "mgmt");
    expect(snap.env.spec.vms.web?.networks.map((n) => n.name)).toEqual([
      "dmz",
      "mgmt",
    ]);
    expect(snap.diagram.edges["attach:web:lan"]).toBeUndefined();
    expect(snap.diagram.edges["attach:web:mgmt"]).toBeTruthy();
  });
});
