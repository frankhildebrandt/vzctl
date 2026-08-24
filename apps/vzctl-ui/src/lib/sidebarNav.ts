import { useQuery } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { useStackLogStreams } from "@/hooks/useStackLogStreams";
import { useVmLogStreams } from "@/hooks/useVmLogStreams";
import { getT, useT, type TFunction } from "@/lib/i18n";
import {
  isLogStreamActive,
  type StackLogStream,
  type VmLogStream,
} from "@/lib/logStreams";
import { getProject } from "@/lib/projects";
import { basename } from "@/lib/vzctl";
import {
  decodeVmIdParam,
  encodeVmIdParam,
  inspectVm,
  isRunning,
  vmKeys,
} from "@/lib/vms";

export type SidebarContext = "root" | "stack" | "vm" | "settings";

export type SidebarNavItem = {
  id: string;
  label: string;
  to: string;
  params?: Record<string, string>;
  search?: Record<string, unknown>;
  exact?: boolean;
  /** Explicit active (for search-param tabs); when set, overrides Link matching. */
  active?: boolean;
  kind?: "link" | "action";
  tone?: "danger";
  disabled?: boolean;
  /** Small source badge (`iwatch`, `journald`). */
  badge?: string;
};

export type SidebarNavGroup = {
  id: string;
  label: string;
  items: SidebarNavItem[];
};

export type SidebarBack = {
  label: string;
  to: string;
  params?: Record<string, string>;
  search?: Record<string, unknown>;
};

export type SidebarNavModel = {
  context: SidebarContext;
  /** Stable key for enter-animation remount. */
  contextKey: string;
  title: string | null;
  back: SidebarBack | null;
  /** Escape to dashboard — only in nested contexts. */
  showDashboard: boolean;
  items: SidebarNavItem[];
  groups?: SidebarNavGroup[];
  /** Settings link in sidebar-bottom (root only). */
  showSettingsBottom: boolean;
};

export type VmSection =
  | "overview"
  | "logs"
  | "services"
  | "shell"
  | "console"
  | "modify"
  | "mount"
  | "replace"
  | "containers";

export type SidebarLocationInput = {
  pathname: string;
  search: Record<string, unknown>;
  /** Decoded `$vmId` from the deepest matching VM route, if any. */
  routeVmId?: string;
  /** True when on `/vms/$vmId/containers` or deeper. */
  onContainers?: boolean;
};

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function vmTail(pathname: string): string {
  const match = pathname.match(/^\/vms\/[^/]+(?:\/(.*))?$/);
  return match?.[1] ?? "";
}

function isVmLogsPath(pathname: string): boolean {
  return /^\/vms\/[^/]+\/logs/.test(pathname);
}

function parseVmSection(tail: string): {
  section: VmSection;
  containerLevel?: "list" | "detail";
} {
  const parts = tail.split("/").filter(Boolean);
  if (parts[0] === "containers") {
    return {
      section: "containers",
      containerLevel: parts.length >= 2 ? "detail" : "list",
    };
  }
  if (
    parts[0] === "logs" ||
    parts[0] === "services" ||
    parts[0] === "shell" ||
    parts[0] === "console" ||
    parts[0] === "modify" ||
    parts[0] === "mount" ||
    parts[0] === "replace"
  ) {
    return { section: parts[0] };
  }
  return { section: "overview" };
}

