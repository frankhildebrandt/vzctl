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
import { ApplyProgress, ConsoleLog, useApplyProgress } from "@/components/ApplyProgress";
import { Breadcrumbs } from "@/components/Breadcrumbs";
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
import { ResultPanel, type ResultModel } from "@/components/ResultPanel";
import { IngressLinksCard } from "@/components/IngressLinks";
import { SettingsPage } from "@/components/SettingsPage";
import { ProjectOidcUplinkSection } from "@/components/ProjectOidcUplinkSection";
import { StackStatusCard } from "@/components/StackStatus";
import { DashboardPage, PlaceholderPage } from "@/components/pages";
import { ContainerDetailPage } from "@/components/ContainerDetailPage";
import { ContainersPage } from "@/components/ContainersPage";
import { VmDetailPage } from "@/components/VmDetailPage";
import { VmListPage } from "@/components/VmListPage";
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
import { useEditorStore } from "@/store/editorStore";
import { DEMO_PROJECT_PATH, enableDemoMode } from "@/lib/demo";

export const rootRoute = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  return (
    <AppShell>
      <Outlet />
    </AppShell>
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

export const vmDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/vms/$vmId",
  validateSearch: (search: Record<string, unknown>): { stackPath?: string } => ({
    stackPath:
      typeof search.stackPath === "string" && search.stackPath.length > 0
        ? search.stackPath
        : undefined,
  }),
  component: function VmDetailRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    const { stackPath } = vmDetailRoute.useSearch();
    return (
      <VmDetailPage
        vmId={decodeVmIdParam(rawVmId)}
        stackPath={stackPath}
      />
    );
  },
});

function vmStackSearch(search: Record<string, unknown>): { stackPath?: string } {
  return {
    stackPath:
      typeof search.stackPath === "string" && search.stackPath.length > 0
        ? search.stackPath
        : undefined,
  };
}

export const vmContainersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/vms/$vmId/containers",
  validateSearch: vmStackSearch,
  component: function VmContainersRoute() {
    const { vmId: rawVmId } = vmContainersRoute.useParams();
    const { stackPath } = vmContainersRoute.useSearch();
    return (
      <ContainersPage
        vmId={decodeVmIdParam(rawVmId)}
        stackPath={stackPath}
      />
    );
  },
});

export const vmContainerDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/vms/$vmId/containers/$containerId",
  validateSearch: vmStackSearch,
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
  component: () => (
    <PlaceholderPage
      title="Networks"
      hint="Hier erscheint die Netzwerk-Übersicht. Noch nicht angebunden."
    />
  ),
});

