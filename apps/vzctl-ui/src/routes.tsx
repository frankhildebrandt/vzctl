import {
  Link,
  Outlet,
  createRootRoute,
  createRoute,
  redirect,
} from "@tanstack/react-router";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { AppShell } from "@/components/AppShell";
import { TerminalSessionRoot } from "@/components/TerminalDock";
import { ApplyProgress, ConsoleLog, useApplyProgress } from "@/components/ApplyProgress";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ChromeCrumbs } from "@/components/Chrome";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  VmnetOrphanDialog,
  type VmnetOrphanChoice,
} from "@/components/VmnetOrphanDialog";
import {
  IconApply,
  IconButton,
  IconDiff,
  IconPlay,
  IconPurge,
  IconStatus,
  IconStop,
  IconTrash,
} from "@/components/IconButton";
import { DoctorPage } from "@/components/DoctorPage";
import { ErrorsPage } from "@/components/ErrorsPage";
import { ResultPanel, type ResultModel } from "@/components/ResultPanel";
import { IngressLinksCard } from "@/components/IngressLinks";
import { SettingsPage } from "@/components/SettingsPage";
import { ProjectOidcUplinkSection } from "@/components/ProjectOidcUplinkSection";
import { StackStatusCard } from "@/components/StackStatus";
import { DashboardPage } from "@/components/pages";
import { ContainerDetailPage } from "@/components/ContainerDetailPage";
import { ContainersPage } from "@/components/ContainersPage";
import { VmServicesPage } from "@/components/VmServicesPage";
import { ImagesPage } from "@/components/ImagesPage";
import { NetworksPage } from "@/components/NetworksPage";
import { VmDetailLayout } from "@/components/VmDetailLayout";
import { VmLogsPage } from "@/components/VmLogsPage";
import {
  VmModifyPage,
  VmMountPage,
  VmOverviewPage,
  VmReplacePage,
} from "@/components/VmDetailPage";
import { VmListPage } from "@/components/VmListPage";
import {
  Alert,
  ActionRow,
  Button,
  Card,
  EmptyState,
  FormField,
  Muted,
  PageHeader,
} from "@/components/ui";
import { parseIngressInfo } from "@/lib/ingress";
import {
  forgetProject,
  formatOpenedAt,
  getProject,
  listProjects,
  projectKeys,
  rememberProject,
} from "@/lib/projects";
import { deriveStackStatus, parseStackInventory } from "@/lib/stackStatus";
import { decodeVmIdParam } from "@/lib/vms";
import {
  basename,
  pickEnvironment,
  queryKeys,
  runVzctl,
  type VzctlCommand,
} from "@/lib/vzctl";
import {
  parseVmnetOrphanError,
  suggestReplacementCidr,
  type VmnetOrphanInfo,
} from "@/lib/vmnetOrphan";
import {
  recoverOrphanByCidrChange,
  requestHostReboot,
} from "@/lib/vmnetOrphanRecovery";
import { TopologyEditor } from "@/features/topology-editor/TopologyEditor";
import { useT } from "@/lib/i18n";
import { useEditorStore } from "@/store/editorStore";
import { DEMO_PROJECT_PATH, enableDemoMode } from "@/lib/demo";

export const rootRoute = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  return (
    <TerminalSessionRoot>
      <AppShell>
        <Outlet />
      </AppShell>
    </TerminalSessionRoot>
  );
}

type EnvSearch = {
  path: string;
  tab?: "topology" | "ops" | "config";
};

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: DashboardPage,
});

export const vmsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/vms",
  component: VmListPage,
});

function vmStackSearch(search: Record<string, unknown>): { stackPath?: string } {
  return {
    stackPath:
      typeof search.stackPath === "string" && search.stackPath.length > 0
        ? search.stackPath
        : undefined,
  };
}

export const vmDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/vms/$vmId",
  validateSearch: vmStackSearch,
  component: function VmDetailLayoutRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { stackPath } = vmDetailRoute.useSearch();
    return (
      <VmDetailLayout
        vmId={decodeVmIdParam(rawVmId)}
        stackPath={stackPath}
      />
    );
  },
});

