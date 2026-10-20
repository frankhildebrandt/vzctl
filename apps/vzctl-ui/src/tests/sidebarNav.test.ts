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

  it("builds stack log-stream group when streams provided", () => {
    const nav = resolveSidebarNav(
      {
        pathname: "/env",
        search: { path: "/tmp/edge-dmz", tab: "ops" },
      },
      {
        t,
        stackLogStreams: [
          {
            vmId: "edge/web",
            source: "nginx.service",
            kind: "journald",
            label: "web/nginx.service",
          },
        ],
      },
    );
    expect(nav.groups?.[0]?.id).toBe("log-streams");
    expect(nav.groups?.[0]?.label).toBe("nav.logStreams");
    expect(nav.groups?.[0]?.items[0]?.label).toBe("web/nginx.service");
    expect(nav.groups?.[0]?.items[0]?.search).toMatchObject({
      stackPath: "/tmp/edge-dmz",
      source: "nginx.service",
    });
  });

  it("uses stack sidebar for vm logs when stackPath is set", () => {
    const nav = resolveSidebarNav(
      {
        pathname: "/vms/edge%2Fweb/logs",
        search: { stackPath: "/tmp/edge-dmz", source: "app" },
        routeVmId: "edge/web",
      },
      {
        t,
        stackTitle: "edge-dmz",
        stackLogStreams: [
          { vmId: "edge/web", source: "app", kind: "iwatch", label: "web/app" },
          {
            vmId: "edge/web",
            source: "worker",
            kind: "iwatch",
            label: "web/worker",
          },
        ],
      },
    );
    expect(nav.context).toBe("stack");
    expect(nav.title).toBe("edge-dmz");
    expect(nav.items.map((i) => i.id)).toEqual(["ops", "topology", "config", "debug"]);
    expect(nav.groups?.[0]?.items.find((i) => i.id === "log:edge/web:app")?.active).toBe(
      true,
    );
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
      "services",
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

  it("builds vm logs group instead of flat logs item", () => {
    const nav = resolveSidebarNav(
      {
        pathname: "/vms/edge%2Fapp/logs",
        search: { source: "app" },
        routeVmId: "edge/app",
      },
      {
        running: true,
        t,
        vmLogStreams: [
          { source: "nginx.service", kind: "journald" },
          { source: "app", kind: "iwatch" },
        ],
      },
    );
    expect(nav.items.map((i) => i.id)).not.toContain("logs");
    expect(nav.groups?.[0]?.id).toBe("logs");
    expect(nav.groups?.[0]?.items.map((i) => i.label)).toEqual([
      "nginx.service",
      "app",
    ]);
    expect(nav.groups?.[0]?.items.map((i) => i.badge)).toEqual([
      "journald",
      "iwatch",
    ]);
    expect(nav.groups?.[0]?.items.find((i) => i.id === "log:app")?.active).toBe(
      true,
    );
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
    expect(nav.items.find((i) => i.id === "services")?.disabled).toBe(true);
  });

  it("builds settings with dashboard escape and no redundant back", () => {
    const nav = resolveSidebarNav({ pathname: "/settings", search: {} }, { t });
    expect(nav.context).toBe("settings");
    expect(nav.back).toBeNull();
    expect(nav.showDashboard).toBe(true);
    expect(nav.showSettingsBottom).toBe(false);
  });
});
