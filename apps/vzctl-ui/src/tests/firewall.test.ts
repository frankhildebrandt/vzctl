import { describe, expect, it } from "vitest";
import { scaffoldEnvironment } from "@/application/services/scaffold";
import { validateEnvironment } from "@/application/validation/topology";
import {
  applyEnsurePolicyForNetwork,
  applySetAllowRules,
  applyUpdateNetwork,
  applyCreateNetwork,
  type EditorSnapshot,
} from "@/application/commands/mutations";
import { emptyDiagramState } from "@/domain/diagram/types";
import { layoutByNetwork } from "@/diagram/projections/layout";

describe("firewall / policy validation", () => {
  it("requires ports for tcp", () => {
    let env = scaffoldEnvironment({ name: "fw" });
    env.spec.networks.dmz = {
      cidr: "10.90.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: true,
      backend: "vmnet",
    };
    let snap = { env, diagram: emptyDiagramState() };
    snap = applyEnsurePolicyForNetwork(snap, "lan");
    snap = applySetAllowRules(snap, "lan-default", [
      { to: "dmz", proto: "tcp", ports: [] },
    ]);
    const issues = validateEnvironment(snap.env);
    expect(issues.some((i) => i.code === "POLICY_PORTS_REQUIRED")).toBe(true);
  });

  it("rejects icmp with ports", () => {
    let env = scaffoldEnvironment({ name: "fw" });
    env.spec.networks.dmz = {
      cidr: "10.90.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: true,
      backend: "vmnet",
    };
    let snap = { env, diagram: emptyDiagramState() };
    snap = applyEnsurePolicyForNetwork(snap, "lan");
    snap = applySetAllowRules(snap, "lan-default", [
      { to: "dmz", proto: "icmp", ports: [8] },
    ]);
    const issues = validateEnvironment(snap.env);
    expect(issues.some((i) => i.code === "POLICY_ICMP_NO_PORTS")).toBe(true);
  });

  it("accepts to:internet when router has natEgress uplink", () => {
    let env = scaffoldEnvironment({ name: "fw" });
    env.spec.networks.dmz = {
      cidr: "10.90.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: false,
      backend: "vmnet",
    };
    env.spec.vms.edge = {
      from: "ubuntu-base",
      clone: "linked",
      dataDisk: "10G",
      networks: [
        { name: "lan", ip: "10.80.0.2" },
        { name: "dmz", ip: "10.90.0.2" },
      ],
      dependsOn: [],
      roles: ["router"],
      requires: [],
      ports: [],
      mounts: [],
    };
    let snap = { env, diagram: emptyDiagramState() };
    snap = applyEnsurePolicyForNetwork(snap, "dmz");
    snap = applySetAllowRules(snap, "dmz-default", [
      { to: "internet", proto: "tcp", ports: [443] },
    ]);
    const issues = validateEnvironment(snap.env);
    expect(issues.some((i) => i.code === "POLICY_INTERNET_NO_ROUTER")).toBe(
      false,
    );
  });

  it("disabling natEgress creates deny-all policy and drops igw layout", () => {
    let env = scaffoldEnvironment({ name: "iso" });
    env.spec.networks.dmz = {
      cidr: "10.90.0.0/24",
      mode: "shared",
      dhcp: false,
      natEgress: true,
      backend: "vmnet",
    };
    let snap: EditorSnapshot = {
      env,
      diagram: {
        ...emptyDiagramState(),
        nodes: {
          "net:dmz": { x: 0, y: 0, width: 320, height: 200 },
          "igw:dmz": { x: 0, y: -12, width: 100, height: 68 },
        },
      },
    };
    snap = applyUpdateNetwork(snap, "dmz", { natEgress: false });
    expect(snap.env.spec.networks.dmz.natEgress).toBe(false);
    expect(snap.env.spec.policies.some((p) => p.network === "dmz")).toBe(true);
    expect(snap.diagram.nodes["igw:dmz"]).toBeUndefined();

    const layout = layoutByNetwork(snap.env);
    expect(layout["igw:lan"]).toBeTruthy();
    expect(layout["igw:dmz"]).toBeUndefined();
  });

  it("create network without natEgress seeds policy", () => {
    let snap = {
      env: scaffoldEnvironment({ name: "iso" }),
      diagram: emptyDiagramState(),
    };
    snap = applyCreateNetwork(
      snap,
      "priv",
      "10.70.0.0/24",
      "shared",
      { x: 10, y: 10 },
      { natEgress: false },
    );
    expect(snap.env.spec.networks.priv.natEgress).toBe(false);
    expect(snap.env.spec.networks.priv.backend).toBe("vmnet");
    expect(
      snap.env.spec.policies.some(
        (p) => p.network === "priv" && p.forward === "deny-all",
      ),
    ).toBe(true);
  });

  it("backend docker forces isolated and validates owner", () => {
    let snap = {
      env: scaffoldEnvironment({ name: "dock" }),
      diagram: emptyDiagramState(),
    };
    snap = applyCreateNetwork(
      snap,
      "containers",
      "10.95.0.0/24",
      "shared",
      { x: 10, y: 10 },
      { backend: "docker" },
    );
    expect(snap.env.spec.networks.containers.backend).toBe("docker");
    expect(snap.env.spec.networks.containers.natEgress).toBe(false);
    expect(snap.env.spec.networks.containers.dhcp).toBe(false);

    snap.env.spec.vms.docker = {
      from: "ubuntu-base",
      clone: "linked",
      dataDisk: "40G",
      networks: [
        { name: "lan", ip: "10.80.0.10" },
        { name: "containers", ip: "10.95.0.2" },
      ],
      dependsOn: [],
      roles: ["docker", "router"],
      requires: [],
      ports: [],
      mounts: [],
    };
    const issues = validateEnvironment(snap.env);
    expect(issues.some((i) => i.code.startsWith("DOCKER_BACKEND"))).toBe(false);
  });
});
