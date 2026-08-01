import type { VzctlCommand, RunVzctlOptions } from "@/lib/vzctl";
import { DEMO_PROJECT_NAME, DEMO_PROJECT_PATH } from "@/lib/demo";

function ok(
  command: string,
  extra: Record<string, unknown> = {},
  summary: Record<string, unknown> = { message: "ok" },
): string {
  return JSON.stringify({
    apiVersion: "vzctl.dev/v1",
    command,
    status: "ok",
    exit_code: 0,
    summary,
    ...extra,
  });
}

const DEMO_VMS = [
  {
    id: `${DEMO_PROJECT_NAME}/router`,
    state: "running",
    pid: 5101,
    bundle: `/state/vms/${DEMO_PROJECT_NAME}/router`,
    "managed-by": "vzctl",
    roles: ["router"],
    ips: ["10.80.0.2", "10.90.0.2"],
    networks: [
      { name: "dmz", ip: "10.80.0.2" },
      { name: "lan", ip: "10.90.0.2" },
    ],
    resources: { cpus: 2, memory_mib: 1024 },
    updated_at: "2026-08-01T12:00:00Z",
  },
  {
    id: `${DEMO_PROJECT_NAME}/web`,
    state: "running",
    pid: 5102,
    bundle: `/state/vms/${DEMO_PROJECT_NAME}/web`,
    "managed-by": "vzctl",
    roles: [],
    ips: ["10.80.0.10"],
    networks: [{ name: "dmz", ip: "10.80.0.10" }],
    resources: { cpus: 2, memory_mib: 2048 },
    updated_at: "2026-08-01T12:00:00Z",
  },
  {
    id: `${DEMO_PROJECT_NAME}/docker`,
    state: "running",
    pid: 5103,
    bundle: `/state/vms/${DEMO_PROJECT_NAME}/docker`,
    "managed-by": "vzctl",
    roles: ["docker", "router"],
    ips: ["10.90.0.10", "10.95.0.2"],
    networks: [
      { name: "lan", ip: "10.90.0.10" },
      { name: "containers", ip: "10.95.0.2" },
    ],
    resources: { cpus: 4, memory_mib: 4096 },
    updated_at: "2026-08-01T12:00:00Z",
  },
  {
    id: `${DEMO_PROJECT_NAME}/host`,
    state: "running",
    pid: 5104,
    bundle: `/state/vms/${DEMO_PROJECT_NAME}/host`,
    "managed-by": "vzctl",
    roles: [],
    ips: ["10.90.0.11"],
    networks: [{ name: "lan", ip: "10.90.0.11" }],
    resources: { cpus: 2, memory_mib: 2048 },
    updated_at: "2026-08-01T12:00:00Z",
  },
];

const DEMO_CONTAINERS = [
  {
    id: "a1b2c3d4e5f6789012345678",
    names: "edge-web",
    image: "nginx:alpine",
    status: "Up 2 hours",
    state: "running",
    ports: "0.0.0.0:8080->80/tcp",
    command: "nginx -g 'daemon off;'",
    ip: "10.95.0.10",
  },
  {
    id: "b2c3d4e5f678901234567890",
    names: "edge-redis",
    image: "redis:7-alpine",
    status: "Up 2 hours",
    state: "running",
    ports: "6379/tcp",
    command: "redis-server",
    ip: "10.95.0.11",
  },
];

function statusBundle(): string {
  return JSON.stringify({
    apiVersion: "vzctl.dev/v1",
    command: "status",
    status: "ok",
    exit_code: 0,
    summary: { message: "demo status" },
    sections: {
      stack: {
        ok: true,
        data: {
          phase: "running",
          label: "Up (Running)",
          stack_id: `${DEMO_PROJECT_NAME}:${DEMO_PROJECT_NAME}`,
          project: DEMO_PROJECT_NAME,
          vms: {
            desired: 4,
            running: 4,
            starting: 0,
            stopping: 0,
            stopped: 0,
            missing: 0,
          },
          items: DEMO_VMS.map((vm) => ({
            id: vm.id,
            name: vm.id.split("/")[1],
            state: vm.state,
            present: true,
          })),
        },
      },
      ingress: {
        ok: true,
        data: {
          enabled: true,
          https_port: 443,
          host_aliases: true,
          routes: [
            {
              host: "web.svc.edge-dmz.vz.test",
              url: "https://web.svc.edge-dmz.vz.test",
              to: "web:80",
              requires: ["oidc"],
              alias: {
                host: "web.local",
                url: "https://127.0.0.1:8443",
              },
            },
            {
              host: "auth.svc.edge-dmz.vz.test",
              url: "https://auth.svc.edge-dmz.vz.test",
              to: "oidc:5556",
              requires: [],
            },
          ],
        },
      },
      dns: {
        ok: false,
        data: {
          dns: {
            ok: false,
            listeners: ["127.0.0.1:15353"],
            zones: 1,
            records: 12,
            upstream: "system",
            last_error:
              "bind 10.80.0.0:53 failed: Permission denied (dns-bind helper missing)",
          },
        },
      },
      certs: {
        ok: true,
        data: {
          data: {
            present: true,
            trusted: true,
            fingerprint: "5a1b2c3d4e5f67890123456789abcdef01234567",
            path: "/Users/demo/Library/Application Support/vzctl/certs/ca.pem",
          },
        },
      },
      oidc: {
        ok: false,
        data: {
          data: {
            running: false,
            pid: null,
            project: DEMO_PROJECT_NAME,
          },
        },
      },
      diff: {
        ok: true,
        data: {
          stack_id: `${DEMO_PROJECT_NAME}:${DEMO_PROJECT_NAME}`,
          actions: [],
          summary: { changed: false, message: "no changes" },
        },
      },
    },
  });
}