export const vmOverviewRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "/",
  component: function VmOverviewRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    return <VmOverviewPage vmId={decodeVmIdParam(rawVmId)} />;
  },
});

export const vmLogsRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "logs",
  validateSearch: (search: Record<string, unknown>) => ({
    ...vmStackSearch(search),
    source:
      typeof search.source === "string" && search.source.length > 0
        ? search.source
        : undefined,
  }),
  component: function VmLogsRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { source } = vmLogsRoute.useSearch();
    return <VmLogsPage vmId={decodeVmIdParam(rawVmId)} source={source} />;
  },
});

export const vmShellRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "shell",
  component: function VmShellRoute() {
    return null;
  },
});

export const vmConsoleRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "console",
  component: function VmConsoleRoute() {
    return null;
  },
});

export const vmModifyRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "modify",
  component: function VmModifyRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { stackPath } = vmDetailRoute.useSearch();
    return (
      <VmModifyPage vmId={decodeVmIdParam(rawVmId)} stackPath={stackPath} />
    );
  },
});

export const vmMountRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "mount",
  component: function VmMountRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { stackPath } = vmDetailRoute.useSearch();
    return (
      <VmMountPage vmId={decodeVmIdParam(rawVmId)} stackPath={stackPath} />
    );
  },
});

export const vmReplaceRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "replace",
  component: function VmReplaceRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { stackPath } = vmDetailRoute.useSearch();
    return (
      <VmReplacePage vmId={decodeVmIdParam(rawVmId)} stackPath={stackPath} />
    );
  },
});

export const vmServicesRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "services",
  component: function VmServicesRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    return <VmServicesPage vmId={decodeVmIdParam(rawVmId)} />;
  },
});

export const vmContainersRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "containers",
  component: function VmContainersRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { stackPath } = vmDetailRoute.useSearch();
    return (
      <ContainersPage
        vmId={decodeVmIdParam(rawVmId)}
        stackPath={stackPath}
      />
    );
  },
});

export const vmContainerDetailRoute = createRoute({
  getParentRoute: () => vmDetailRoute,
  path: "containers/$containerId",
  component: function VmContainerDetailRoute() {
    const { vmId: rawVmId, containerId: rawContainerId } =
      vmContainerDetailRoute.useParams();
    const { stackPath } = vmContainerDetailRoute.useSearch();
    return (
      <ContainerDetailPage
        vmId={decodeVmIdParam(rawVmId)}
        containerId={decodeURIComponent(rawContainerId)}
        stackPath={stackPath}
      />
    );
  },
});

export const projectsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects",
  component: ProjectListPage,
});

export const networksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/networks",
  component: NetworksPage,
});

export const doctorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/doctor",
  component: DoctorPage,
});

export const errorsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/errors",
  component: ErrorsPage,
});

export const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsPage,
});

export const demoRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/demo",
  beforeLoad: () => {
    enableDemoMode();
    throw redirect({
      to: "/env",
      search: { path: DEMO_PROJECT_PATH, tab: "ops" },
    });
  },
  component: () => null,
});

export const imagesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/images",
  component: ImagesPage,
});

export const envRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/env",
  validateSearch: (search: Record<string, unknown>): EnvSearch => ({
    path: typeof search.path === "string" ? search.path : "",
    tab:
      search.tab === "topology" || search.tab === "ops" || search.tab === "config"
        ? search.tab
        : undefined,
  }),
  beforeLoad: ({ search }) => {
    if (!search.path) {
      throw redirect({ to: "/projects" });
    }
  },
  component: ProjectDetailPage,
});

