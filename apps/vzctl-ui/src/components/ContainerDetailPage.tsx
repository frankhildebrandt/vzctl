import { Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Terminal } from "@/components/Terminal";
import { getT, useT } from "@/lib/i18n";
import {
  dockerKeys,
  inspectContainer,
  isContainerRunning,
  listContainers,
  projectFromVmId,
  restartContainer,
  shortContainerId,
  startContainer,
  stopContainer,
} from "@/lib/docker";
import { getProject } from "@/lib/projects";
import { encodeVmIdParam, inspectVm, isRunning, vmKeys } from "@/lib/vms";
import { basename } from "@/lib/vzctl";

type PendingConfirm = "stop" | "restart" | null;

export function ContainerDetailPage({
  vmId,
  containerId,
  stackPath,
}: {
  vmId: string;
  containerId: string;
  stackPath?: string;
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const project = projectFromVmId(vmId);
  const [showShell, setShowShell] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingConfirm>(null);

  const stackName = useMemo(() => {
    if (!stackPath) return null;
    return getProject(stackPath)?.name ?? basename(stackPath);
  }, [stackPath]);

  const vmQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
    refetchInterval: 5000,
  });

  const listQuery = useQuery({
    queryKey: dockerKeys.containers(project ?? ""),
    queryFn: () => listContainers(project!),
    enabled: project != null,
    refetchInterval: 5000,
  });

  const inspectQuery = useQuery({
    queryKey: dockerKeys.inspect(project ?? "", containerId),
    queryFn: () => inspectContainer(project!, containerId),
    enabled: project != null,
  });

  const running = isRunning(vmQuery.data?.vm.state);
  const listed = listQuery.data?.find(
    (c) => c.id === containerId || c.id.startsWith(containerId) || containerId.startsWith(c.id),
  );
  const state =
    listed?.state ||
    listed?.status ||
    (typeof inspectQuery.data?.State === "object" &&
    inspectQuery.data.State &&
    typeof (inspectQuery.data.State as Record<string, unknown>).Status === "string"
      ? String((inspectQuery.data.State as Record<string, unknown>).Status)
      : "");
  const up = isContainerRunning(state);
  const displayName =
    listed?.names ||
    (typeof inspectQuery.data?.Name === "string"
      ? inspectQuery.data.Name.replace(/^\//, "")
      : shortContainerId(containerId));

  async function refresh() {
    if (!project) return;
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: dockerKeys.containers(project) }),
      queryClient.invalidateQueries({
        queryKey: dockerKeys.inspect(project, containerId),
      }),
    ]);
  }

  const lifecycle = useMutation({
    mutationFn: async (action: "start" | "stop" | "restart") => {
      if (!project) throw new Error(getT()("containers.dockerProjectOnly"));
      if (action === "start") return startContainer(project, containerId);
      if (action === "stop") return stopContainer(project, containerId);
      return restartContainer(project, containerId);
    },
    onMutate: (action) => {
      setBusy(action);
      setError(null);
      setMessage(null);
    },
    onSuccess: (_data, action) => setMessage(`${action} ${shortContainerId(containerId)}`),
    onError: (err) => setError(String(err)),
    onSettled: async () => {
      setBusy(null);
      setPending(null);
      await refresh();
    },
  });

  const crumbs = stackPath
    ? [
        {
          label: t("crumb.stacks"),
          node: (
            <Link to="/projects" className="crumb-link">
              {t("crumb.stacks")}
            </Link>
          ),
        },
        {
          label: stackName ?? t("nav.stackFallback"),
          node: (
            <Link
              to="/env"
              search={{ path: stackPath, tab: "ops" as const }}
              className="crumb-link"
            >
              {stackName}
            </Link>
          ),
        },
        {
          label: vmId,
          node: (
            <Link
              to="/vms/$vmId"
              params={{ vmId: encodeVmIdParam(vmId) }}
              search={stackPath ? { stackPath } : {}}
              className="crumb-link"
            >
              {vmId}
            </Link>
          ),
        },
        {
          label: t("nav.containers"),
          node: (
            <Link
              to="/vms/$vmId/containers"
              params={{ vmId: encodeVmIdParam(vmId) }}
              search={stackPath ? { stackPath } : {}}
              className="crumb-link"
            >
              {t("nav.containers")}
            </Link>
          ),
        },
        { label: shortContainerId(containerId) },
      ]
    : [
        {
          label: t("crumb.vms"),
          node: (
            <Link to="/vms" className="crumb-link">
              {t("crumb.vms")}
            </Link>
          ),
        },
        {
          label: vmId,
          node: (
            <Link
              to="/vms/$vmId"
              params={{ vmId: encodeVmIdParam(vmId) }}
              className="crumb-link"
            >
              {vmId}
            </Link>
          ),
        },
        {
          label: t("nav.containers"),
          node: (
            <Link
              to="/vms/$vmId/containers"
              params={{ vmId: encodeVmIdParam(vmId) }}
              className="crumb-link"
            >
              {t("nav.containers")}
            </Link>
          ),
        },
        { label: shortContainerId(containerId) },
      ];

  if (project == null) {
    return (
      <section>
        <Breadcrumbs items={crumbs} />
        <h2 className="section-title">{displayName}</h2>
        <div className="card error-card">{t("containers.noProject")}</div>
      </section>
    );
  }

  return (
    <section>
      <div className="row" style={{ justifyContent: "space-between", gap: "1rem" }}>
        <div>
          <Breadcrumbs items={crumbs} />
          <h2 className="section-title">{displayName}</h2>
          <p className="muted">
            <span className={`vm-state ${up ? "state-running" : "state-stopped"}`}>
              {listed?.status || state || t("common.emDash")}
            </span>
            {" · "}
            <span className="mono">{shortContainerId(containerId)}</span>
          </p>
        </div>
        <div className="toolbar">
          {up ? (
            <>
              <button
                type="button"
                className="secondary"
                disabled={busy != null || !running}
                onClick={() => setPending("stop")}
              >
                {busy === "stop" ? `${t("containers.stop")}…` : t("containers.stop")}
              </button>
              <button
                type="button"
                className="secondary"
                disabled={busy != null || !running}
                onClick={() => setPending("restart")}
              >
                {busy === "restart" ? `${t("containers.restart")}…` : t("containers.restart")}
              </button>
            </>
          ) : (
            <button
              type="button"
              disabled={busy != null || !running}
              onClick={() => lifecycle.mutate("start")}
            >
              {busy === "start" ? t("containers.startBusy") : t("containers.start")}
            </button>
          )}
          <button
            type="button"
            className="secondary"
            disabled={!running || !up || busy != null}
            onClick={() => setShowShell((v) => !v)}
          >
            {t("vmDetail.shell")}
          </button>
        </div>
      </div>

      {message ? <p className="ok-banner">{message}</p> : null}
      {error ? (
        <div className="card error-card">
          <h3>{t("common.error")}</h3>
          <p>{error}</p>
        </div>
      ) : null}

      {showShell ? (
        <div className="card terminal-card">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <h3>{t("containerDetail.shellTitle")}</h3>
            <button
              type="button"
              className="secondary"
              onClick={() => setShowShell(false)}
            >
              {t("common.close")}
            </button>
          </div>
          <Terminal
            mode="exec"
            vmId={vmId}
            cmd={["docker", "exec", "-it", containerId, "/bin/sh"]}
          />
        </div>
      ) : null}

      <div className="dash-grid">
        <div className="card">
          <h3>{t("containerDetail.overview")}</h3>
          <dl className="kv">
            <dt>{t("containerDetail.id")}</dt>
            <dd className="mono">{containerId}</dd>
            <dt>{t("containerDetail.name")}</dt>
            <dd>{displayName}</dd>
            <dt>{t("containerDetail.image")}</dt>
            <dd className="mono">
              {listed?.image ||
                (typeof inspectQuery.data?.Config === "object" &&
                inspectQuery.data.Config &&
                typeof (inspectQuery.data.Config as Record<string, unknown>).Image ===
                  "string"
                  ? String((inspectQuery.data.Config as Record<string, unknown>).Image)
                  : t("common.emDash"))}
            </dd>
            <dt>{t("containerDetail.ports")}</dt>
            <dd className="mono">{listed?.ports || t("common.emDash")}</dd>
          </dl>
        </div>

        <div className="card">
          <h3>{t("containerDetail.inspect")}</h3>
          {inspectQuery.isLoading ? (
            <p className="muted">{t("common.loading")}</p>
          ) : inspectQuery.isError ? (
            <p className="muted">{String(inspectQuery.error)}</p>
          ) : (
            <pre className="mono inspect-json">
              {JSON.stringify(inspectQuery.data ?? {}, null, 2)}
            </pre>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={pending != null}
        title={pending === "stop" ? t("containerDetail.stopTitle") : t("containerDetail.restartTitle")}
        message={`${pending === "stop" ? t("containers.stopConfirm") : t("containers.restartConfirm")} ${displayName}`}
        confirmLabel={pending === "stop" ? t("containers.stopConfirm") : t("containers.restartConfirm")}
        tone="danger"
        busy={busy != null}
        onCancel={() => {
          if (busy == null) setPending(null);
        }}
        onConfirm={() => {
          if (!pending) return;
          lifecycle.mutate(pending);
        }}
      />
    </section>
  );
}
