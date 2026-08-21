import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ChromeActions, ChromeCrumbs } from "@/components/Chrome";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  IconButton,
  IconPlay,
  IconRestart,
  IconStop,
} from "@/components/IconButton";
import { VmMetrics } from "@/components/VmMetrics";
import { VmShellWorkspace } from "@/components/VmShellWorkspace";
import { Alert, Muted, SectionTitle, StatusPill } from "@/components/ui";
import { cx } from "@/components/ui/cx";
import { useT } from "@/lib/i18n";
import { getProject } from "@/lib/projects";
import { registerSidebarAction } from "@/lib/sidebarActions";
import {
  deleteVm,
  inspectVm,
  isRunning,
  restartVm,
  startVm,
  stopVm,
  vmKeys,
} from "@/lib/vms";
import { basename } from "@/lib/vzctl";

function vmLeaf(pathname: string): string {
  const parts = pathname.split("/").filter(Boolean);
  return parts[2] ?? "overview";
}

/** Shared VM chrome, lifecycle icons, and keep-alive terminals. */
export function VmDetailLayout({
  vmId,
  stackPath,
}: {
  vmId: string;
  stackPath?: string;
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const leaf = vmLeaf(pathname);
  const isShell = leaf === "shell";
  const isConsole = leaf === "console";
  const onContainers = leaf === "containers" || pathname.includes("/containers");

  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState(false);

  const stackName = useMemo(() => {
    if (!stackPath) return null;
    return getProject(stackPath)?.name ?? basename(stackPath);
  }, [stackPath]);

  const detailQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
    refetchInterval: 5000,
  });

  const inspect = detailQuery.data;
  const vm = inspect?.vm;
  const running = isRunning(vm?.state);

  useEffect(() => {
    return registerSidebarAction("delete", () => {
      setPendingDelete(true);
      setError(null);
    });
  }, []);

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
    onError: (err) => setError(String(err)),
    onSettled: async () => {
      setBusy(null);
      await refresh();
    },
  });

  const restartMutation = useMutation({
    mutationFn: () => restartVm(vmId),
    onMutate: () => {
      setBusy("restart");
      setError(null);
    },
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
      setPendingDelete(false);
      if (stackPath) {
        void navigate({ to: "/env", search: { path: stackPath, tab: "ops" } });
      } else {
        void navigate({ to: "/vms" });
      }
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusy(null),
  });

  const crumbs =
    stackPath && stackName
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
            label: t("crumb.vms"),
            node: (
              <Link to="/vms" className="crumb-link">
                {t("crumb.vms")}
              </Link>
            ),
          },
          { label: vmId },
        ];

  return (
    <section className="vm-detail">
      {onContainers ? null : (
        <ChromeCrumbs>
          <Breadcrumbs items={crumbs} />
        </ChromeCrumbs>
      )}
      <ChromeActions>
        <IconButton
          label={busy === "start" ? t("vmDetail.startBusy") : t("vmDetail.start")}
          disabled={running || busy != null}
          tone="quiet"
          onClick={() => startMutation.mutate()}
        >
          <IconPlay />
        </IconButton>
        <IconButton
          label={busy === "stop" ? t("vmDetail.stopBusy") : t("vmDetail.stop")}
          disabled={!running || busy != null}
          tone="quiet"
          onClick={() => stopMutation.mutate()}
        >
          <IconStop />
        </IconButton>
        <IconButton
          label={
            busy === "restart" ? t("vmDetail.restartBusy") : t("vmDetail.restart")
          }
          disabled={!running || busy != null}
          tone="quiet"
          onClick={() => restartMutation.mutate()}
        >
          <IconRestart />
        </IconButton>
      </ChromeActions>

      <header className="vm-detail-heading">
        <div className="vm-detail-title-row">
          <SectionTitle>{vmId}</SectionTitle>
          <VmMetrics vmId={vmId} running={running} />
        </div>
        {vm ? (
          <Muted className="vm-detail-status">
            <StatusPill state={vm.state} />
            {vm.pid != null ? ` · pid ${vm.pid}` : null}
            {inspect?.resources
              ? ` · ${inspect.resources.cpus} CPU / ${inspect.resources.memory_mib} MiB`
              : null}
          </Muted>
        ) : null}
      </header>

      {error ? <Alert title={t("common.error")}>{error}</Alert> : null}
      {detailQuery.isError ? (
        <Alert title={t("vmDetail.inspectFailed")}>
          {String(detailQuery.error)}
        </Alert>
      ) : null}

      <div className={cx("vm-pane", !isShell && "is-hidden")}>
        <VmShellWorkspace vmId={vmId} kind="shell" active={isShell} />
      </div>

      <div className={cx("vm-pane", !isConsole && "is-hidden")}>
        <VmShellWorkspace vmId={vmId} kind="console" active={isConsole} />
      </div>

      <div className={cx("vm-page", (isShell || isConsole) && "is-hidden")}>
        <Outlet />
      </div>

      <ConfirmDialog
        open={pendingDelete}
        title={t("vmDetail.deleteTitle")}
        message={t("vmDetail.deleteMessage", { id: vmId })}
        confirmLabel={t("dialog.delete")}
        busy={busy === "delete"}
        error={pendingDelete ? error : null}
        onCancel={() => {
          if (busy !== "delete") {
            setPendingDelete(false);
            setError(null);
          }
        }}
        onConfirm={() => {
          setError(null);
          deleteMutation.mutate();
        }}
      />
    </section>
  );
}
