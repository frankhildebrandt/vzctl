import { describe, expect, it } from "vitest";
import type { MessageKey } from "@/lib/i18n";
import { resolveSidebarNav } from "@/lib/sidebarNav";

const t = (key: MessageKey) => key;

describe("resolveSidebarNav", () => {
  it("returns root nav without back", () => {
    const nav = resolveSidebarNav({ pathname: "/", search: {} }, { t });
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
      "errors",
    ]);
  });

  it("builds stack nav with tabs and back to projects", () => {
    const nav = resolveSidebarNav(
      {
        pathname: "/env",
        search: { path: "/tmp/edge-dmz", tab: "topology" },
      },
      { t },
    );
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
      { hasDockerRole: true, t },
    );
    expect(nav.context).toBe("vm");
    expect(nav.back?.to).toBe("/env");
    expect(nav.items.map((i) => i.id)).toEqual([
      "overview",
      "shell",
      "console",
      "modify",
      "mount",
      "replace",
      "containers",
      "delete",
    ]);
    expect(nav.items.find((i) => i.id === "overview")?.active).toBe(true);
    expect(nav.items.find((i) => i.id === "delete")?.kind).toBe("action");
  });

  it("marks shell active and enables it when running", () => {
    const nav = resolveSidebarNav(
      { pathname: "/vms/plain/shell", search: {} },
      { running: true, t },
    );
    expect(nav.items.find((i) => i.id === "shell")?.active).toBe(true);
    expect(nav.items.find((i) => i.id === "shell")?.disabled).toBe(false);
    expect(nav.items.find((i) => i.id === "overview")?.active).toBe(false);
  });

  it("builds container list back to vm overview", () => {
    const nav = resolveSidebarNav(
      {
        pathname: "/vms/edge%2Fapp/containers",
        search: { stackPath: "/tmp/edge-dmz" },
      },
      { t },
    );
    expect(nav.back?.to).toBe("/vms/$vmId");
    expect(nav.items.find((i) => i.id === "containers")?.active).toBe(true);
  });

  it("hides containers without docker role", () => {
    const nav = resolveSidebarNav(
      { pathname: "/vms/plain", search: {} },
      { hasDockerRole: false, t },
    );
    expect(nav.items.map((i) => i.id)).not.toContain("containers");
    expect(nav.items.map((i) => i.id)).toContain("overview");
    expect(nav.items.map((i) => i.id)).toContain("delete");
    expect(nav.items.find((i) => i.id === "shell")?.disabled).toBe(true);
  });

  it("builds settings with dashboard escape and no redundant back", () => {
    const nav = resolveSidebarNav({ pathname: "/settings", search: {} }, { t });
    expect(nav.context).toBe("settings");
    expect(nav.back).toBeNull();
    expect(nav.showDashboard).toBe(true);
    expect(nav.showSettingsBottom).toBe(false);
  });
});