function ProjectListPage() {
  const t = useT();
  const navigate = projectsRoute.useNavigate();
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [createName, setCreateName] = useState("");
  const [pendingRemove, setPendingRemove] = useState<{
    path: string;
    name: string;
  } | null>(null);

  const projectsQuery = useQuery({
    queryKey: projectKeys.all,
    queryFn: listProjects,
  });

  const addMutation = useMutation({
    mutationFn: async () => {
      const path = await pickEnvironment();
      if (!path) return null;
      return rememberProject(path);
    },
    onSuccess: async (projects) => {
      if (!projects) return;
      await queryClient.invalidateQueries({ queryKey: projectKeys.all });
      const path = projects[0]?.path;
      if (path) {
        await navigate({ to: "/env", search: { path, tab: "ops" } });
      }
    },
    onError: (err) => setError(String(err)),
  });

  const createMutation = useMutation({
    mutationFn: async () => {
      const name = createName.trim();
      if (!name) throw new Error(t("projects.nameRequired"));
      const { pickDirectory } = await import("@/lib/dialogs");
      const parent = await pickDirectory(t("projects.pickParent"));
      if (!parent) return null;
      const sep = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
      const slug = name
        .toLowerCase()
        .replace(/[^a-z0-9._-]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 63);
      const path = `${parent.replace(/[/\\]+$/, "")}${sep}${slug}`;
      const { createProjectFlexible } = await import(
        "@/features/persistence/projectIo"
      );
      await createProjectFlexible(path, slug);
      await rememberProject(path);
      return path;
    },
    onSuccess: async (path) => {
      if (!path) return;
      setCreateName("");
      await queryClient.invalidateQueries({ queryKey: projectKeys.all });
      await navigate({ to: "/env", search: { path, tab: "ops" } });
    },
    onError: (err) => setError(String(err)),
  });

  const removeMutation = useMutation({
    mutationFn: async (path: string) => forgetProject(path),
    onSuccess: async () => {
      setPendingRemove(null);
      await queryClient.invalidateQueries({ queryKey: projectKeys.all });
    },
  });

  const projects = projectsQuery.data ?? [];
  const busy =
    addMutation.isPending ||
    removeMutation.isPending ||
    createMutation.isPending;

  return (
    <section>
      <PageHeader
        title={t("projects.title")}
        subtitle={t("projects.subtitle")}
        actions={
          <Button
            tone="secondary"
            disabled={busy}
            onClick={() => {
              setError(null);
              addMutation.mutate();
            }}
          >
            {addMutation.isPending ? t("projects.openBusy") : t("projects.open")}
          </Button>
        }
      />

      <Card
        className="create-project-card"
        title={t("projects.newTitle")}
        titleAs="h3"
        subtitle={t("projects.newHint")}
      >
        <ActionRow gap="md" style={{ flexWrap: "wrap" }}>
          <FormField
            label={<span className="sr-only">{t("projects.nameLabel")}</span>}
            variant="compact"
            style={{ flex: "1 1 12rem" }}
          >
            <input
              value={createName}
              onChange={(e) => setCreateName(e.target.value)}
              placeholder={t("projects.namePlaceholder")}
              aria-label={t("projects.nameLabel")}
              disabled={busy}
            />
          </FormField>
          <Button
            disabled={busy || !createName.trim()}
            onClick={() => {
              setError(null);
              createMutation.mutate();
            }}
          >
            {createMutation.isPending ? t("projects.createBusy") : t("projects.create")}
          </Button>
        </ActionRow>
      </Card>

      {error ? (
        <Alert title={t("common.error")}>{error}</Alert>
      ) : null}

      {projects.length === 0 ? (
        <EmptyState
          title={t("projects.emptyTitle")}
          message={t("projects.emptyHint")}
        />
      ) : (
        <ul className="project-list">
          {projects.map((project) => (
            <li key={project.path} className="project-item">
              <Link
                to="/env"
                search={{ path: project.path, tab: "ops" }}
                className="project-link"
              >
                <span className="project-name">{project.name}</span>
                <span className="path">{project.path}</span>
                <Muted as="span" className="project-meta">
                  {t("stack.openedAt", { date: formatOpenedAt(project.openedAt) })}
                </Muted>
              </Link>
              <Button
                tone="secondary"
                disabled={busy}
                onClick={() =>
                  setPendingRemove({ path: project.path, name: project.name })
                }
              >
                {t("projects.remove")}
              </Button>
            </li>
          ))}
        </ul>
      )}

      <ConfirmDialog
        open={pendingRemove != null}
        title={t("projects.removeTitle")}
        message={
          pendingRemove
            ? t("projects.removeMessage", { name: pendingRemove.name })
            : ""
        }
        confirmLabel={t("projects.removeLabel")}
        busy={removeMutation.isPending}
        onCancel={() => {
          if (!removeMutation.isPending) setPendingRemove(null);
        }}
        onConfirm={() => {
          if (pendingRemove) removeMutation.mutate(pendingRemove.path);
        }}
      />
    </section>
  );
}

