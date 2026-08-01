import { useQuery } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { getProject } from "@/lib/projects";
import { basename } from "@/lib/vzctl";
import {
  decodeVmIdParam,
  encodeVmIdParam,
  inspectVm,
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
  /** Settings link in sidebar-bottom (root only). */
  showSettingsBottom: boolean;
};

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

function parseLocation(input: SidebarLocationInput): {
  kind: "root" | "stack" | "vm" | "settings";
  vmId?: string;
  containerLevel?: "list" | "detail";
  stackPath?: string;
  envPath?: string;
  tab?: "ops" | "topology" | "config";
} {
  const { pathname, search } = input;

  if (pathname === "/settings" || pathname.startsWith("/settings/")) {
    return { kind: "settings" };
  }
  if (pathname === "/env" || pathname.startsWith("/env/")) {
    const tabRaw = search.tab;
    const tab =
      tabRaw === "topology" || tabRaw === "ops" || tabRaw === "config"
        ? tabRaw
        : "ops";
    return {
      kind: "stack",
      envPath: asString(search.path),
      tab,
    };
  }

  if (input.routeVmId) {
    let containerLevel: "list" | "detail" | undefined;
    if (input.onContainers) {
      containerLevel = pathname.includes("/containers/") ? "detail" : "list";
    }
    return {
      kind: "vm",
      vmId: input.routeVmId,
      containerLevel,
      stackPath: asString(search.stackPath),
    };
  }

  // Fallback for pure resolveSidebarNav callers (tests / no matches).
  const containerDetail = pathname.match(
    /^\/vms\/([^/]+)\/containers\/([^/]+)/,
  );
  if (containerDetail) {
    return {
      kind: "vm",
      vmId: decodeVmIdParam(containerDetail[1]),
      containerLevel: "detail",
      stackPath: asString(search.stackPath),
    };
  }

  const containers = pathname.match(/^\/vms\/([^/]+)\/containers\/?$/);
  if (containers) {
    return {
      kind: "vm",
      vmId: decodeVmIdParam(containers[1]),
      containerLevel: "list",
      stackPath: asString(search.stackPath),
    };
  }

  const vmOnly = pathname.match(/^\/vms\/([^/]+)$/);
  if (vmOnly) {
    return {
      kind: "vm",
      vmId: decodeVmIdParam(vmOnly[1]),
      stackPath: asString(search.stackPath),
    };
  }

  return { kind: "root" };
}

const ROOT_ITEMS: SidebarNavItem[] = [
  { id: "dashboard", label: "Dashboard", to: "/", exact: true },
  { id: "vms", label: "VMs", to: "/vms" },
  { id: "projects", label: "Stacks", to: "/projects" },
  { id: "networks", label: "Networks", to: "/networks" },
  { id: "images", label: "Images", to: "/images" },
  { id: "doctor", label: "Doctor", to: "/doctor" },
];

function buildRoot(): SidebarNavModel {
  return {
    context: "root",
    contextKey: "root",
    title: null,
    back: null,
    showDashboard: false,
    items: ROOT_ITEMS,
    showSettingsBottom: true,
  };
}

function buildSettings(): SidebarNavModel {
  return {
    context: "settings",
    contextKey: "settings",
    title: "Settings",
    back: { label: "Zurück", to: "/" },
    showDashboard: true,
    items: [
      {
        id: "settings",
        label: "Settings",
        to: "/settings",
        exact: true,
        active: true,
      },
    ],
    showSettingsBottom: false,
  };
}

function buildStack(
  envPath: string | undefined,
  tab: "ops" | "topology" | "config",
  titleOverride?: string | null,
): SidebarNavModel {
  const title =
    titleOverride ??
    (envPath ? basename(envPath) : "Stack");
  const searchBase = envPath ? { path: envPath } : { path: "" };

  return {
    context: "stack",
    contextKey: `stack:${envPath ?? ""}`,
    title,
    back: { label: "Zurück", to: "/projects" },
    showDashboard: true,
    items: [
      {
        id: "ops",
        label: "Betrieb",
        to: "/env",
        search: { ...searchBase, tab: "ops" },
        active: tab === "ops",
      },
      {
        id: "topology",
        label: "Topologie",
        to: "/env",
        search: { ...searchBase, tab: "topology" },
        active: tab === "topology",
      },
      {
        id: "config",
        label: "Config",
        to: "/env",
        search: { ...searchBase, tab: "config" },
        active: tab === "config",
      },
    ],
    showSettingsBottom: false,
  };
}

