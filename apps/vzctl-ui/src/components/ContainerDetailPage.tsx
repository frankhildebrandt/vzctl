import { Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Terminal } from "@/components/Terminal";
import {
  ActionRow,
  Alert,
  Button,
  Card,
  DescriptionList,
  JsonBlock,
  LoadingState,
  Mono,
  PageHeader,
  StatusPill,
  Toolbar,
} from "@/components/ui";
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
        <PageHeader
          layout="detail"
          breadcrumbs={<Breadcrumbs items={crumbs} />}
          title={displayName}
        />
        <Alert title={t("common.error")}>{t("containers.noProject")}</Alert>
      </section>
    );
  }

  return (
    <section>
      <PageHeader
        breadcrumbs={<Breadcrumbs items={crumbs} />}
        title={displayName}
        subtitle={
          <>
            <StatusPill state={up ? "running" : "stopped"}>
              {listed?.status || state || t("common.emDash")}
            </StatusPill>
            {" · "}
            <Mono>{shortContainerId(containerId)}</Mono>
          </>
        }
        actions={
          <Toolbar>
          {up ? (
            <>
              <Button
                tone="secondary"
                disabled={busy != null || !running}
                onClick={() => setPending("stop")}
              >
                {busy === "stop" ? `${t("containers.stop")}…` : t("containers.stop")}
              </Button>
              <Button
                tone="secondary"
                disabled={busy != null || !running}
                onClick={() => setPending("restart")}
              >
                {busy === "restart" ? `${t("containers.restart")}…` : t("containers.restart")}
              </Button>
            </>
          ) : (
            <Button
              disabled={busy != null || !running}
              onClick={() => lifecycle.mutate("start")}
            >
              {busy === "start" ? t("containers.startBusy") : t("containers.start")}
            </Button>
          )}
          <Button
            tone="secondary"
            disabled={!running || !up || busy != null}
            onClick={() => setShowShell((v) => !v)}
          >
            {t("vmDetail.shell")}
          </Button>
          </Toolbar>
        }
      />

      {message ? <p className="ok-banner">{message}</p> : null}
      {error ? (
        <Alert title={t("common.error")}>{error}</Alert>
      ) : null}

      {showShell ? (
        <Card className="terminal-card">
          <ActionRow align="between">
            <h3>{t("containerDetail.shellTitle")}</h3>
            <Button tone="secondary" onClick={() => setShowShell(false)}>
              {t("common.close")}
            </Button>
          </ActionRow>
          <Terminal
            mode="exec"
            vmId={vmId}
            cmd={["docker", "exec", "-it", containerId, "/bin/sh"]}
          />
        </Card>
      ) : null}

      <div className="dash-grid">
        <Card title={t("containerDetail.overview")} titleAs="h3">
          <DescriptionList
            items={[
              { label: t("containerDetail.id"), value: <Mono>{containerId}</Mono> },
              { label: t("containerDetail.name"), value: displayName },
              {
                label: t("containerDetail.image"),
                value: (
                  <Mono>
                    {listed?.image ||
                      (typeof inspectQuery.data?.Config === "object" &&
                      inspectQuery.data.Config &&
                      typeof (inspectQuery.data.Config as Record<string, unknown>).Image ===
                        "string"
                        ? String((inspectQuery.data.Config as Record<string, unknown>).Image)
                        : t("common.emDash"))}
                  </Mono>
                ),
              },
              {
                label: t("containerDetail.ports"),
                value: <Mono>{listed?.ports || t("common.emDash")}</Mono>,
              },
            ]}
          />
        </Card>

        <Card title={t("containerDetail.inspect")} titleAs="h3">
          {inspectQuery.isLoading ? (
            <LoadingState message={t("common.loading")} />
          ) : inspectQuery.isError ? (
            <Alert title={t("common.error")}>{String(inspectQuery.error)}</Alert>
          ) : (
            <JsonBlock value={inspectQuery.data ?? {}} />
          )}
        </Card>
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
