import { Link, useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Terminal } from "@/components/Terminal";
import { VmForm } from "@/components/VmForm";
import { VmMountForm } from "@/components/VmMountForm";
import { getProject } from "@/lib/projects";
import {
  createVm,
  deleteVm,
  encodeVmIdParam,
  inspectVm,
  isRunning,
  listMounts,
  modifyVm,
  startVm,
  stopVm,
  unmountVm,
  vmKeys,
  type CreateVmInput,
  type VmMount,
} from "@/lib/vms";
import { basename } from "@/lib/vzctl";

type Panel =
  | null
  | "modify"
  | "mount"
  | "replace"
  | "console"
  | "shell";

type PendingConfirm =
  | { kind: "delete" }
  | { kind: "unmount"; mount: VmMount }
  | { kind: "replace"; input: CreateVmInput }
  | null;

export function VmDetailPage({
  vmId,
  stackPath,
}: {
  vmId: string;
  stackPath?: string;
}) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [panel, setPanel] = useState<Panel>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [cpus, setCpus] = useState(2);
  const [memory, setMemory] = useState("1024");
  const [pending, setPending] = useState<PendingConfirm>(null);

  const stackName = useMemo(() => {
    if (!stackPath) return null;
    return getProject(stackPath)?.name ?? basename(stackPath);
  }, [stackPath]);

  const detailQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
    refetchInterval: 5000,
  });

  const mountsQuery = useQuery({
    queryKey: vmKeys.mounts(vmId),
    queryFn: () => listMounts(vmId),
  });

  useEffect(() => {
    const resources = detailQuery.data?.resources;
    if (resources) {
      setCpus(resources.cpus);
      setMemory(String(resources.memory_mib));
    }
  }, [detailQuery.data?.resources]);

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: vmKeys.detail(vmId) });
    await queryClient.invalidateQueries({ queryKey: vmKeys.mounts(vmId) });
    await queryClient.invalidateQueries({ queryKey: vmKeys.list() });
  }

  const startMutation = useMutation({
    mutationFn: () => startVm(vmId),
    onMutate: () => {
      setBusy("start");
      setError(null);
    },
    onSuccess: () => setMessage("VM gestartet"),
    onError: (err) => setError(String(err)),
    onSettled: async () => {
      setBusy(null);
      await refresh();
    },
  });

  const stopMutation = useMutation({
    mutationFn: () => stopVm(vmId),
    onMutate: () => {
      setBusy("stop");
      setError(null);
    },
    onSuccess: () => setMessage("VM gestoppt"),
    onError: (err) => setError(String(err)),
    onSettled: async () => {
      setBusy(null);
      await refresh();
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteVm(vmId, true),
    onMutate: () => {
      setBusy("delete");
      setError(null);
    },
    onSuccess: () => {
      setPending(null);
      if (stackPath) {
        void navigate({ to: "/env", search: { path: stackPath, tab: "ops" } });
      } else {
        void navigate({ to: "/vms" });
      }
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusy(null),
  });

  const inspect = detailQuery.data;
  const vm = inspect?.vm;
  const running = isRunning(vm?.state);
  const mounts = mountsQuery.data ?? [];

  async function submitModify(event: React.FormEvent) {
    event.preventDefault();
    setBusy("modify");
    setError(null);
    setMessage(null);
    try {
      const envelope = await modifyVm({
        id: vmId,
        cpus,
        memory: memory.trim(),
      });
      const restart = envelope.restart_required === true;
      setMessage(
        restart
          ? "Ressourcen gespeichert — Restart nötig"
          : "Ressourcen gespeichert (nächster Start)",
      );
      setPanel(null);
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function removeMount(mount: VmMount) {
    setBusy(`unmount:${mount.name}`);
    setError(null);
    try {
      await unmountVm(vmId, { tag: mount.name, target: mount.target });
      setPending(null);
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function replaceVm(input: CreateVmInput) {
    setBusy("replace");
    setError(null);
    try {
      if (running) {
        await stopVm(vmId);
      }
      await deleteVm(vmId, true);
      await createVm(input);
      setPending(null);
      setPanel(null);
      setMessage("VM ersetzt");
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  const crumbs =
    stackPath && stackName
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
            label: stackName,
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
          { label: vmId },
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
          { label: vmId },
        ];

  return (
    <section>
      <div className="row" style={{ justifyContent: "space-between", gap: "1rem" }}>
        <div>
          <Breadcrumbs items={crumbs} />
          <h2 className="section-title">{vmId}</h2>
          {vm ? (
            <p className="muted">
              <span className={`vm-state state-${vm.state}`}>{vm.state}</span>
              {vm.pid != null ? ` · pid ${vm.pid}` : null}
              {inspect?.resources
                ? ` · ${inspect.resources.cpus} CPU / ${inspect.resources.memory_mib} MiB`
                : null}
            </p>
          ) : null}
        </div>
        <div className="toolbar">
          {running ? (
            <button
              type="button"
              className="secondary"
              disabled={busy != null}
              onClick={() => stopMutation.mutate()}
            >
              {busy === "stop" ? "Stop…" : "Stop"}
            </button>
          ) : (
            <button
              type="button"
              disabled={busy != null}
              onClick={() => startMutation.mutate()}
            >
              {busy === "start" ? "Start…" : "Start"}
            </button>
          )}
          <button
            type="button"
            className="secondary"
            disabled={busy != null}
            onClick={() => setPanel(panel === "modify" ? null : "modify")}
          >
            Modify
          </button>
          <button
            type="button"
            className="secondary"
            disabled={busy != null}
            onClick={() => setPanel(panel === "mount" ? null : "mount")}
          >
            Mount
          </button>
          <button
            type="button"
            className="secondary"
            disabled={!running || busy != null}
            onClick={() => setPanel(panel === "console" ? null : "console")}
          >
            Attach
          </button>
          <button
            type="button"
            className="secondary"
            disabled={!running || busy != null}
            onClick={() => setPanel(panel === "shell" ? null : "shell")}
          >
            Shell
          </button>
          {vm?.roles?.includes("docker") ? (
            <button
              type="button"
              className="secondary"
              disabled={!running || busy != null}
              onClick={() =>
                void navigate({
                  to: "/vms/$vmId/containers",
                  params: { vmId: encodeVmIdParam(vmId) },
                  search: stackPath ? { stackPath } : {},
                })
              }
            >
              Containers
            </button>
          ) : null}
          <button
            type="button"
            className="secondary"
            disabled={busy != null}
            onClick={() => setPanel(panel === "replace" ? null : "replace")}
          >
            Replace
          </button>
          <button
            type="button"
            className="secondary"
            disabled={busy != null}
            onClick={() => setPending({ kind: "delete" })}
          >
            Delete
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
      {detailQuery.isError ? (
        <div className="card error-card">
          <h3>Inspect fehlgeschlagen</h3>
          <p>{String(detailQuery.error)}</p>
        </div>
      ) : null}

      {panel === "modify" ? (
        <form className="card vm-form" onSubmit={(e) => void submitModify(e)}>
          <h3>Ressourcen ändern</h3>
          <p className="muted">Wirkt erst nach Stop/Start (kein Hotplug).</p>
          <div className="form-grid">
            <label>
              CPUs
              <input
                type="number"
                min={1}
                value={cpus}
                disabled={busy != null}
                onChange={(e) => setCpus(Number(e.target.value))}
              />
            </label>
            <label>
              Memory
              <input
                value={memory}
                disabled={busy != null}
                onChange={(e) => setMemory(e.target.value)}
              />
            </label>
          </div>
          <div className="row" style={{ gap: "0.5rem" }}>
            <button type="submit" disabled={busy != null}>
              {busy === "modify" ? "Speichern…" : "Speichern"}
            </button>
            <button
              type="button"
              className="secondary"
              onClick={() => setPanel(null)}
            >
              Abbrechen
            </button>
          </div>
        </form>
      ) : null}

      {panel === "mount" ? (
        <VmMountForm
          vmId={vmId}
          onDone={async () => {
            setPanel(null);
            await refresh();
          }}
          onCancel={() => setPanel(null)}
        />
      ) : null}

      {panel === "replace" ? (
        <VmForm
          mode="replace"
          initial={{
            id: vmId,
            from: "ubuntu",
            dataDiskGib: 8,
            cpus: inspect?.resources?.cpus,
            memory: inspect?.resources
              ? String(inspect.resources.memory_mib)
              : undefined,
          }}
          onSubmitReplace={async (input) => {
            setPending({ kind: "replace", input });
          }}
          onDone={() => {
            /* replace completes via ConfirmDialog */
          }}
          onCancel={() => setPanel(null)}
        />
      ) : null}

      {panel === "console" ? (
        <div className="card terminal-card">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <h3>Serial Console</h3>
            <button
              type="button"
              className="secondary"
              onClick={() => setPanel(null)}
            >
              Schließen
            </button>
          </div>
          <Terminal mode="attach" vmId={vmId} />
        </div>
      ) : null}

      {panel === "shell" ? (
        <div className="card terminal-card">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <h3>Shell</h3>
            <button
              type="button"
              className="secondary"
              onClick={() => setPanel(null)}
            >
              Schließen
            </button>
          </div>
          <Terminal mode="exec" vmId={vmId} cmd={["/bin/bash"]} />
        </div>
      ) : null}

      <div className="dash-grid">
        <div className="card">
          <h3>Overview</h3>
          {detailQuery.isLoading ? (
            <p className="muted">Laden…</p>
          ) : (
            <dl className="kv">
              <dt>Bundle</dt>
              <dd className="mono">{vm?.bundle ?? "—"}</dd>
              <dt>Managed-by</dt>
              <dd>{vm?.["managed-by"] ?? "—"}</dd>
              <dt>Roles</dt>
              <dd>{vm?.roles?.join(", ") || "—"}</dd>
              <dt>IPs</dt>
              <dd className="mono">
                {(inspect?.networks ?? [])
                  .map((n) => (n.name ? `${n.name}:${n.ip ?? "?"}` : n.ip))
                  .filter(Boolean)
                  .join(", ") || "—"}
              </dd>
              <dt>Serial log</dt>
              <dd className="mono">{inspect?.logs?.serial ?? "—"}</dd>
              <dt>Agent</dt>
              <dd>
                {typeof inspect?.agent?.state === "string"
                  ? inspect.agent.state
                  : "—"}
              </dd>
            </dl>
          )}
        </div>

        <div className="card">
          <h3>Mounts</h3>
          {mountsQuery.isLoading ? (
            <p className="muted">Laden…</p>
          ) : mounts.length === 0 ? (
            <p className="muted">Keine Mounts.</p>
          ) : (
            <ul className="mount-list">
              {mounts.map((mount) => (
                <li key={`${mount.name}:${mount.target}`}>
                  <div>
                    <strong>{mount.name}</strong>{" "}
                    <span className="muted">
                      {mount.read_only ? "ro" : "rw"}
                    </span>
                    <div className="mono path">
                      {mount.target} ← {mount.source}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="secondary"
                    disabled={busy != null}
                    onClick={() => setPending({ kind: "unmount", mount })}
                  >
                    Unmount
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {inspect?.warnings?.length ? (
        <div className="card">
          <h3>Warnings</h3>
          <ul>
            {inspect.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      ) : null}

      <ConfirmDialog
        open={pending?.kind === "delete"}
        title="VM löschen"
        message={`VM „${vmId}“ wirklich löschen? Bundle und Attachments werden entfernt.`}
        confirmLabel="Löschen"
        busy={busy === "delete"}
        error={pending?.kind === "delete" ? error : null}
        onCancel={() => {
          if (busy !== "delete") {
            setPending(null);
            setError(null);
          }
        }}
        onConfirm={() => {
          setError(null);
          deleteMutation.mutate();
        }}
      />

      <ConfirmDialog
        open={pending?.kind === "unmount"}
        title="Mount entfernen"
        message={
          pending?.kind === "unmount"
            ? `Mount „${pending.mount.name}“ (${pending.mount.target}) wirklich entfernen?`
            : ""
        }
        confirmLabel="Unmount"
        busy={busy?.startsWith("unmount:") === true}
        onCancel={() => {
          if (!busy?.startsWith("unmount:")) setPending(null);
        }}
        onConfirm={() => {
          if (pending?.kind === "unmount") void removeMount(pending.mount);
        }}
      />

      <ConfirmDialog
        open={pending?.kind === "replace"}
        title="VM ersetzen"
        message={`VM „${vmId}“ löschen und neu anlegen? Bestehende Daten gehen verloren.`}
        confirmLabel="Ersetzen"
        busy={busy === "replace"}
        onCancel={() => {
          if (busy !== "replace") setPending(null);
        }}
        onConfirm={() => {
          if (pending?.kind === "replace") void replaceVm(pending.input);
        }}
      />
    </section>
  );
}
