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

const DEMO_SYSTEMD_UNITS = [
  {
    name: "vzctl-agent.service",
    type: "service",
    load: "loaded",
    active: "active",
    sub: "running",
    description: "vzctl guest agent",
  },
  {
    name: "ssh.service",
    type: "service",
    load: "loaded",
    active: "active",
    sub: "running",
    description: "OpenBSD Secure Shell server",
  },
  {
    name: "apt-daily.timer",
    type: "timer",
    load: "loaded",
    active: "inactive",
    sub: "dead",
    description: "Daily apt download activities",
  },
  {
    name: "docker.socket",
    type: "socket",
    load: "loaded",
    active: "active",
    sub: "listening",
    description: "Docker Socket for the API",
  },
];

const DEMO_IMAGES = [
  {
    alias: "ubuntu-latest",
    canonical_alias: "ubuntu-latest",
    aliases: ["ubuntu-latest"],
    distribution: "Ubuntu",
    release: "26.04 LTS",
    architecture: "arm64",
    path: "/Users/demo/Library/Application Support/vzctl/images/sealed/ubuntu-latest@v1.raw",
    format: "raw",
    sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    baked: true,
    sealed: true,
    agent_version: "demo",
  },
  {
    alias: "debian-latest",
    canonical_alias: "debian-latest",
    aliases: ["debian-latest"],
    distribution: "Debian",
    release: "13 (stable/Trixie)",
    architecture: "arm64",
    path: "/Users/demo/Library/Application Support/vzctl/images/baked/debian-latest@v1.raw",
    format: "raw",
    sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    baked: true,
    sealed: false,
    agent_version: "demo",
  },
];