function parseLocation(input: SidebarLocationInput): {
  kind: "root" | "stack" | "stack-logs" | "vm" | "settings";
  vmId?: string;
  containerLevel?: "list" | "detail";
  section?: VmSection;
  stackPath?: string;
  envPath?: string;
  tab?: "ops" | "topology" | "config" | "debug";
  logSource?: string;
} {
  const { pathname, search } = input;

  if (pathname === "/settings" || pathname.startsWith("/settings/")) {
    return { kind: "settings" };
  }
  if (pathname === "/env" || pathname.startsWith("/env/")) {
    const tabRaw = search.tab;
    const tab =
      tabRaw === "topology" ||
      tabRaw === "ops" ||
      tabRaw === "config" ||
      tabRaw === "debug"
        ? tabRaw
        : "ops";
    return {
      kind: "stack",
      envPath: asString(search.path),
      tab,
    };
  }

  const stackPath = asString(search.stackPath);
  const logSource = asString(search.source);

  if (input.routeVmId && isVmLogsPath(pathname) && stackPath) {
    return {
      kind: "stack-logs",
      vmId: input.routeVmId,
      stackPath,
      logSource,
      section: "logs",
    };
  }

  if (input.routeVmId) {
    const parsed = parseVmSection(vmTail(pathname));
    return {
      kind: "vm",
      vmId: input.routeVmId,
      containerLevel: parsed.containerLevel,
      section: parsed.section,
      stackPath,
      logSource: parsed.section === "logs" ? logSource : undefined,
    };
  }

  const vmMatch = pathname.match(/^\/vms\/([^/]+)(?:\/(.*))?$/);
  if (vmMatch) {
    const parsed = parseVmSection(vmMatch[2] ?? "");
    const vmId = decodeVmIdParam(vmMatch[1]);
    if (isVmLogsPath(pathname) && stackPath) {
      return {
        kind: "stack-logs",
        vmId,
        stackPath,
        logSource,
        section: "logs",
      };
    }
    return {
      kind: "vm",
      vmId,
      containerLevel: parsed.containerLevel,
      section: parsed.section,
      stackPath,
      logSource: parsed.section === "logs" ? logSource : undefined,
    };
  }

  return { kind: "root" };
}

function buildVmLogStreamNavItems(
  vmId: string,
  streams: VmLogStream[],
  opts: {
    stackPath?: string;
    section: VmSection;
    activeSource?: string;
    running: boolean;
  },
): SidebarNavItem[] {
  const encoded = encodeVmIdParam(vmId);
  const baseSearch = opts.stackPath ? { stackPath: opts.stackPath } : {};

  return streams.map((stream) => ({
    id: `log:${stream.source}`,
    label: stream.source,
    badge: stream.kind,
    to: "/vms/$vmId/logs",
    params: { vmId: encoded },
    search: { ...baseSearch, source: stream.source },
    active: isLogStreamActive(
      opts.section,
      stream.source,
      opts.activeSource,
      streams,
    ),
    disabled: !opts.running,
  }));
}

function buildStackLogStreamNavItems(
  streams: StackLogStream[],
  stackPath: string,
  activeLog?: { vmId: string; source: string },
): SidebarNavItem[] {
  return streams.map((stream) => ({
    id: `log:${stream.vmId}:${stream.source}`,
    label: stream.label,
    badge: stream.kind,
    to: "/vms/$vmId/logs",
    params: { vmId: encodeVmIdParam(stream.vmId) },
    search: { stackPath, source: stream.source },
    active:
      activeLog?.vmId === stream.vmId && activeLog?.source === stream.source,
  }));
}

function buildRootItems(t: TFunction): SidebarNavItem[] {
  return [
    { id: "dashboard", label: t("nav.dashboard"), to: "/", exact: true },
    { id: "vms", label: t("nav.vms"), to: "/vms" },
    { id: "projects", label: t("nav.stacks"), to: "/projects" },
    { id: "networks", label: t("nav.networks"), to: "/networks" },
    { id: "images", label: t("nav.images"), to: "/images" },
    { id: "doctor", label: t("nav.doctor"), to: "/doctor" },
    { id: "errors", label: t("nav.errors"), to: "/errors" },
  ];
}

function buildRoot(t: TFunction): SidebarNavModel {
  return {
    context: "root",
    contextKey: "root",
    title: null,
    back: null,
    showDashboard: false,
    items: buildRootItems(t),
    showSettingsBottom: true,
  };
}

function buildSettings(t: TFunction): SidebarNavModel {
  return {
    context: "settings",
    contextKey: "settings",
    title: t("nav.settings"),
    back: null,
    showDashboard: true,
    items: [
      {
        id: "settings",
        label: t("nav.settings"),
        to: "/settings",
        exact: true,
        active: true,
      },
    ],
    showSettingsBottom: false,
  };
}

