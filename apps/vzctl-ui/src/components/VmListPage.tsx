import { Link } from "@tanstack/react-router";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { IconButton, IconPlay, IconStop, IconTrash } from "@/components/IconButton";
import { StackCardsSection } from "@/components/StackCard";
import { VmForm } from "@/components/VmForm";
import { useT } from "@/lib/i18n";
import { listProjects, projectKeys } from "@/lib/projects";
import { partitionStacksAndVms } from "@/lib/stackPartition";
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
import { queryKeys, runVzctl, type VzctlEvent } from "@/lib/vzctl";

export function VmListPage() {
  const t = useT();
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

  const projects = projectsQuery.data ?? [];
  const vms = listQuery.data ?? [];

  const statusQueries = useQueries({
    queries: projects.map((project) => ({
      queryKey: queryKeys.status(project.path),
      queryFn: () => runVzctl(project.path, "status"),
      refetchInterval: 8000,
      retry: false,
    })),
  });

  const statusByPath = useMemo(() => {
    const map: Record<string, string | undefined> = {};
    projects.forEach((project, index) => {
      map[project.path] = statusQueries[index]?.data;
    });
    return map;
  }, [projects, statusQueries]);

  const { activeStacks, standaloneVms } = useMemo(
    () =>
      partitionStacksAndVms({
        projects,
        vms,
        statusByPath,
      }),
    [projects, vms, statusByPath],
  );

  const activeProjects = useMemo(
    () => activeStacks.map((entry) => entry.project),
    [activeStacks],
  );

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

  const deleting = deleteMutation.isPending;

  return (
    <section>
      {activeProjects.length > 0 ? (
        <StackCardsSection title={t("vms.stacksTitle")} projects={activeProjects} />
      ) : null}

      <div className="row" style={{ justifyContent: "space-between" }}>
        <div>
          <h2 className="section-title">{t("vms.title")}</h2>
          <p className="muted">{t("vms.subtitle")}</p>
        </div>
        <button type="button" onClick={() => setShowCreate((v) => !v)}>
          {showCreate ? t("vms.cancelCreate") : t("vms.create")}
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
          <h3>{t("common.error")}</h3>
          <p>{error}</p>
        </div>
      ) : null}

      {listQuery.isError ? (
        <div className="card error-card">
          <h3>{t("vms.listFailed")}</h3>
          <p>{String(listQuery.error)}</p>
        </div>
      ) : null}

      {listQuery.isLoading ? (
        <p className="muted">{t("vms.loading")}</p>
      ) : standaloneVms.length === 0 ? (
        <div className="card">
          <h2>{t("vms.emptyTitle")}</h2>
          <p className="muted">
            {activeProjects.length > 0
              ? t("vms.emptyInStacks")
              : t("vms.emptyHint")}
          </p>
        </div>
      ) : (
        <div className="card" style={{ padding: 0, overflow: "auto" }}>
          <table className="vm-table">
            <thead>
              <tr>
                <th>{t("vms.col.id")}</th>
                <th>{t("vms.col.state")}</th>
                <th>{t("vms.col.ips")}</th>
                <th>{t("vms.col.roles")}</th>
                <th>{t("vms.col.pid")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {standaloneVms.map((vm) => (
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
        title={t("vms.deleteTitle")}
        message={
          pendingDeleteId
            ? t("vms.deleteMessage", { id: pendingDeleteId })
            : ""
        }
        confirmLabel={t("dialog.delete")}
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
  const t = useT();
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
      <td className="mono">{vm.ips.join(", ") || t("common.emDash")}</td>
      <td>{vm.roles.join(", ") || t("common.emDash")}</td>
      <td className="mono">{vm.pid ?? t("common.emDash")}</td>
      <td>
        <div className="row" style={{ gap: "0.35rem", justifyContent: "flex-end" }}>
          {running ? (
            <IconButton label={t("vms.stop")} disabled={busy} onClick={onStop} tone="danger">
              <IconStop />
            </IconButton>
          ) : (
            <IconButton label={t("vms.start")} disabled={busy} onClick={onStart} tone="primary">
              <IconPlay />
            </IconButton>
          )}
          <IconButton label={t("common.delete")} disabled={busy} onClick={onDelete} tone="danger">
            <IconTrash />
          </IconButton>
        </div>
      </td>
    </tr>
  );
}
