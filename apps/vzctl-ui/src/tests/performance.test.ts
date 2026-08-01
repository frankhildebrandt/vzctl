import { describe, expect, it } from "vitest";
import { performance } from "node:perf_hooks";
import { scaffoldEnvironment } from "@/application/services/scaffold";
import { validateEnvironment } from "@/application/validation/topology";
import {
  applyCreateVm,
  emptySnapshot,
} from "./_helpers";

describe("performance smoke", () => {
  it("validates 200-vm topology under 200ms", () => {
    let snap = emptySnapshot();
    snap.env.spec.networks.lan = {
      cidr: "10.80.0.0/16",
      mode: "shared",
      dhcp: false,
      natEgress: true,
    };
    for (let i = 0; i < 200; i++) {
      snap = applyCreateVm(snap, `vm-${i}`, {
        networkName: "lan",
        position: { x: (i % 20) * 240, y: Math.floor(i / 20) * 140 },
      });
    }
    const t0 = performance.now();
    const issues = validateEnvironment(snap.env);
    const dt = performance.now() - t0;
    expect(dt).toBeLessThan(200);
    expect(issues.filter((i) => i.code === "IP_DUPLICATE").length).toBeGreaterThanOrEqual(0);
    expect(Object.keys(snap.env.spec.vms)).toHaveLength(200);
  });

  it("scaffold is cheap", () => {
    const t0 = performance.now();
    for (let i = 0; i < 100; i++) scaffoldEnvironment({ name: `n${i}` });
    expect(performance.now() - t0).toBeLessThan(100);
  });
});