function buildStack(
  t: TFunction,
  envPath: string | undefined,
  tab: "ops" | "topology" | "config" | "debug",
  opts?: {
    titleOverride?: string | null;
    stackLogStreams?: StackLogStream[];
    activeLog?: { vmId: string; source: string };
  },
): SidebarNavModel {
  const title =
    opts?.titleOverride ??
    (envPath ? basename(envPath) : t("nav.stackFallback"));
  const searchBase = envPath ? { path: envPath } : { path: "" };
  const streams = opts?.stackLogStreams ?? [];
  const groups: SidebarNavGroup[] = [];

  if (streams.length > 0 && envPath) {
    groups.push({
      id: "log-streams",
      label: t("nav.logStreams"),
      items: buildStackLogStreamNavItems(streams, envPath, opts?.activeLog),
    });
  }

  return {
    context: "stack",
    contextKey: `stack:${envPath ?? ""}`,
    title,
    back: { label: t("nav.back"), to: "/projects" },
    showDashboard: true,
    items: [
      {
        id: "ops",
        label: t("nav.ops"),
        to: "/env",
        search: { ...searchBase, tab: "ops" },
        active: tab === "ops",
      },
      {
        id: "topology",
        label: t("nav.topology"),
        to: "/env",
        search: { ...searchBase, tab: "topology" },
        active: tab === "topology",
      },
      {
        id: "config",
        label: t("nav.config"),
        to: "/env",
        search: { ...searchBase, tab: "config" },
        active: tab === "config",
      },
      {
        id: "debug",
        label: t("nav.debug"),
        to: "/env",
        search: { ...searchBase, tab: "debug" },
        active: tab === "debug",
      },
    ],
    groups,
    showSettingsBottom: false,
  };
}

function buildVm(
  t: TFunction,
  opts: {
    vmId: string;
    stackPath?: string;
    section: VmSection;
    containerLevel?: "list" | "detail";
    showContainers: boolean;
    running: boolean;
    logSource?: string;
    vmLogStreams?: VmLogStream[];
  },
): SidebarNavModel {
  const {
    vmId,
    stackPath,
    section,
    containerLevel,
    showContainers,
    running,
    logSource,
    vmLogStreams = [],
  } = opts;
  const encoded = encodeVmIdParam(vmId);
  const vmSearch = stackPath ? { stackPath } : {};
  const onContainers = containerLevel != null;
  const backLabel = t("nav.back");

  let back: SidebarBack;
  if (onContainers) {
    back = {
      label: backLabel,
      to: "/vms/$vmId",
      params: { vmId: encoded },
      search: vmSearch,
    };
  } else if (stackPath) {
    back = {
      label: backLabel,
      to: "/env",
      search: { path: stackPath, tab: "ops" },
    };
  } else {
    back = { label: backLabel, to: "/vms" };
  }

  const items: SidebarNavItem[] = [
    {
      id: "overview",
      label: t("nav.overview"),
      to: "/vms/$vmId",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "overview",
    },
    {
      id: "services",
      label: t("nav.services"),
      to: "/vms/$vmId/services",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "services",
      disabled: !running,
    },
    {
      id: "shell",
      label: t("vmDetail.shell"),
      to: "/vms/$vmId/shell",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "shell",
      disabled: !running,
    },
    {
      id: "console",
      label: t("vmDetail.attach"),
      to: "/vms/$vmId/console",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "console",
      disabled: !running,
    },
    {
      id: "modify",
      label: t("vmDetail.modify"),
      to: "/vms/$vmId/modify",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "modify",
    },
    {
      id: "mount",
      label: t("vmDetail.mount"),
      to: "/vms/$vmId/mount",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "mount",
    },
    {
      id: "replace",
      label: t("vmDetail.replace"),
      to: "/vms/$vmId/replace",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "replace",
    },
  ];

  if (showContainers) {
    items.push({
      id: "containers",
      label: t("nav.containers"),
      to: "/vms/$vmId/containers",
      params: { vmId: encoded },
      search: vmSearch,
      active: onContainers,
    });
  }

  items.push({
    id: "delete",
    label: t("vmDetail.delete"),
    to: "/vms/$vmId",
    params: { vmId: encoded },
    search: vmSearch,
    kind: "action",
    tone: "danger",
  });

  const groups: SidebarNavGroup[] = [];
  if (vmLogStreams.length > 0) {
    groups.push({
      id: "logs",
      label: t("nav.logs"),
      items: buildVmLogStreamNavItems(vmId, vmLogStreams, {
        stackPath,
        section,
        activeSource: logSource,
        running,
      }),
    });
  }

  return {
    context: "vm",
    contextKey: `vm:${vmId}`,
    title: vmId,
    back,
    showDashboard: true,
    items,
    groups,
    showSettingsBottom: false,
  };
}

