import { describe, expect, it } from "vitest";
import type { GuestService } from "@/lib/guestLogs";
import type { Environment } from "@/domain/hypernetwork/schema";
import {
  filterIwatchServices,
  isLogStreamActive,
  journaldStreamsFromEnv,
  mergeVmLogStreams,
  stackVmShortLabel,
} from "@/lib/logStreams";

const env: Environment = {
  apiVersion: "hypernetwork/v1",
  kind: "Environment",
  metadata: { name: "edge" },
  spec: {
    project: "edge",
    domain: "edge.vz.test",
    dns: {
      enabled: true,
      hostResolver: true,
      hostListen: "127.0.0.1:15353",
      forward: { enabled: true, upstream: "system" },
    },
    images: {},
    networks: {
      lan: {
        cidr: "10.0.0.0/24",
        mode: "shared",
        dhcp: false,
        natEgress: true,
        backend: "vmnet",
      },
    },
    routes: [],
    policies: [],
    volumes: {},
    ports: [],
    vms: {
      web: {
        from: "base",
        clone: "linked",
        disk: "10G",
        networks: [{ name: "lan", ip: "10.0.0.10" }],
        dependsOn: [],
        roles: [],
        requires: [],
        ports: [],
        mounts: [],
        logs: { journald: ["nginx.service", "caddy.service"] },
      },
    },
  },
};

describe("filterIwatchServices", () => {
  it("keeps only iwatch publishers", () => {
    const services: GuestService[] = [
      { name: "app", kind: "iwatch" },
      { name: "other", kind: "http" },
    ];
    expect(filterIwatchServices(services).map((s) => s.name)).toEqual(["app"]);
  });
});

describe("journaldStreamsFromEnv", () => {
  it("reads configured units for runtime vm id", () => {
    const streams = journaldStreamsFromEnv("edge/web", env);
    expect(streams.map((s) => s.source)).toEqual([
      "nginx.service",
      "caddy.service",
    ]);
    expect(streams.every((s) => s.kind === "journald")).toBe(true);
  });
});

describe("mergeVmLogStreams", () => {
  it("prefers journald and avoids duplicate names", () => {
    const merged = mergeVmLogStreams(
      [{ source: "nginx.service", kind: "journald" }],
      [{ source: "app", kind: "iwatch", live: true }],
    );
    expect(merged.map((s) => s.source)).toEqual(["nginx.service", "app"]);
  });
});

describe("stackVmShortLabel", () => {
  it("strips project prefix", () => {
    expect(stackVmShortLabel("edge/web")).toBe("web");
    expect(stackVmShortLabel("plain")).toBe("plain");
  });
});

describe("isLogStreamActive", () => {
  const streams = [
    { source: "nginx.service", kind: "journald" as const },
    { source: "app", kind: "iwatch" as const },
  ];

  it("marks explicit source on logs section", () => {
    expect(isLogStreamActive("logs", "app", "app", streams)).toBe(true);
    expect(isLogStreamActive("logs", "nginx.service", "app", streams)).toBe(
      false,
    );
  });

  it("defaults to first stream when source missing", () => {
    expect(isLogStreamActive("logs", "nginx.service", undefined, streams)).toBe(
      true,
    );
    expect(isLogStreamActive("logs", "app", undefined, streams)).toBe(false);
  });

  it("is inactive outside logs section", () => {
    expect(isLogStreamActive("overview", "nginx.service", "nginx.service", streams)).toBe(
      false,
    );
  });
});
