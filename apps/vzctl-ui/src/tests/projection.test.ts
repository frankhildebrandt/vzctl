import { describe, expect, it } from "vitest";
import { scaffoldEnvironment } from "@/application/services/scaffold";
import { emptyDiagramState } from "@/domain/diagram/types";
import { layoutByNetwork } from "@/diagram/projections/layout";

describe("projection helpers", () => {
  it("auto-layout places nets and vms", () => {
    const env = scaffoldEnvironment({ name: "perf" });
    env.spec.networks.dmz = { cidr: "10.90.0.0/24", mode: "shared", dhcp: false, natEgress: true };
    for (let i = 0; i < 20; i++) {
      env.spec.vms[`vm-${i}`] = {
        from: "ubuntu-base",
        clone: "linked",
        dataDisk: "10G",
        networks: [{ name: i % 2 === 0 ? "lan" : "dmz", ip: `10.${i % 2 === 0 ? 80 : 90}.0.${10 + i}` }],
        dependsOn: [],
        roles: [],
        requires: [],
        ports: [],
        mounts: [],
      };
    }
    const nodes = layoutByNetwork(env);
    expect(nodes["net:lan"]).toBeTruthy();
    expect(nodes["net:dmz"]).toBeTruthy();
    expect(nodes["vm:vm-0"]).toBeTruthy();
    expect(emptyDiagramState().schemaVersion).toBe(1);
  });
});