export function resolveSidebarNav(
  location: SidebarLocationInput,
  opts?: {
    hasDockerRole?: boolean;
    running?: boolean;
    stackTitle?: string | null;
    vmLogStreams?: VmLogStream[];
    stackLogStreams?: StackLogStream[];
    activeLog?: { vmId: string; source: string };
    t?: TFunction;
  },
): SidebarNavModel {
  const t = opts?.t ?? getT();
  const parsed = parseLocation(location);

  if (parsed.kind === "settings") return buildSettings(t);
  if (parsed.kind === "stack") {
    return buildStack(t, parsed.envPath, parsed.tab ?? "ops", {
      titleOverride: opts?.stackTitle,
      stackLogStreams: opts?.stackLogStreams,
      activeLog: opts?.activeLog,
    });
  }
  if (parsed.kind === "stack-logs" && parsed.stackPath) {
    return buildStack(t, parsed.stackPath, "ops", {
      titleOverride: opts?.stackTitle,
      stackLogStreams: opts?.stackLogStreams,
      activeLog:
        parsed.vmId && parsed.logSource
          ? { vmId: parsed.vmId, source: parsed.logSource }
          : opts?.activeLog,
    });
  }
  if (parsed.kind === "vm" && parsed.vmId) {
    const onContainers = parsed.containerLevel != null;
    const showContainers =
      onContainers || opts?.hasDockerRole === true;
    return buildVm(t, {
      vmId: parsed.vmId,
      stackPath: parsed.stackPath,
      section: parsed.section ?? "overview",
      containerLevel: parsed.containerLevel,
      showContainers,
      running: opts?.running === true,
      logSource: parsed.logSource,
      vmLogStreams: opts?.vmLogStreams,
    });
  }
  return buildRoot(t);
}

function pickVmFromMatches(
  matches: ReadonlyArray<{ routeId: string; params: Record<string, unknown> }>,
): { routeVmId?: string; onContainers: boolean } {
  let routeVmId: string | undefined;
  let onContainers = false;
  for (const match of matches) {
    const id = match.routeId;
    if (id === "/vms/$vmId" || id.startsWith("/vms/$vmId/")) {
      const raw = match.params.vmId;
      if (typeof raw === "string" && raw.length > 0) {
        routeVmId = decodeVmIdParam(raw);
      }
    }
    if (id.includes("/containers")) {
      onContainers = true;
    }
  }
  return { routeVmId, onContainers };
}

export function useSidebarNav(): SidebarNavModel {
  const t = useT();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const search = useRouterState({
    select: (s) => s.location.search as Record<string, unknown>,
  });
  const matchInfo = useRouterState({
    select: (s) =>
      pickVmFromMatches(
        s.matches.map((m) => ({
          routeId: m.routeId,
          params: m.params as Record<string, unknown>,
        })),
      ),
  });

  const location: SidebarLocationInput = {
    pathname,
    search,
    routeVmId: matchInfo.routeVmId,
    onContainers: matchInfo.onContainers,
  };

  const parsed = parseLocation(location);
  const vmId = parsed.vmId;
  const onContainers = parsed.kind === "vm" && parsed.containerLevel != null;
  const needsVmInspect = parsed.kind === "vm" && Boolean(vmId);

  const inspectQuery = useQuery({
    queryKey: vmKeys.detail(vmId ?? ""),
    queryFn: () => inspectVm(vmId!),
    enabled: Boolean(vmId) && needsVmInspect,
    staleTime: 30_000,
  });

  const hasDockerRole =
    onContainers ||
    inspectQuery.data?.vm.roles?.includes("docker") === true;
  const running = isRunning(inspectQuery.data?.vm.state);

  const stackPathForStreams =
    parsed.kind === "stack"
      ? parsed.envPath
      : parsed.kind === "stack-logs"
        ? parsed.stackPath
        : undefined;

  const stackLogStreamsQuery = useStackLogStreams(stackPathForStreams);

  const vmLogStreamsQuery = useVmLogStreams(
    parsed.kind === "vm" ? vmId : undefined,
    parsed.stackPath,
    running,
  );

  let stackTitle: string | null | undefined;
  if ((parsed.kind === "stack" || parsed.kind === "stack-logs") && stackPathForStreams) {
    stackTitle =
      getProject(stackPathForStreams)?.name ?? basename(stackPathForStreams);
  }

  return resolveSidebarNav(location, {
    hasDockerRole,
    running,
    stackTitle,
    vmLogStreams: vmLogStreamsQuery.data,
    stackLogStreams: stackLogStreamsQuery.data,
    t,
  });
}
