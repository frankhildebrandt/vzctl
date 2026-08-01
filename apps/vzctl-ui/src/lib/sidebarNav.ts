import { useQuery } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { getT, useT, type TFunction } from "@/lib/i18n";
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
    // Kein Zurück: Escape ist bereits „← Dashboard“ in der Brand-Zeile.
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
  tab: "ops" | "topology" | "config",
  titleOverride?: string | null,
): SidebarNavModel {
  const title =
    titleOverride ??
    (envPath ? basename(envPath) : t("nav.stackFallback"));
  const searchBase = envPath ? { path: envPath } : { path: "" };

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
    ],
    showSettingsBottom: false,
  };
}

function buildVm(
  t: TFunction,
  opts: {
    vmId: string;
    stackPath?: string;
    containerLevel?: "list" | "detail";
    showContainers: boolean;
  },
): SidebarNavModel {
  const { vmId, stackPath, containerLevel, showContainers } = opts;
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
      active: !onContainers,
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
  opts?: {
    hasDockerRole?: boolean;
    stackTitle?: string | null;
    t?: TFunction;
  },
): SidebarNavModel {
  const t = opts?.t ?? getT();
  const parsed = parseLocation(location);

  if (parsed.kind === "settings") return buildSettings(t);
  if (parsed.kind === "stack") {
    return buildStack(t, parsed.envPath, parsed.tab ?? "ops", opts?.stackTitle);
  }
  if (parsed.kind === "vm" && parsed.vmId) {
    const onContainers = parsed.containerLevel != null;
    const showContainers =
      onContainers || opts?.hasDockerRole === true;
    return buildVm(t, {
      vmId: parsed.vmId,
      stackPath: parsed.stackPath,
      containerLevel: parsed.containerLevel,
      showContainers,
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

  return resolveSidebarNav(location, { hasDockerRole, stackTitle, t });
}