export const doctorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/doctor",
  component: DoctorPage,
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
  component: () => (
    <PlaceholderPage
      title="Images"
      hint="Hier erscheint der Image-Cache (vzctl image …). Noch nicht angebunden."
    />
  ),
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
      if (!name) throw new Error("Name angeben");
      const { pickDirectory } = await import("@/lib/dialogs");
      const parent = await pickDirectory("Zielordner für neues Projekt");
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
      <div className="row" style={{ justifyContent: "space-between" }}>
        <div>
          <h2 className="section-title">Stacks</h2>
          <p className="muted">
            Environments mit <code>hypernetwork.config.yaml</code> — Topologie
            bearbeiten und Apply.
          </p>
        </div>
        <div className="row" style={{ gap: "0.5rem" }}>
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => {
              setError(null);
              addMutation.mutate();
            }}
          >
            {addMutation.isPending ? "Öffnen…" : "Öffnen…"}
          </button>
        </div>
      </div>

      <div className="card create-project-card">
        <h3>Neues Projekt</h3>
        <p className="muted">
          Legt Ordner + Minimal-<code>hypernetwork.config.yaml</code> an und
          öffnet den Topologie-Editor.
        </p>
        <div className="row" style={{ gap: "0.5rem", flexWrap: "wrap" }}>
          <label className="topology-field" style={{ flex: "1 1 12rem" }}>
            <span className="sr-only">Projektname</span>
            <input
              value={createName}
              onChange={(e) => setCreateName(e.target.value)}
              placeholder="Projektname (z. B. edge-lab)"
              aria-label="Projektname"
              disabled={busy}
            />
          </label>
          <button
            type="button"
            disabled={busy || !createName.trim()}
            onClick={() => {
              setError(null);
              createMutation.mutate();
            }}
          >
            {createMutation.isPending ? "Anlegen…" : "Anlegen…"}
          </button>
        </div>
      </div>

      {error ? (
        <div className="card error-card">
          <h3>Fehler</h3>
          <p>{error}</p>
        </div>
      ) : null}

      {projects.length === 0 ? (
        <div className="card">
          <h2>Noch keine Stacks</h2>
          <p className="muted">
            Öffne ein Verzeichnis mit <code>hypernetwork.config.yaml</code>.
            Die Auswahl bleibt lokal gespeichert.
          </p>
        </div>
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
                <span className="muted project-meta">
                  Zuletzt: {formatOpenedAt(project.openedAt)}
                </span>
              </Link>
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() =>
                  setPendingRemove({ path: project.path, name: project.name })
                }
              >
                Entfernen
              </button>
            </li>
          ))}
        </ul>
      )}

      <ConfirmDialog
        open={pendingRemove != null}
        title="Aus Liste entfernen"
        message={
          pendingRemove
            ? `„${pendingRemove.name}“ aus der Liste entfernen? Die Dateien und der Stack bleiben erhalten.`
            : ""
        }
        confirmLabel="Entfernen"
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
    setBusyLabel("Status laden…");
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
      }),
    [
      inventory,
      progress.state.active,
      progress.state.mode,
      progress.state.finished,
      progress.state.error,
    ],
  );

  const diffQuery = useQuery({
    queryKey: queryKeys.diff(path),
    queryFn: () => runVzctl(path, "diff"),
    enabled: false,
  });

  useEffect(() => {
    if (statusQuery.isLoading) {
      setBusyLabel("Status laden…");
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
      setBusyLabel(purge ? "stack entfernen…" : `${command}…`);
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
    setBusyLabel("Status laden…");
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
    setBusyLabel("Diff laden…");
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
          title: "Stack up",
          message:
            "Stack up ausführen? Breaking Changes (z. B. Recreate) werden mit --force übernommen.",
          confirmLabel: "Up ausführen",
          tone: "default",
        };
      case "apply":
        return {
          title: "Stack apply",
          message:
            "Stack apply ausführen? Breaking Changes (z. B. VM-Recreate) werden mit --force übernommen.",
          confirmLabel: "Apply ausführen",
          tone: "default",
        };
      case "down":
        return {
          title: "Stack down",
          message: "Stack down — laufende VMs stoppen? Ressourcen bleiben erhalten.",
          confirmLabel: "Down ausführen",
          tone: "danger",
        };
      case "purge":
        return {
          title: "Stack entfernen",
          message:
            "Stack stoppen und löschen (VMs, Netze, Ports, Ingress, OIDC, DNS-Einträge)? Das Projektverzeichnis und die Config bleiben erhalten.",
          confirmLabel: "Stack entfernen",
          tone: "danger",
        };
      case "remove":
      default:
        return {
          title: "Projekt entfernen",
          message: `„${title}“ aus der Liste entfernen? Die Dateien auf der Festplatte bleiben erhalten.`,
          confirmLabel: "Entfernen",
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
        raw: `CIDR ${orphan.cidr} → ${result.newCidr} (${result.networkNames.join(", ")}); starte up erneut…`,
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
      <header className="detail-header">
        <div className="detail-heading">
          <Breadcrumbs
            items={[
              {
                label: "Stacks",
                node: (
                  <Link to="/projects" className="crumb-link">
                    Stacks
                  </Link>
                ),
              },
              { label: title },
            ]}
          />
          {tab === "topology" ? (
            <>
              <h2 className="detail-title">{title}</h2>
              <p className="path detail-path">{path}</p>
              {project ? (
                <p className="muted detail-meta">
                  Zuletzt geöffnet: {formatOpenedAt(project.openedAt)}
                </p>
              ) : null}
            </>
          ) : tab === "config" ? (
            <>
              <h2 className="detail-title">{title}</h2>
              <p className="path detail-path">{path}</p>
              <p className="muted detail-meta">Stack-Konfiguration</p>
            </>
          ) : null}
        </div>

        <div className="detail-actions">
          {tab === "ops" ? (
            <div
              className="detail-toolbar"
              role="toolbar"
              aria-label="Projektaktionen"
            >
              <IconButton
                label="Diff"
                showLabel
                disabled={busy}
                tone="quiet"
                onClick={() => void runDiff()}
              >
                <IconDiff />
              </IconButton>
              <IconButton
                label="Up"
                showLabel
                disabled={busy}
                tone="quiet"
                onClick={() => setConfirm("up")}
              >
                <IconPlay />
              </IconButton>
              <IconButton
                label="Apply"
                showLabel
                disabled={busy}
                tone="primary"
                onClick={() => setConfirm("apply")}
              >
                <IconApply />
              </IconButton>
              <IconButton
                label="Down"
                showLabel
                disabled={busy}
                tone="danger"
                onClick={() => setConfirm("down")}
              >
                <IconStop />
              </IconButton>
              <IconButton
                label="Stack entfernen"
                showLabel
                disabled={busy}
                tone="danger"
                onClick={() => setConfirm("purge")}
              >
                <IconPurge />
              </IconButton>
              <IconButton
                label="DNS / OIDC / CA Status"
                disabled={busy}
                tone="quiet"
                onClick={() => void runStatus()}
              >
                <IconStatus />
              </IconButton>
              <span className="toolbar-sep" aria-hidden />
              <IconButton
                label="Aus Liste entfernen"
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
              aria-label="Stack Config"
            >
              <IconButton
                label="Aus Liste entfernen"
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
              aria-label="Topologie"
            >
              <div
                className="detail-toolbar-slot"
                ref={setTopologyToolbarHost}
              />
              <span className="toolbar-sep" aria-hidden />
              <IconButton
                label="Aus Liste entfernen"
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
          <div className="card error-card">
            <h3>Topologie konnte nicht geladen werden</h3>
            <p>{topologyError}</p>
          </div>
        ) : (
          <TopologyEditor
            projectPath={path}
            toolbarHost={topologyToolbarHost}
          />
        )
      ) : tab === "config" ? (
        <section>
          <h2 className="section-title">Stack Config</h2>
          <p className="muted">
            Stack-spezifische Einstellungen in{" "}
            <code>hypernetwork.config.yaml</code>. Host-Defaults unter{" "}
            <Link to="/settings">Settings</Link>.
          </p>
          <ProjectOidcUplinkSection projectPath={path} />
        </section>
      ) : (
        <>
      <StackStatusCard
        title={title}
        path={path}
        openedAt={
          project ? formatOpenedAt(project.openedAt) : null
        }
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
              ? `${progress.state.mode ?? "apply"} fehlgeschlagen`
              : `${progress.state.mode ?? "apply"} fertig`}
          </span>
          <div className="apply-banner-actions">
            <button
              type="button"
              className="debug-btn"
              onClick={() => setLogOpen((open) => !open)}
            >
              {logOpen ? "Log aus" : "Log"}
            </button>
            <button
              type="button"
              className="debug-btn"
              onClick={() => {
                progress.reset();
                setLogOpen(false);
              }}
            >
              Schließen
            </button>
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
        <ResultPanel result={result} busyLabel={busyLabel} />
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
