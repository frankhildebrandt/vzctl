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
import { ConfirmDialog } from "@/components/ConfirmDialog";
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
import { ResultPanel, type ResultModel } from "@/components/ResultPanel";
import { IngressLinksCard } from "@/components/IngressLinks";
import { StackCardsSection } from "@/components/StackCard";
import { StackStatusCard } from "@/components/StackStatus";
import { DashboardPage, PlaceholderPage } from "@/components/pages";
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
  component: function VmDetailRoute() {
    const { vmId: rawVmId } = vmDetailRoute.useParams();
    return <VmDetailPage vmId={decodeVmIdParam(rawVmId)} />;
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
        await navigate({ to: "/env", search: { path } });
      }
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
  const busy = addMutation.isPending || removeMutation.isPending;

  return (
    <section>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <div>
          <h2 className="section-title">Stacks</h2>
          <p className="muted">
            Declarative Environments — Start, Stop und Löschen pro Stack.
          </p>
        </div>
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setError(null);
            addMutation.mutate();
          }}
        >
          {addMutation.isPending ? "Öffnen…" : "Stack hinzufügen…"}
        </button>
      </div>

      {error ? (
        <div className="card error-card">
          <h3>Fehler</h3>
          <p>{error}</p>
        </div>
      ) : null}

      <StackCardsSection
        title=""
        projects={projects}
        emptyHint={
          <>
            Noch keine Stacks. Öffne ein Verzeichnis mit{" "}
            <code>hypernetwork.config.yaml</code>. Die Auswahl bleibt lokal
            gespeichert.
          </>
        }
        onForget={(path) => {
          const project = projects.find((item) => item.path === path);
          if (project) setPendingRemove({ path: project.path, name: project.name });
        }}
      />

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
  const { path } = envRoute.useSearch();
  const navigate = envRoute.useNavigate();
  const queryClient = useQueryClient();
  const [result, setResult] = useState<ResultModel>({ kind: "idle", raw: "" });
  const [busyLabel, setBusyLabel] = useState<string | null>(null);
  const progress = useApplyProgress(true);
  const [logOpen, setLogOpen] = useState(false);
  const [confirm, setConfirm] = useState<
    null | "up" | "apply" | "down" | "purge" | "remove"
  >(null);

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
    // progress.reset is stable enough for path switches; omit from deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, queryClient]);

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
    removeMutation.isPending;

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

  return (
    <section className="detail">
      <header className="detail-header">
        <div className="detail-heading">
          <Link to="/projects" className="crumb-link">
            ← Stacks
          </Link>
          <h2 className="detail-title">{title}</h2>
          <p className="path detail-path">{path}</p>
          {project ? (
            <p className="muted detail-meta">
              Zuletzt geöffnet: {formatOpenedAt(project.openedAt)}
            </p>
          ) : null}
        </div>

        <div className="detail-toolbar" role="toolbar" aria-label="Projektaktionen">
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
      </header>

      <StackStatusCard
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
          title={`${progress.state.mode ?? "apply"} — Log`}
          onDismiss={
            progress.state.active
              ? undefined
              : () => {
                  progress.reset();
                  setLogOpen(false);
                }
          }
        />
      ) : null}

      {!progress.state.active ? (
        <ResultPanel result={result} busyLabel={busyLabel} />
      ) : null}

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
    </section>
  );
}