function doctorBundle(): string {
  return JSON.stringify({
    apiVersion: "vzctl.dev/v1",
    command: "doctor",
    status: "warn",
    exit_code: 0,
    summary: { ok: 3, warnings: 2, failures: 0 },
    checks: [
      {
        id: "host.macos",
        status: "ok",
        message: "macOS 26 meets the baseline",
        details: { major: 26, minimum_major: 26 },
      },
      {
        id: "certs.host_trust",
        status: "ok",
        message: "Local CA present and trusted",
        details: {
          present: true,
          trusted: true,
          fingerprint: "5a1b2c3d4e5f67890123456789abcdef01234567",
          path: "/Users/demo/Library/Application Support/vzctl/certs/ca.pem",
        },
      },
      {
        id: "dns.bind_helper",
        status: "warn",
        message: "DNS bind helper not installed",
        details: { requires_helper: true, installed: false },
      },
      {
        id: "codesign.vz-helper",
        status: "ok",
        message: "vz-helper codesign ok (demo)",
        details: { found: true },
      },
      {
        id: "supervisor",
        status: "warn",
        message: "Demo mode — no live supervisor",
        details: { demo: true },
      },
    ],
  });
}

function findVm(id: string) {
  return DEMO_VMS.find((vm) => vm.id === id || vm.id.endsWith(`/${id}`));
}

export async function mockRunVzctl(
  path: string,
  command: VzctlCommand,
  _options: RunVzctlOptions = {},
): Promise<string> {
  void path;
  switch (command) {
    case "status":
      return statusBundle();
    case "diff":
      return ok(
        "diff",
        {
          stack_id: `${DEMO_PROJECT_NAME}:${DEMO_PROJECT_NAME}`,
          actions: [],
        },
        { changed: false, message: "no changes (demo)" },
      );
    case "validate":
      return ok("validate", {}, { message: "valid (demo)" });
    case "up":
    case "apply":
    case "down":
      return ok(command, {}, { message: `${command} ok (demo, no-op)` });
    default:
      return ok(command);
  }
}

export async function mockRunVzctlArgv(args: string[]): Promise<string> {
  const [cmd, sub, ...rest] = args;

  if (cmd === "vm" && sub === "list") {
    return ok(
      "vm.list",
      { vms: DEMO_VMS, warnings: [] },
      { message: `${DEMO_VMS.length} VM(s)`, vms: DEMO_VMS.length, running: 4 },
    );
  }

  if (cmd === "vm" && sub === "inspect") {
    const id = rest[0] ?? "";
    const vm = findVm(id) ?? {
      id,
      state: "running",
      pid: 9999,
      roles: [],
      ips: ["10.80.0.99"],
      networks: [{ name: "dmz", ip: "10.80.0.99" }],
      resources: { cpus: 2, memory_mib: 1024 },
    };
    return ok(
      "vm.inspect",
      {
        vm,
        networks: vm.networks,
        identity: { hostname: id.split("/").pop() ?? id },
        disks: { root: "linked-clone", data: "40G" },
        agent: { connected: true, version: "demo" },
        logs: { serial: "[demo] cloud-init done\n[demo] agent ready\n" },
        warnings: [],
      },
      { message: `inspected ${vm.id}` },
    );
  }

  if (cmd === "vm" && sub === "mounts") {
    return ok(
      "vm.mounts",
      {
        mounts: [
          {
            name: "workspace",
            source: "/Users/demo/Projects/app",
            target: "/mnt/workspace",
            read_only: false,
          },
        ],
      },
      { message: "1 mount(s)" },
    );
  }

  if (cmd === "vm" && (sub === "start" || sub === "stop" || sub === "delete")) {
    return ok(`vm.${sub}`, {}, { message: `${sub} ok (demo)`, vm_id: rest[0] });
  }

  if (cmd === "vm" && (sub === "modify" || sub === "mount" || sub === "unmount")) {
    return ok(`vm.${sub}`, {}, { message: `${sub} ok (demo)`, restart_required: false });
  }

  if (cmd === "doctor") {
    return doctorBundle();
  }

  if (cmd === "docker" && sub === "ps") {
    return ok(
      "docker.ps",
      {},
      { containers: DEMO_CONTAINERS, message: `${DEMO_CONTAINERS.length} containers` },
    );
  }

  if (cmd === "docker" && sub === "inspect") {
    const id = rest[rest.length - 1] ?? "unknown";
    const container =
      DEMO_CONTAINERS.find((c) => c.id === id || c.id.startsWith(id)) ??
      DEMO_CONTAINERS[0];
    return ok(
      "docker.inspect",
      {},
      {
        inspect: {
          Id: container.id,
          Name: `/${container.names}`,
          Config: {
            Image: container.image,
            Cmd: container.command.split(" "),
            Env: ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
          },
          State: {
            Status: container.state,
            Running: container.state === "running",
            Pid: 42,
          },
          NetworkSettings: {
            Ports: { "80/tcp": [{ HostIp: "0.0.0.0", HostPort: "8080" }] },
            IPAddress: "10.95.0.10",
          },
        },
      },
    );
  }

  if (cmd === "docker" && (sub === "start" || sub === "stop" || sub === "restart" || sub === "run")) {
    return ok(`docker.${sub}`, {}, { message: `${sub} ok (demo)`, id: DEMO_CONTAINERS[0].id });
  }

  if (cmd === "certs" && sub === "ca") {
    return ok(`certs.ca.${rest[0] ?? "ok"}`, {}, { message: "ca ok (demo)" });
  }

  if (cmd === "dns" && sub === "install-bind-helper") {
    return ok("dns.install-bind-helper", {}, { message: "installed (demo)" });
  }

  return ok(args.join("."), {}, { message: "demo no-op", path: DEMO_PROJECT_PATH });
}