function buildVm(opts: {
  vmId: string;
  stackPath?: string;
  containerLevel?: "list" | "detail";
  showContainers: boolean;
}): SidebarNavModel {
  const { vmId, stackPath, containerLevel, showContainers } = opts;
  const encoded = encodeVmIdParam(vmId);
  const vmSearch = stackPath ? { stackPath } : {};
  const onContainers = containerLevel != null;

  let back: SidebarBack;
  if (onContainers) {
    back = {
      label: "Zurück",
      to: "/vms/$vmId",
      params: { vmId: encoded },
      search: vmSearch,
    };
  } else if (stackPath) {
    back = {
      label: "Zurück",
      to: "/env",
      search: { path: stackPath, tab: "ops" },
    };
  } else {
    back = { label: "Zurück", to: "/vms" };
  }

  const items: SidebarNavItem[] = [
    {
      id: "overview",
      label: "Übersicht",
      to: "/vms/$vmId",
      params: { vmId: encoded },
      search: vmSearch,
      active: !onContainers,
    },
  ];

  if (showContainers) {
    items.push({
      id: "containers",
      label: "Containers",
      to: "/vms/$vmId/containers",
      params: { vmId: encoded },
      search: vmSearch,
      active: onContainers,
    });
  }

  return {
    context: "vm",
    contextKey: `vm:${vmId}`,
    title: vmId,
    back,
    showDashboard: true,
    items,
    showSettingsBottom: false,
  };
}

export function resolveSidebarNav(
  location: SidebarLocationInput,
  opts?: { hasDockerRole?: boolean; stackTitle?: string | null },
): SidebarNavModel {
  const parsed = parseLocation(location);

  if (parsed.kind === "settings") return buildSettings();
  if (parsed.kind === "stack") {
    return buildStack(parsed.envPath, parsed.tab ?? "ops", opts?.stackTitle);
  }
  if (parsed.kind === "vm" && parsed.vmId) {
    const onContainers = parsed.containerLevel != null;
    const showContainers =
      onContainers || opts?.hasDockerRole === true;
    return buildVm({
      vmId: parsed.vmId,
      stackPath: parsed.stackPath,
      containerLevel: parsed.containerLevel,
      showContainers,
    });
  }
  return buildRoot();
}

function pickVmFromMatches(
  matches: ReadonlyArray<{ routeId: string; params: Record<string, unknown> }>,
): { routeVmId?: string; onContainers: boolean } {
  let routeVmId: string | undefined;
  let onContainers = false;
  for (const match of matches) {
    const id = match.routeId;
    if (
      id === "/vms/$vmId" ||
      id === "/vms/$vmId/containers" ||
      id === "/vms/$vmId/containers/$containerId"
    ) {
      const raw = match.params.vmId;
      if (typeof raw === "string" && raw.length > 0) {
        routeVmId = decodeVmIdParam(raw);
      }
    }
    if (
      id === "/vms/$vmId/containers" ||
      id === "/vms/$vmId/containers/$containerId"
    ) {
      onContainers = true;
    }
  }
  return { routeVmId, onContainers };
}

export function useSidebarNav(): SidebarNavModel {
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
  const vmId = parsed.kind === "vm" ? parsed.vmId : undefined;
  const onContainers = parsed.kind === "vm" && parsed.containerLevel != null;

  const inspectQuery = useQuery({
    queryKey: vmKeys.detail(vmId ?? ""),
    queryFn: () => inspectVm(vmId!),
    enabled: Boolean(vmId) && !onContainers,
    staleTime: 30_000,
  });

  const hasDockerRole =
    onContainers ||
    inspectQuery.data?.vm.roles?.includes("docker") === true;

  let stackTitle: string | null | undefined;
  if (parsed.kind === "stack" && parsed.envPath) {
    stackTitle = getProject(parsed.envPath)?.name ?? basename(parsed.envPath);
  }

  return resolveSidebarNav(location, { hasDockerRole, stackTitle });
}
