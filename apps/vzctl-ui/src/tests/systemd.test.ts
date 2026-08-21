import { describe, expect, it } from "vitest";
import {
  isUnitActive,
  unitStatusLabel,
  type SystemdUnit,
} from "@/lib/systemd";

describe("systemd helpers", () => {
  const running: SystemdUnit = {
    name: "ssh.service",
    type: "service",
    load: "loaded",
    active: "active",
    sub: "running",
    description: "ssh",
  };

  it("detects active units", () => {
    expect(isUnitActive(running)).toBe(true);
    expect(
      isUnitActive({
        ...running,
        active: "inactive",
        sub: "dead",
      }),
    ).toBe(false);
  });

  it("formats status label", () => {
    expect(unitStatusLabel(running)).toBe("active/running");
  });
});
