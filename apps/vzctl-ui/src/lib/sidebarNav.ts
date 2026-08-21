import { useQuery } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { getT, useT, type TFunction } from "@/lib/i18n";
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
  kind: "root" | "stack" | "vm" | "settings";
  vmId?: string;
  containerLevel?: "list" | "detail";
  section?: VmSection;
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
    const parsed = parseVmSection(vmTail(pathname));
    return {
      kind: "vm",
      vmId: input.routeVmId,
      containerLevel: parsed.containerLevel,
      section: parsed.section,
      stackPath: asString(search.stackPath),
    };
  }

  // Fallback for pure resolveSidebarNav callers (tests / no matches).
  const vmMatch = pathname.match(/^\/vms\/([^/]+)(?:\/(.*))?$/);
  if (vmMatch) {
    const parsed = parseVmSection(vmMatch[2] ?? "");
    return {
      kind: "vm",
      vmId: decodeVmIdParam(vmMatch[1]),
      containerLevel: parsed.containerLevel,
      section: parsed.section,
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
    section: VmSection;
    containerLevel?: "list" | "detail";
    showContainers: boolean;
    running: boolean;
  },
): SidebarNavModel {
  const { vmId, stackPath, section, containerLevel, showContainers, running } =
    opts;
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
      id: "logs",
      label: t("nav.logs"),
      to: "/vms/$vmId/logs",
      params: { vmId: encoded },
      search: vmSearch,
      active: section === "logs",
      disabled: !running,
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
    running?: boolean;
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
      section: parsed.section ?? "overview",
      containerLevel: parsed.containerLevel,
      showContainers,
      running: opts?.running === true,
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
  const vmId = parsed.kind === "vm" ? parsed.vmId : undefined;
  const onContainers = parsed.kind === "vm" && parsed.containerLevel != null;

  const inspectQuery = useQuery({
    queryKey: vmKeys.detail(vmId ?? ""),
    queryFn: () => inspectVm(vmId!),
    enabled: Boolean(vmId),
    staleTime: 30_000,
  });

  const hasDockerRole =
    onContainers ||
    inspectQuery.data?.vm.roles?.includes("docker") === true;
  const running = isRunning(inspectQuery.data?.vm.state);

  let stackTitle: string | null | undefined;
  if (parsed.kind === "stack" && parsed.envPath) {
    stackTitle = getProject(parsed.envPath)?.name ?? basename(parsed.envPath);
  }

  return resolveSidebarNav(location, {
    hasDockerRole,
    running,
    stackTitle,
    t,
  });
}