const DEMO_IMAGE_CATALOG = [
  {
    alias: "ubuntu-latest",
    aliases: ["ubuntu-latest"],
    distribution: "Ubuntu",
    release: "26.04 LTS",
  },
  {
    alias: "ubuntu-26.04",
    aliases: ["ubuntu-26.04"],
    distribution: "Ubuntu",
    release: "26.04 LTS",
  },
  {
    alias: "ubuntu-24.04",
    aliases: ["ubuntu-24.04"],
    distribution: "Ubuntu",
    release: "24.04 LTS",
  },
  {
    alias: "ubuntu-22.04",
    aliases: ["ubuntu-22.04"],
    distribution: "Ubuntu",
    release: "22.04 LTS",
  },
  {
    alias: "debian-latest",
    aliases: ["debian-latest"],
    distribution: "Debian",
    release: "13 (stable/Trixie)",
  },
  {
    alias: "debian-13",
    aliases: ["debian-13"],
    distribution: "Debian",
    release: "13 (Trixie)",
  },
  {
    alias: "debian-12",
    aliases: ["debian-12"],
    distribution: "Debian",
    release: "12 (Bookworm)",
  },
  {
    alias: "debian-11",
    aliases: ["debian-11"],
    distribution: "Debian",
    release: "11 (Bullseye)",
  },
  {
    alias: "alpine-latest",
    aliases: ["alpine-latest"],
    distribution: "Alpine Linux",
    release: "3.24.1",
  },
  {
    alias: "fedora-latest",
    aliases: ["fedora-latest"],
    distribution: "Fedora",
    release: "44",
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

  if (cmd === "image" && sub === "list") {
    return ok(
      "image.list",
      {
        images: DEMO_IMAGES,
        catalog: DEMO_IMAGE_CATALOG,
      },
      {
        message: "image cache listed",
        count: DEMO_IMAGES.length,
        images_dir: "/Users/demo/Library/Application Support/vzctl/images",
      },
    );
  }

  if (cmd === "image" && sub === "pull") {
    const alias = rest.find((arg) => !arg.startsWith("-")) ?? "ubuntu-latest";
    return ok(
      "image.pull",
      {
        image: {
          alias,
          canonical_alias: alias,
          sealed: false,
          path: `/images/objects/demo-${alias}.raw`,
        },
      },
      { message: "image pulled", change: "pulled" },
    );
  }

  if (cmd === "image" && sub === "bake") {
    const alias = rest.find((arg) => !arg.startsWith("-")) ?? "ubuntu-latest";
    return ok(
      "image.bake",
      {
        image: {
          alias,
          canonical_alias: alias,
          baked: true,
          agent_version: "demo",
        },
      },
      { message: "image baked", change: "baked" },
    );
  }

  if (cmd === "image" && sub === "seal") {
    const target = rest.find((arg) => !arg.startsWith("-")) ?? "ubuntu-latest";
    return ok(
      "image.seal",
      {
        image: {
          name: target,
          path: `/images/sealed/${target}@v1.raw`,
          sealed: true,
          read_only: true,
        },
      },
      { message: "image sealed", sealed: true, already_sealed: false },
    );
  }

  return ok(args.join("."), {}, { message: "demo no-op", path: DEMO_PROJECT_PATH });
}

/** REST-shaped demo backend for `lib/api.ts`. */
export async function mockApiRequest<T = unknown>(
  path: string,
  options: { method?: string; body?: unknown; rawBody?: string } = {},
): Promise<T> {
  const method = (options.method ?? "GET").toUpperCase();
  const segments = path.split("/").filter(Boolean);

  if (path === "/v1/stacks" && method === "GET") {
    return {
      stacks: [
        {
          id: DEMO_PROJECT_NAME,
          path: DEMO_PROJECT_PATH,
          name: DEMO_PROJECT_NAME,
          openedAt: new Date().toISOString(),
        },
      ],
    } as T;
  }
  if (path === "/v1/stacks" && method === "POST") {
    return {
      id: DEMO_PROJECT_NAME,
      path: DEMO_PROJECT_PATH,
      name: DEMO_PROJECT_NAME,
      openedAt: new Date().toISOString(),
    } as T;
  }
  if (segments[0] === "v1" && segments[1] === "stacks" && segments[3] === "status") {
    return JSON.parse(await mockRunVzctl(DEMO_PROJECT_PATH, "status")) as T;
  }
  if (segments[0] === "v1" && segments[1] === "stacks" && segments[3] === "diff") {
    return JSON.parse(await mockRunVzctl(DEMO_PROJECT_PATH, "diff")) as T;
  }
  if (segments[0] === "v1" && segments[1] === "stacks" && segments[3] === "validate") {
    return JSON.parse(await mockRunVzctl(DEMO_PROJECT_PATH, "validate")) as T;
  }
  if (
    segments[0] === "v1" &&
    segments[1] === "stacks" &&
    ["up", "apply", "down"].includes(segments[3] ?? "") &&
    method === "POST"
  ) {
    return { jobId: "demo-job" } as T;
  }
  if (segments[0] === "v1" && segments[1] === "jobs") {
    const cmd = "apply";
    return {
      jobId: segments[2],
      status: "succeeded",
      result: JSON.parse(await mockRunVzctl(DEMO_PROJECT_PATH, cmd as "apply")),
    } as T;
  }
  if (path === "/v1/vms" && method === "GET") {
    return JSON.parse(await mockRunVzctlArgv(["vm", "list"])) as T;
  }
  if (segments[0] === "v1" && segments[1] === "vms" && segments.length === 4) {
    const action = segments[3];
    const id = decodeURIComponent(segments[2] ?? "");
    if (action === "systemd" && method === "GET") {
      return { available: true, version: "255" } as T;
    }
    if (action === "stats" && method === "GET") {
      return {
        cpu: { percent: 18.5 },
        memory: { used_mib: 512, total_mib: 1024, percent: 50 },
        disk: { read_iops: 4.2, write_iops: 1.8 },
      } as T;
    }
    if (action === "guest-services" && method === "GET") {
      return {
        services: [
          {
            name: "app",
            kind: "iwatch",
            url: "http://127.0.0.1:8787",
            pid: 42,
          },
        ],
      } as T;
    }
    if (
      (action === "start" || action === "stop" || action === "restart") &&
      method === "POST"
    ) {
      const state = action === "stop" ? "stopped" : "running";
      return JSON.parse(
        ok(
          `vm.${action}`,
          { vm: { id, state } },
          { message: `VM ${id} ${state}`, vm_id: id, state },
        ),
      ) as T;
    }
  }
  if (
    segments[0] === "v1" &&
    segments[1] === "vms" &&
    segments[3] === "guest-services" &&
    segments.length >= 6
  ) {
    const apiTail = segments.slice(5).join("/");
    if (method === "POST" && apiTail === "api/restart") {
      return { ok: "restarted" } as T;
    }
    if (method === "GET" && apiTail === "api/status") {
      return {
        observedFields: ["component", "msg"],
        groupField: "component",
        groupValues: ["api", "worker"],
      } as T;
    }
    if (method === "GET" && apiTail.startsWith("api/logs")) {
      return {
        lines: [
          {
            index: 1,
            source: "app",
            text: "demo heartbeat ok",
            level: "info",
            fields: { component: "api" },
          },
          {
            index: 2,
            source: "app",
            text: "demo request failed",
            level: "error",
            fields: { component: "api", msg: "fail" },
          },
        ],
      } as T;
    }
  }
  if (
    segments[0] === "v1" &&
    segments[1] === "vms" &&
    segments[3] === "systemd" &&
    segments[4] === "units"
  ) {
    const unitType = new URL(`http://local${path}`).searchParams.get("type") ?? "service";
    if (segments.length === 5 && method === "GET") {
      const units = DEMO_SYSTEMD_UNITS.filter((unit) => unit.type === unitType);
      return { units } as T;
    }
    if (segments.length === 7 && method === "POST") {
      return {
        ok: true,
        unit: decodeURIComponent(segments[5] ?? ""),
        action: segments[6],
      } as T;
    }
  }
  if (segments[0] === "v1" && segments[1] === "vms" && segments.length === 3) {
    return JSON.parse(await mockRunVzctlArgv(["vm", "inspect", decodeURIComponent(segments[2])])) as T;
  }
  if (path === "/v1/images" && method === "GET") {
    return JSON.parse(await mockRunVzctlArgv(["image", "list"])) as T;
  }
  if (segments[0] === "v1" && segments[1] === "images" && method === "POST") {
    return { jobId: "demo-image-job" } as T;
  }
  if (path === "/v1/nets" && method === "GET") {
    return {
      networks: [
        {
          name: "dmz",
          cidr: "10.80.0.0/24",
          backend: "vmnet",
          runtime_state: "active",
        },
        {
          name: "lan",
          cidr: "10.90.0.0/24",
          backend: "vmnet",
          runtime_state: "active",
        },
      ],
      attachments: [],
    } as T;
  }
  if (path === "/v1/nets/default") {
    return { name: "dmz", cidr: "10.80.0.0/24" } as T;
  }
  if (path === "/v1/doctor") {
    return JSON.parse(await mockRunVzctlArgv(["doctor"])) as T;
  }
  if (path === "/v1/dns/status") {
    return { ok: true, listeners: ["127.0.0.1:15353"] } as T;
  }
  if (path.startsWith("/v1/dns/resolver")) {
    return {
      apiVersion: "vzctl.dev/v1",
      command:
        method === "DELETE" ? "dns.uninstall-resolver" : "dns.install-resolver",
      status: "ok",
      exit_code: 0,
      summary: { message: "resolver unchanged (demo)", change: "unchanged" },
    } as T;
  }
  if (path === "/v1/dns/bind-helper" && method === "POST") {
    return {
      apiVersion: "vzctl.dev/v1",
      command: "dns.install-bind-helper",
      status: "ok",
      exit_code: 0,
      summary: { message: "installed (demo)" },
    } as T;
  }
  if (path === "/v1/oidc/uplink") {
    return "" as T;
  }
  if (segments[0] === "v1" && segments[1] === "projects" && segments[3] === "containers") {
    if (segments.length === 4 && method === "GET") {
      return JSON.parse(await mockRunVzctlArgv(["docker", "ps", "--project", segments[2], "--all"])) as T;
    }
  }
  if (segments[0] === "v1" && segments[1] === "stacks" && segments[3] === "config") {
    return "apiVersion: hypernetwork/v1\nkind: Environment\n" as T;
  }
  if (segments[0] === "v1" && segments[1] === "stacks" && segments[3] === "diagram") {
    return "{}" as T;
  }

  return { ok: true, path, method } as T;
}

