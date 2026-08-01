import { Link, useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ConfirmDialog } from "@/components/ConfirmDialog";
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
      if (!project) throw new Error("Docker nur mit Projekt");
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
      if (!project) throw new Error("Docker nur mit Projekt");
      if (!image.trim()) throw new Error("Image ist erforderlich");
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
      setMessage(`gestartet ${shortContainerId(id)}`);
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
          label: "Stacks",
          node: (
            <Link to="/projects" className="crumb-link">
              Stacks
            </Link>
          ),
        },
        {
          label: stackName ?? "Stack",
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
        { label: "Containers" },
      ]
    : [
        {
          label: "VMs",
          node: (
            <Link to="/vms" className="crumb-link">
              VMs
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
        { label: "Containers" },
      ];

  if (project == null) {
    return (
      <section>
        <Breadcrumbs items={crumbs} />
        <h2 className="section-title">Containers</h2>
        <div className="card error-card">
          Docker-Context braucht eine Projekt-VM (`project/vm`).
        </div>
      </section>
    );
  }

  return (
    <section>
      <div className="row" style={{ justifyContent: "space-between", gap: "1rem" }}>
        <div>
          <Breadcrumbs items={crumbs} />
          <h2 className="section-title">Containers</h2>
          <p className="muted">
            {vmId}
            {running ? null : " · VM nicht running"}
          </p>
        </div>
        <div className="toolbar">
          <button
            type="button"
            disabled={!running || busyId != null}
            onClick={() => setShowRun((v) => !v)}
          >
            Run
          </button>
        </div>
      </div>

      {message ? <p className="ok-banner">{message}</p> : null}
      {error ? (
        <div className="card error-card">
          <h3>Fehler</h3>
          <p>{error}</p>
        </div>
      ) : null}

      {showRun ? (
        <div className="card">
          <h3>Container starten</h3>
          <form
            className="form-grid"
            onSubmit={(event) => {
              event.preventDefault();
              runMutation.mutate();
            }}
          >
            <label>
              Image
              <input
                value={image}
                onChange={(e) => setImage(e.target.value)}
                placeholder="nginx:alpine"
                required
                autoFocus
              />
            </label>
            <label>
              Name
              <input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="optional"
              />
            </label>
            <label>
              Ports
              <input
                value={ports}
                onChange={(e) => setPorts(e.target.value)}
                placeholder="8080:80, 8443:443"
              />
            </label>
            <label className="form-span-2">
              Env (eine Zeile pro KEY=VAL)
              <textarea
                value={envText}
                onChange={(e) => setEnvText(e.target.value)}
                rows={3}
                placeholder={"FOO=bar\nBAZ=qux"}
              />
            </label>
            <label className="form-span-2">
              Command
              <input
                value={cmd}
                onChange={(e) => setCmd(e.target.value)}
                placeholder="optional"
              />
            </label>
            <div className="row" style={{ gap: "0.5rem" }}>
              <button type="submit" disabled={busyId != null}>
                {busyId === "run" ? "Start…" : "Start"}
              </button>
              <button
                type="button"
                className="secondary"
                disabled={busyId != null}
                onClick={() => setShowRun(false)}
              >
                Abbrechen
              </button>
            </div>
          </form>
        </div>
      ) : null}

      <div className="card">
        {containersQuery.isLoading ? (
          <p className="muted">Laden…</p>
        ) : containersQuery.isError ? (
          <p className="muted">{String(containersQuery.error)}</p>
        ) : containers.length === 0 ? (
          <p className="muted">Keine Container.</p>
        ) : (
          <table className="vm-table">
            <thead>
              <tr>
                <th>ID</th>
                <th>Name</th>
                <th>Image</th>
                <th>Status</th>
                <th>IP</th>
                <th>Ports</th>
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
                    <td>{c.names || "—"}</td>
                    <td className="mono">{c.image || "—"}</td>
                    <td>
                      <span
                        className={`vm-state ${up ? "state-running" : "state-stopped"}`}
                      >
                        {c.status || c.state || "—"}
                      </span>
                    </td>
                    <td className="mono">{c.ip || "—"}</td>
                    <td className="mono">{c.ports || "—"}</td>
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
                              {busyId === `stop:${c.id}` ? "…" : "Stop"}
                            </button>
                            <button
                              type="button"
                              className="secondary"
                              disabled={busyId != null || !running}
                              onClick={() => setPending({ kind: "restart", container: c })}
                            >
                              {busyId === `restart:${c.id}` ? "…" : "Restart"}
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
                            {busyId === `start:${c.id}` ? "…" : "Start"}
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
            ? "Container stoppen?"
            : "Container neu starten?"
        }
        message={
          pending
            ? `${pending.kind === "stop" ? "Stop" : "Restart"} ${pending.container.names || shortContainerId(pending.container.id)}`
            : ""
        }
        confirmLabel={pending?.kind === "stop" ? "Stop" : "Restart"}
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
