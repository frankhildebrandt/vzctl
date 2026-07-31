import { Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { IconButton, IconPlay, IconStop, IconTrash } from "@/components/IconButton";
import { StackCardsSection } from "@/components/StackCard";
import { VmForm } from "@/components/VmForm";
import { listProjects, projectKeys } from "@/lib/projects";
import {
  deleteVm,
  encodeVmIdParam,
  isRunning,
  listVms,
  startVm,
  stopVm,
  vmKeys,
  type VmListItem,
} from "@/lib/vms";
import { listen } from "@tauri-apps/api/event";
import type { VzctlEvent } from "@/lib/vzctl";

export function VmListPage() {
  const queryClient = useQueryClient();
  const [showCreate, setShowCreate] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const listQuery = useQuery({
    queryKey: vmKeys.list(),
    queryFn: listVms,
    refetchInterval: 5000,
  });
  const projectsQuery = useQuery({
    queryKey: projectKeys.all,
    queryFn: listProjects,
  });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        await invoke("subscribe_events");
        unlisten = await listen<VzctlEvent>("vzctl-event", (event) => {
          if (event.payload?.type?.startsWith("vm.")) {
            void queryClient.invalidateQueries({ queryKey: vmKeys.all });
          }
        });
      } catch {
        // events optional when supervisor is down
      }
    })();
    return () => {
      unlisten?.();
    };
  }, [queryClient]);

  const startMutation = useMutation({
    mutationFn: startVm,
    onMutate: (id) => {
      setBusyId(id);
      setError(null);
    },
    onSettled: async () => {
      setBusyId(null);
      await queryClient.invalidateQueries({ queryKey: vmKeys.all });
    },
    onError: (err) => setError(String(err)),
  });

  const stopMutation = useMutation({
    mutationFn: stopVm,
    onMutate: (id) => {
      setBusyId(id);
      setError(null);
    },
    onSettled: async () => {
      setBusyId(null);
      await queryClient.invalidateQueries({ queryKey: vmKeys.all });
    },
    onError: (err) => setError(String(err)),
  });

  const deleteMutation = useMutation({
    mutationFn: async (id: string) => {
      const envelope = await deleteVm(id, true);
      return { id, envelope };
    },
    onMutate: (id) => {
      setBusyId(id);
      setError(null);
    },
    onSuccess: () => {
      setPendingDeleteId(null);
    },
    onSettled: async () => {
      setBusyId(null);
      await queryClient.invalidateQueries({ queryKey: vmKeys.all });
    },
    onError: (err) => setError(String(err)),
  });

  const vms = listQuery.data ?? [];
  const projects = projectsQuery.data ?? [];
  const deleting = deleteMutation.isPending;

  return (
    <section>
      <StackCardsSection
        title="Stacks"
        projects={projects}
        emptyHint={
          <>
            Noch keine Stacks.{" "}
            <Link to="/projects">Stack hinzufügen</Link> und gemeinsam starten.
          </>
        }
      />

      <div className="row" style={{ justifyContent: "space-between" }}>
        <div>
          <h2 className="section-title">VMs</h2>
          <p className="muted">Einzelne Bundles und Runtime-State.</p>
        </div>
        <button type="button" onClick={() => setShowCreate((v) => !v)}>
          {showCreate ? "Abbrechen" : "VM erstellen…"}
        </button>
      </div>

      {showCreate ? (
        <VmForm
          mode="create"
          onDone={async () => {
            setShowCreate(false);
            await queryClient.invalidateQueries({ queryKey: vmKeys.all });
          }}
          onCancel={() => setShowCreate(false)}
        />
      ) : null}

      {error ? (
        <div className="card error-card">
          <h3>Fehler</h3>
          <p>{error}</p>
        </div>
      ) : null}

      {listQuery.isError ? (
        <div className="card error-card">
          <h3>Liste fehlgeschlagen</h3>
          <p>{String(listQuery.error)}</p>
        </div>
      ) : null}

      {listQuery.isLoading ? (
        <p className="muted">Lade VMs…</p>
      ) : vms.length === 0 ? (
        <div className="card">
          <h2>Keine VMs</h2>
          <p className="muted">
            Erstelle eine VM oder starte einen{" "}
            <Link to="/projects">Stack</Link>.
          </p>
        </div>
      ) : (
        <div className="card" style={{ padding: 0, overflow: "auto" }}>
          <table className="vm-table">
            <thead>
              <tr>
                <th>ID</th>
                <th>State</th>
                <th>IPs</th>
                <th>Roles</th>
                <th>PID</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {vms.map((vm) => (
                <VmRow
                  key={vm.id}
                  vm={vm}
                  busy={busyId === vm.id}
                  onStart={() => startMutation.mutate(vm.id)}
                  onStop={() => stopMutation.mutate(vm.id)}
                  onDelete={() => setPendingDeleteId(vm.id)}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}

      <ConfirmDialog
        open={pendingDeleteId != null}
        title="VM löschen"
        message={
          pendingDeleteId
            ? `VM „${pendingDeleteId}“ wirklich löschen? Bundle und Attachments werden entfernt.`
            : ""
        }
        confirmLabel="Löschen"
        busy={deleting}
        error={pendingDeleteId != null ? error : null}
        onCancel={() => {
          if (!deleting) {
            setPendingDeleteId(null);
            setError(null);
          }
        }}
        onConfirm={() => {
          const id = pendingDeleteId;
          if (id) {
            setError(null);
            deleteMutation.mutate(id);
          }
        }}
      />
    </section>
  );
}

function VmRow({
  vm,
  busy,
  onStart,
  onStop,
  onDelete,
}: {
  vm: VmListItem;
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
  onDelete: () => void;
}) {
  const running = isRunning(vm.state);
  return (
    <tr>
      <td>
        <Link
          to="/vms/$vmId"
          params={{ vmId: encodeVmIdParam(vm.id) }}
          className="project-name"
        >
          {vm.id}
        </Link>
      </td>
      <td>
        <span className={`vm-state state-${vm.state}`}>{vm.state}</span>
      </td>
      <td className="mono">{vm.ips.join(", ") || "—"}</td>
      <td>{vm.roles.join(", ") || "—"}</td>
      <td className="mono">{vm.pid ?? "—"}</td>
      <td>
        <div className="row" style={{ gap: "0.35rem", justifyContent: "flex-end" }}>
          {running ? (
            <IconButton label="Stop" disabled={busy} onClick={onStop} tone="danger">
              <IconStop />
            </IconButton>
          ) : (
            <IconButton label="Start" disabled={busy} onClick={onStart} tone="primary">
              <IconPlay />
            </IconButton>
          )}
          <IconButton label="Löschen" disabled={busy} onClick={onDelete} tone="danger">
            <IconTrash />
          </IconButton>
        </div>
      </td>
    </tr>
  );
}
