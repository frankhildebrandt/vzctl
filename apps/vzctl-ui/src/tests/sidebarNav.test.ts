import { describe, expect, it } from "vitest";
import { resolveSidebarNav } from "@/lib/sidebarNav";

describe("resolveSidebarNav", () => {
  it("returns root nav without back", () => {
    const nav = resolveSidebarNav({ pathname: "/", search: {} });
    expect(nav.context).toBe("root");
    expect(nav.back).toBeNull();
    expect(nav.showDashboard).toBe(false);
    expect(nav.showSettingsBottom).toBe(true);
    expect(nav.items.map((i) => i.id)).toEqual([
      "dashboard",
      "vms",
      "projects",
      "networks",
      "images",
      "doctor",
    ]);
  });

  it("builds stack nav with tabs and back to projects", () => {
    const nav = resolveSidebarNav({
      pathname: "/env",
      search: { path: "/tmp/edge-dmz", tab: "topology" },
    });
    expect(nav.context).toBe("stack");
    expect(nav.back?.to).toBe("/projects");
    expect(nav.showDashboard).toBe(true);
    expect(nav.items.find((i) => i.id === "topology")?.active).toBe(true);
    expect(nav.items.find((i) => i.id === "ops")?.active).toBe(false);
  });

  it("builds vm overview back to stack when stackPath set", () => {
    const nav = resolveSidebarNav(
      {
        pathname: "/vms/edge%2Fapp",
        search: { stackPath: "/tmp/edge-dmz" },
      },
      { hasDockerRole: true },
    );
    expect(nav.context).toBe("vm");
    expect(nav.back?.to).toBe("/env");
    expect(nav.items.map((i) => i.id)).toEqual(["overview", "containers"]);
    expect(nav.items.find((i) => i.id === "overview")?.active).toBe(true);
  });

  it("builds container list back to vm overview", () => {
    const nav = resolveSidebarNav({
      pathname: "/vms/edge%2Fapp/containers",
      search: { stackPath: "/tmp/edge-dmz" },
    });
    expect(nav.back?.to).toBe("/vms/$vmId");
    expect(nav.items.find((i) => i.id === "containers")?.active).toBe(true);
  });

  it("hides containers without docker role", () => {
    const nav = resolveSidebarNav(
      { pathname: "/vms/plain", search: {} },
      { hasDockerRole: false },
    );
    expect(nav.items.map((i) => i.id)).toEqual(["overview"]);
  });

  it("builds settings with dashboard escape and no redundant back", () => {
    const nav = resolveSidebarNav({ pathname: "/settings", search: {} });
    expect(nav.context).toBe("settings");
    expect(nav.back).toBeNull();
    expect(nav.showDashboard).toBe(true);
    expect(nav.showSettingsBottom).toBe(false);
  });
});