function ProjectDetailPage() {
  const t = useT();
  const { path, tab: tabParam } = envRoute.useSearch();
  const tab = tabParam ?? "ops";
  const navigate = envRoute.useNavigate();
  const queryClient = useQueryClient();
  const [result, setResult] = useState<ResultModel>({ kind: "idle", raw: "" });
  const [busyLabel, setBusyLabel] = useState<string | null>(null);
  const progress = useApplyProgress(true);
  const [logOpen, setLogOpen] = useState(false);
  const [confirm, setConfirm] = useState<
    null | "up" | "apply" | "down" | "purge" | "remove"
  >(null);
  const [orphan, setOrphan] = useState<VmnetOrphanInfo | null>(null);
  const [orphanBusy, setOrphanBusy] = useState(false);
  const [orphanError, setOrphanError] = useState<string | null>(null);
  const [topologyError, setTopologyError] = useState<string | null>(null);
  const [topologyToolbarHost, setTopologyToolbarHost] =
    useState<HTMLDivElement | null>(null);
  const loadEditor = useEditorStore((s) => s.load);
  const resetEditor = useEditorStore((s) => s.reset);

  const projectsQuery = useQuery({
    queryKey: projectKeys.all,
    queryFn: listProjects,
  });
  const project =
    projectsQuery.data?.find((entry) => entry.path === path) ??
    getProject(path);
  const title = project?.name ?? basename(path);

  useEffect(() => {
    rememberProject(path);
    void queryClient.invalidateQueries({ queryKey: projectKeys.all });
    setResult({ kind: "idle", raw: "" });
    setBusyLabel(t("stack.statusLoading"));
    setLogOpen(false);
    progress.reset();
    setTopologyError(null);
    let cancelled = false;
    void (async () => {
      try {
        const { loadProjectFlexible } = await import(
          "@/features/persistence/projectIo"
        );
        const { env, diagram } = await loadProjectFlexible(path);
        if (!cancelled) loadEditor(path, env, diagram);
      } catch (err) {
        if (!cancelled) {
          setTopologyError(String(err instanceof Error ? err.message : err));
          resetEditor();
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // progress.reset is stable enough for path switches; omit from deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, queryClient, loadEditor, resetEditor]);

  const statusQuery = useQuery({
    queryKey: queryKeys.status(path),
    queryFn: () => runVzctl(path, "status"),
    enabled: Boolean(path),
    refetchInterval: progress.state.active ? 4000 : 8000,
  });

  const inventory = useMemo(
    () => parseStackInventory(statusQuery.data ?? (result.kind === "status" ? result.raw : null)),
    [statusQuery.data, result.kind, result.raw],
  );

  const ingress = useMemo(
    () => parseIngressInfo(statusQuery.data ?? (result.kind === "status" ? result.raw : null)),
    [statusQuery.data, result.kind, result.raw],
  );

  const stack = useMemo(
    () =>
      deriveStackStatus({
        inventory,
        applyActive: progress.state.active,
        applyMode: progress.state.mode,
        applyFailed: Boolean(progress.state.finished && progress.state.error),
        t,
      }),
    [
      inventory,
      progress.state.active,
      progress.state.mode,
      progress.state.finished,
      progress.state.error,
      t,
    ],
  );

  const diffQuery = useQuery({
    queryKey: queryKeys.diff(path),
    queryFn: () => runVzctl(path, "diff"),
    enabled: false,
  });

  useEffect(() => {
    if (statusQuery.isLoading) {
      setBusyLabel(t("stack.statusLoading"));
      return;
    }
    if (statusQuery.isError) {
      setBusyLabel(null);
      setResult({ kind: "error", raw: String(statusQuery.error) });
      return;
    }
    if (statusQuery.data) {
      setBusyLabel(null);
      setResult({ kind: "status", raw: statusQuery.data });
    }
  }, [
    statusQuery.isLoading,
    statusQuery.isError,
    statusQuery.error,
    statusQuery.data,
  ]);

  const suggestedCidr = useMemo(() => {
    if (!orphan) return "";
    try {
      return suggestReplacementCidr(orphan.cidr, [orphan.cidr]);
    } catch {
      return "10.78.0.0/24";
    }
  }, [orphan]);

  const mutate = useMutation({
    mutationFn: ({
      command,
      force,
      purge,
    }: {
      command: VzctlCommand;
      force?: boolean;
      purge?: boolean;
    }) => runVzctl(path, command, { force, purge }),
    onMutate: ({ command, purge }) => {
      setBusyLabel(purge ? t("apply.busy.down") : t("apply.busy.mode", { mode: command }));
      setOrphan(null);
      setOrphanError(null);
      if (command === "up" || command === "apply" || command === "down") {
        setLogOpen(true);
        progress.begin(command);
      }
    },
    onSuccess: (_data, { command }) => {
      setBusyLabel(null);
      rememberProject(path);
      if (command === "up" || command === "apply" || command === "down") {
        progress.end(true);
        setLogOpen(false);
        void queryClient.invalidateQueries({ queryKey: queryKeys.diff(path) });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.status(path),
        });
      }
    },
    onError: (err, { command }) => {
      setBusyLabel(null);
      const info = parseVmnetOrphanError(err);
      if (info && (command === "up" || command === "apply")) {
        setOrphan(info);
        setResult({ kind: "error", raw: String(err) });
        progress.end(false);
        setLogOpen(true);
        return;
      }
      setResult({ kind: "error", raw: String(err) });
      if (command === "up" || command === "apply" || command === "down") {
        progress.end(false);
        // Keep log open on failure so the error stays visible.
        setLogOpen(true);
      }
    },
  });

  const removeMutation = useMutation({
    mutationFn: async () => forgetProject(path),
    onSuccess: async () => {
      setConfirm(null);
      await queryClient.invalidateQueries({ queryKey: projectKeys.all });
      await navigate({ to: "/projects" });
    },
  });

  const busy =
    mutate.isPending ||
    statusQuery.isFetching ||
    diffQuery.isFetching ||
    removeMutation.isPending ||
    orphanBusy;

  async function runStatus() {
    setBusyLabel(t("stack.statusLoading"));
    try {
      const response = await statusQuery.refetch();
      if (response.error) throw response.error;
      setResult({ kind: "status", raw: response.data ?? "" });
      rememberProject(path);
    } catch (err) {
      setResult({ kind: "error", raw: String(err) });
    } finally {
      setBusyLabel(null);
    }
  }

  async function runDiff() {
    setBusyLabel(t("stack.diffLoading"));
    try {
      const response = await diffQuery.refetch();
      if (response.error) throw response.error;
      setResult({ kind: "diff", raw: response.data ?? "" });
      rememberProject(path);
    } catch (err) {
      setResult({ kind: "error", raw: String(err) });
    } finally {
      setBusyLabel(null);
    }
  }

  function confirmCopy(): {
    title: string;
    message: string;
    confirmLabel: string;
    tone: "danger" | "default";
  } {
    switch (confirm) {
      case "up":
        return {
          title: t("stack.confirmUpTitle"),
          message: t("stack.confirmUpMessage"),
          confirmLabel: t("stack.confirmUpLabel"),
          tone: "default",
        };
      case "apply":
        return {
          title: t("stack.confirmApplyTitle"),
          message: t("stack.confirmApplyMessage"),
          confirmLabel: t("stack.confirmApplyLabel"),
          tone: "default",
        };
      case "down":
        return {
          title: t("stack.confirmDownTitle"),
          message: t("stack.confirmDownMessage"),
          confirmLabel: t("stack.confirmDownLabel"),
          tone: "danger",
        };
      case "purge":
        return {
          title: t("stack.confirmPurgeTitle"),
          message: t("stack.confirmPurgeMessage"),
          confirmLabel: t("stack.confirmPurgeLabel"),
          tone: "danger",
        };
      case "remove":
      default:
        return {
          title: t("stack.confirmRemoveTitle"),
          message: t("stack.confirmRemoveMessage", { name: title }),
          confirmLabel: t("stack.confirmRemoveLabel"),
          tone: "danger",
        };
    }
  }

  function runConfirmed() {
    if (confirm === "up") mutate.mutate({ command: "up", force: true });
    else if (confirm === "apply") mutate.mutate({ command: "apply", force: true });
    else if (confirm === "down") mutate.mutate({ command: "down" });
    else if (confirm === "purge") mutate.mutate({ command: "down", purge: true });
    else if (confirm === "remove") removeMutation.mutate();
    if (confirm !== "remove") setConfirm(null);
  }

  async function handleOrphanChoice(choice: VmnetOrphanChoice) {
    if (!orphan) return;
    setOrphanBusy(true);
    setOrphanError(null);
    try {
      if (choice === "reboot") {
        await requestHostReboot();
        setOrphan(null);
        return;
      }
      const result = await recoverOrphanByCidrChange(path, orphan, suggestedCidr);
      setOrphan(null);
      setResult({
        kind: "text",
        raw: t("stack.orphanRecovery", {
          old: orphan.cidr,
          new: result.newCidr,
          nets: result.networkNames.join(", "),
        }),
      });
      mutate.mutate({ command: "up", force: true });
    } catch (err) {
      setOrphanError(String(err));
    } finally {
      setOrphanBusy(false);
    }
  }

  return (
    <section className={`detail${tab === "topology" ? " detail-topology" : ""}`}>
      <ChromeCrumbs>
        <Breadcrumbs
          items={[
            {
              label: t("projects.title"),
              node: (
                <Link to="/projects" className="crumb-link">
                  {t("projects.title")}
                </Link>
              ),
            },
            { label: title },
          ]}
        />
      </ChromeCrumbs>
      <header className="detail-header">
        <div className="detail-heading">
          {tab === "topology" ? (
            <>
              <h2 className="detail-title">{title}</h2>
              <p className="path detail-path">{path}</p>
              {project ? (
                <p className="muted detail-meta">
                  {t("stack.lastOpened", { date: formatOpenedAt(project.openedAt) })}
                </p>
              ) : null}
            </>
          ) : tab === "config" ? (
            <>
              <h2 className="detail-title">{title}</h2>
              <p className="path detail-path">{path}</p>
              <p className="muted detail-meta">{t("stack.configMeta")}</p>
            </>
          ) : null}
        </div>

        <div className="detail-actions">
          {tab === "ops" ? (
            <div
              className="detail-toolbar"
              role="toolbar"
              aria-label={t("stack.toolbarAria")}
            >
              <IconButton
                label={t("stack.diff")}
                showLabel
                disabled={busy}
                tone="quiet"
                onClick={() => void runDiff()}
              >
                <IconDiff />
              </IconButton>
              <IconButton
                label={t("stack.up")}
                showLabel
                disabled={busy}
                tone="quiet"
                onClick={() => setConfirm("up")}
              >
                <IconPlay />
              </IconButton>
              <IconButton
                label={t("stack.apply")}
                showLabel
                disabled={busy}
                tone="primary"
                onClick={() => setConfirm("apply")}
              >
                <IconApply />
              </IconButton>
              <IconButton
                label={t("stack.down")}
                showLabel
                disabled={busy}
                tone="danger"
                onClick={() => setConfirm("down")}
              >
                <IconStop />
              </IconButton>
              <IconButton
                label={t("stack.remove")}
                showLabel
                disabled={busy}
                tone="danger"
                onClick={() => setConfirm("purge")}
              >
                <IconPurge />
              </IconButton>
              <IconButton
                label={t("stack.statusBtn")}
                disabled={busy}
                tone="quiet"
                onClick={() => void runStatus()}
              >
                <IconStatus />
              </IconButton>
              <span className="toolbar-sep" aria-hidden />
              <IconButton
                label={t("stack.removeFromList")}
                disabled={busy}
                tone="quiet"
                onClick={() => setConfirm("remove")}
              >
                <IconTrash />
              </IconButton>
            </div>
          ) : tab === "config" ? (
            <div
              className="detail-toolbar"
              role="toolbar"
              aria-label={t("stack.configToolbarAria")}
            >
              <IconButton
                label={t("stack.removeFromList")}
                disabled={busy}
                tone="quiet"
                onClick={() => setConfirm("remove")}
              >
                <IconTrash />
              </IconButton>
            </div>
          ) : (
            <div
              className="detail-toolbar"
              role="toolbar"
              aria-label={t("stack.topologyToolbarAria")}
            >
              <div
                className="detail-toolbar-slot"
                ref={setTopologyToolbarHost}
              />
              <span className="toolbar-sep" aria-hidden />
              <IconButton
                label={t("stack.removeFromList")}
                disabled={busy}
                tone="quiet"
                onClick={() => setConfirm("remove")}
              >
                <IconTrash />
              </IconButton>
            </div>
          )}
        </div>
      </header>

      {tab === "topology" ? (
        topologyError ? (
          <Alert title={t("stack.topologyLoadFail")}>{topologyError}</Alert>
        ) : (
          <TopologyEditor
            projectPath={path}
            toolbarHost={topologyToolbarHost}
          />
        )
      ) : tab === "config" ? (
        <section>
          <PageHeader
            layout="detail"
            title={t("stack.configTitle")}
            subtitle={t("stack.configSubtitle")}
          />
          <ProjectOidcUplinkSection projectPath={path} />
        </section>
      ) : (
        <>
      <StackStatusCard
        title={title}
        path={path}
        openedAt={project?.openedAt ?? null}
        phase={stack.phase}
        label={stack.label}
        inventory={stack.inventory}
        loading={statusQuery.isFetching && !stack.inventory}
      />

      <IngressLinksCard
        ingress={ingress}
        loading={statusQuery.isLoading && !ingress}
      />

      {progress.state.active ? (
        <ApplyProgress
          visible
          ordered={progress.ordered}
          percent={progress.percent}
          mode={progress.state.mode}
          error={progress.state.error}
        />
      ) : null}

      {progress.state.finished && !progress.state.active ? (
        <div className="apply-banner" role="status">
          <span>
            {progress.state.error
              ? t("apply.finished.fail", {
                  mode: progress.state.mode ?? t("apply.modeDefault"),
                })
              : t("apply.finished.ok", {
                  mode: progress.state.mode ?? t("apply.modeDefault"),
                })}
          </span>
          <div className="apply-banner-actions">
            <Button
              tone="secondary"
              className="debug-btn"
              onClick={() => setLogOpen((open) => !open)}
            >
              {logOpen ? t("apply.logOff") : t("apply.logOn")}
            </Button>
            <Button
              tone="secondary"
              className="debug-btn"
              onClick={() => {
                progress.reset();
                setLogOpen(false);
              }}
            >
              {t("apply.close")}
            </Button>
          </div>
        </div>
      ) : null}

      {progress.state.active || logOpen ? (
        <ConsoleLog
          visible
          lines={progress.state.lines}
        />
      ) : null}

      {!progress.state.active ? (
        <ResultPanel
          result={result}
          busyLabel={busyLabel}
          stackPath={path}
          onJournalRecovered={() => {
            void queryClient.invalidateQueries({ queryKey: queryKeys.diff(path) });
            void queryClient.invalidateQueries({
              queryKey: queryKeys.status(path),
            });
            setResult({ kind: "idle", raw: "" });
            setLogOpen(false);
          }}
        />
      ) : null}
        </>
      )}

      <ConfirmDialog
        open={confirm != null}
        title={confirmCopy().title}
        message={confirmCopy().message}
        confirmLabel={confirmCopy().confirmLabel}
        tone={confirmCopy().tone}
        busy={confirm === "remove" ? removeMutation.isPending : mutate.isPending}
        onCancel={() => {
          if (!busy) setConfirm(null);
        }}
        onConfirm={runConfirmed}
      />

      <VmnetOrphanDialog
        open={orphan != null}
        orphanedCidr={orphan?.cidr ?? ""}
        suggestedCidr={suggestedCidr}
        busy={orphanBusy || mutate.isPending}
        error={orphanError}
        onCancel={() => {
          if (!orphanBusy && !mutate.isPending) {
            setOrphan(null);
            setOrphanError(null);
          }
        }}
        onChoose={(choice) => void handleOrphanChoice(choice)}
      />
    </section>
  );
}
