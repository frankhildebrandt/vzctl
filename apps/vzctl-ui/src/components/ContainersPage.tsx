import { Link, useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { getT, useT } from "@/lib/i18n";
import {
  dockerKeys,
  isContainerRunning,
  listContainers,
  projectFromVmId,
  restartContainer,
  runContainer,
  shortContainerId,
  startContainer,
  stopContainer,
  type DockerContainer,
} from "@/lib/docker";
import { getProject } from "@/lib/projects";
import { encodeVmIdParam, inspectVm, isRunning, vmKeys } from "@/lib/vms";
import { basename } from "@/lib/vzctl";

type PendingConfirm =
  | { kind: "stop"; container: DockerContainer }
  | { kind: "restart"; container: DockerContainer }
  | null;

export function ContainersPage({
  vmId,
  stackPath,
}: {
  vmId: string;
  stackPath?: string;
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const project = projectFromVmId(vmId);
  const [showRun, setShowRun] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingConfirm>(null);

  const [image, setImage] = useState("");
  const [name, setName] = useState("");
  const [ports, setPorts] = useState("");
  const [envText, setEnvText] = useState("");
  const [cmd, setCmd] = useState("");

  const stackName = useMemo(() => {
    if (!stackPath) return null;
    return getProject(stackPath)?.name ?? basename(stackPath);
  }, [stackPath]);

  const vmQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
    refetchInterval: 5000,
  });

  const containersQuery = useQuery({
    queryKey: dockerKeys.containers(project ?? ""),
    queryFn: () => listContainers(project!),
    enabled: project != null,
    refetchInterval: 5000,
  });

  const running = isRunning(vmQuery.data?.vm.state);
  const containers = containersQuery.data ?? [];

  async function refresh() {
    if (project) {
      await queryClient.invalidateQueries({
        queryKey: dockerKeys.containers(project),
      });
    }
  }

  const lifecycle = useMutation({
    mutationFn: async ({
      action,
      id,
    }: {
      action: "start" | "stop" | "restart";
      id: string;
    }) => {
      if (!project) throw new Error(getT()("containers.dockerProjectOnly"));
      if (action === "start") return startContainer(project, id);
      if (action === "stop") return stopContainer(project, id);
      return restartContainer(project, id);
    },
    onMutate: ({ action, id }) => {
      setBusyId(`${action}:${id}`);
      setError(null);
      setMessage(null);
    },
    onSuccess: (_data, { action, id }) => {
      setMessage(`${action} ${shortContainerId(id)}`);
    },
    onError: (err) => setError(String(err)),
    onSettled: async () => {
      setBusyId(null);
      setPending(null);
      await refresh();
    },
  });

  const runMutation = useMutation({
    mutationFn: async () => {
      if (!project) throw new Error(getT()("containers.dockerProjectOnly"));
      if (!image.trim()) throw new Error(getT()("containers.imageRequired"));
      const env = envText
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean);
      const portList = ports
        .split(",")
        .map((p) => p.trim())
        .filter(Boolean);
      const cmdParts = cmd
        .trim()
        .split(/\s+/)
        .filter(Boolean);
      return runContainer({
        project,
        image: image.trim(),
        name: name.trim() || undefined,
        env,
        ports: portList,
        cmd: cmdParts,
      });
    },
    onMutate: () => {
      setBusyId("run");
      setError(null);
      setMessage(null);
    },
    onSuccess: async (id) => {
      setMessage(t("containers.started", { id: shortContainerId(id) }));
      setShowRun(false);
      setImage("");
      setName("");
      setPorts("");
      setEnvText("");
      setCmd("");
      await refresh();
      await navigate({
        to: "/vms/$vmId/containers/$containerId",
        params: {
          vmId: encodeVmIdParam(vmId),
          containerId: encodeURIComponent(id),
        },
        search: stackPath ? { stackPath } : {},
      });
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyId(null),
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
        { label: t("nav.containers") },
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
        { label: t("nav.containers") },
      ];

  if (project == null) {
    return (
      <section>
        <Breadcrumbs items={crumbs} />
        <h2 className="section-title">{t("containers.title")}</h2>
        <div className="card error-card">{t("containers.noProject")}</div>
      </section>
    );
  }

  return (
    <section>
      <div className="row" style={{ justifyContent: "space-between", gap: "1rem" }}>
        <div>
          <Breadcrumbs items={crumbs} />
          <h2 className="section-title">{t("containers.title")}</h2>
          <p className="muted">
            {vmId}
            {running ? null : t("containers.vmNotRunning")}
          </p>
        </div>
        <div className="toolbar">
          <button
            type="button"
            disabled={!running || busyId != null}
            onClick={() => setShowRun((v) => !v)}
          >
            {t("containers.run")}
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

      {showRun ? (
        <div className="card">
          <h3>{t("containers.runTitle")}</h3>
          <form
            className="form-grid"
            onSubmit={(event) => {
              event.preventDefault();
              runMutation.mutate();
            }}
          >
            <label>
              {t("containers.image")}
              <input
                value={image}
                onChange={(e) => setImage(e.target.value)}
                placeholder="nginx:alpine"
                required
                autoFocus
              />
            </label>
            <label>
              {t("containers.name")}
              <input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("containers.namePlaceholder")}
              />
            </label>
            <label>
              {t("containers.ports")}
              <input
                value={ports}
                onChange={(e) => setPorts(e.target.value)}
                placeholder={t("containers.portsPlaceholder")}
              />
            </label>
            <label className="form-span-2">
              {t("containers.env")}
              <textarea
                value={envText}
                onChange={(e) => setEnvText(e.target.value)}
                rows={3}
                placeholder={"FOO=bar\nBAZ=qux"}
              />
            </label>
            <label className="form-span-2">
              {t("containers.command")}
              <input
                value={cmd}
                onChange={(e) => setCmd(e.target.value)}
                placeholder={t("containers.namePlaceholder")}
              />
            </label>
            <div className="row" style={{ gap: "0.5rem" }}>
              <button type="submit" disabled={busyId != null}>
                {busyId === "run" ? t("containers.startBusy") : t("containers.start")}
              </button>
              <button
                type="button"
                className="secondary"
                disabled={busyId != null}
                onClick={() => setShowRun(false)}
              >
                {t("common.cancel")}
              </button>
            </div>
          </form>
        </div>
      ) : null}

      <div className="card">
        {containersQuery.isLoading ? (
          <p className="muted">{t("common.loading")}</p>
        ) : containersQuery.isError ? (
          <p className="muted">{String(containersQuery.error)}</p>
        ) : containers.length === 0 ? (
          <p className="muted">{t("containers.empty")}</p>
        ) : (
          <table className="vm-table">
            <thead>
              <tr>
                <th>{t("containers.col.id")}</th>
                <th>{t("containers.col.name")}</th>
                <th>{t("containers.col.image")}</th>
                <th>{t("containers.col.status")}</th>
                <th>{t("containers.col.ip")}</th>
                <th>{t("containers.col.ports")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {containers.map((c) => {
                const up = isContainerRunning(c.state) || isContainerRunning(c.status);
                return (
                  <tr key={c.id}>
                    <td className="mono">
                      <Link
                        to="/vms/$vmId/containers/$containerId"
                        params={{
                          vmId: encodeVmIdParam(vmId),
                          containerId: encodeURIComponent(c.id),
                        }}
                        search={stackPath ? { stackPath } : {}}
                        className="crumb-link"
                      >
                        {shortContainerId(c.id)}
                      </Link>
                    </td>
                    <td>{c.names || t("common.emDash")}</td>
                    <td className="mono">{c.image || t("common.emDash")}</td>
                    <td>
                      <span
                        className={`vm-state ${up ? "state-running" : "state-stopped"}`}
                      >
                        {c.status || c.state || t("common.emDash")}
                      </span>
                    </td>
                    <td className="mono">{c.ip || t("common.emDash")}</td>
                    <td className="mono">{c.ports || t("common.emDash")}</td>
                    <td>
                      <div className="row" style={{ gap: "0.35rem", justifyContent: "flex-end" }}>
                        {up ? (
                          <>
                            <button
                              type="button"
                              className="secondary"
                              disabled={busyId != null || !running}
                              onClick={() => setPending({ kind: "stop", container: c })}
                            >
                              {busyId === `stop:${c.id}` ? t("common.ellipsis") : t("containers.stop")}
                            </button>
                            <button
                              type="button"
                              className="secondary"
                              disabled={busyId != null || !running}
                              onClick={() => setPending({ kind: "restart", container: c })}
                            >
                              {busyId === `restart:${c.id}` ? t("common.ellipsis") : t("containers.restart")}
                            </button>
                          </>
                        ) : (
                          <button
                            type="button"
                            className="secondary"
                            disabled={busyId != null || !running}
                            onClick={() =>
                              lifecycle.mutate({ action: "start", id: c.id })
                            }
                          >
                            {busyId === `start:${c.id}` ? t("common.ellipsis") : t("containers.start")}
                          </button>
                        )}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      <ConfirmDialog
        open={pending != null}
        title={
          pending?.kind === "stop"
            ? t("containers.stopTitle")
            : t("containers.restartTitle")
        }
        message={
          pending
            ? t("containers.confirmNamed", {
                action:
                  pending.kind === "stop"
                    ? t("containers.stopConfirm")
                    : t("containers.restartConfirm"),
                name:
                  pending.container.names ||
                  shortContainerId(pending.container.id),
              })
            : ""
        }
        confirmLabel={pending?.kind === "stop" ? t("containers.stopConfirm") : t("containers.restartConfirm")}
        tone="danger"
        busy={busyId != null}
        onCancel={() => {
          if (busyId == null) setPending(null);
        }}
        onConfirm={() => {
          if (!pending) return;
          lifecycle.mutate({
            action: pending.kind,
            id: pending.container.id,
          });
        }}
      />
    </section>
  );
}
