import { Link, useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type FormEvent } from "react";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Terminal } from "@/components/Terminal";
import { VmForm } from "@/components/VmForm";
import { VmMountForm } from "@/components/VmMountForm";
import {
  ActionRow,
  Alert,
  Button,
  Card,
  DataTable,
  DescriptionList,
  EmptyState,
  FormActions,
  FormField,
  FormGrid,
  LoadingState,
  Mono,
  PageHeader,
  StatusPill,
  Toolbar,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
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
  const t = useT();
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
    onSuccess: () => setMessage(t("vmDetail.started")),
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
    onSuccess: () => setMessage(t("vmDetail.stopped")),
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

  async function submitModify(event: FormEvent) {
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
          ? t("vmDetail.resourcesRestart")
          : t("vmDetail.resourcesSaved"),
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
      setMessage(t("vmDetail.replaced"));
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
    <section>
      <PageHeader
        breadcrumbs={<Breadcrumbs items={crumbs} />}
        title={vmId}
        subtitle={
          vm ? (
            <>
              <StatusPill state={vm.state} />
              {vm.pid != null ? ` · pid ${vm.pid}` : null}
              {inspect?.resources
                ? ` · ${inspect.resources.cpus} CPU / ${inspect.resources.memory_mib} MiB`
                : null}
            </>
          ) : null
        }
        actions={
          <Toolbar>
          {running ? (
            <Button
              tone="secondary"
              disabled={busy != null}
              onClick={() => stopMutation.mutate()}
            >
              {busy === "stop" ? t("vmDetail.stopBusy") : t("vmDetail.stop")}
            </Button>
          ) : (
            <Button
              disabled={busy != null}
              onClick={() => startMutation.mutate()}
            >
              {busy === "start" ? t("vmDetail.startBusy") : t("vmDetail.start")}
            </Button>
          )}
          <Button
            tone="secondary"
            disabled={busy != null}
            onClick={() => setPanel(panel === "modify" ? null : "modify")}
          >
            {t("vmDetail.modify")}
          </Button>
          <Button
            tone="secondary"
            disabled={busy != null}
            onClick={() => setPanel(panel === "mount" ? null : "mount")}
          >
            {t("vmDetail.mount")}
          </Button>
          <Button
            tone="secondary"
            disabled={!running || busy != null}
            onClick={() => setPanel(panel === "console" ? null : "console")}
          >
            {t("vmDetail.attach")}
          </Button>
          <Button
            tone="secondary"
            disabled={!running || busy != null}
            onClick={() => setPanel(panel === "shell" ? null : "shell")}
          >
            {t("vmDetail.shell")}
          </Button>
          {vm?.roles?.includes("docker") ? (
            <Button
              tone="secondary"
              disabled={!running || busy != null}
              onClick={() =>
                void navigate({
                  to: "/vms/$vmId/containers",
                  params: { vmId: encodeVmIdParam(vmId) },
                  search: stackPath ? { stackPath } : {},
                })
              }
            >
              {t("vmDetail.containers")}
            </Button>
          ) : null}
          <Button
            tone="secondary"
            disabled={busy != null}
            onClick={() => setPanel(panel === "replace" ? null : "replace")}
          >
            {t("vmDetail.replace")}
          </Button>
          <Button
            tone="secondary"
            disabled={busy != null}
            onClick={() => setPending({ kind: "delete" })}
          >
            {t("vmDetail.delete")}
          </Button>
          </Toolbar>
        }
      />

      {message ? <p className="ok-banner">{message}</p> : null}
      {error ? (
        <Alert title={t("common.error")}>{error}</Alert>
      ) : null}
      {detailQuery.isError ? (
        <Alert title={t("vmDetail.inspectFailed")}>{String(detailQuery.error)}</Alert>
      ) : null}

      {panel === "modify" ? (
        <Card
          as="form"
          className="vm-form"
          title={t("vmDetail.modifyTitle")}
          titleAs="h3"
          subtitle={t("vmDetail.modifyHint")}
          onSubmit={(e) => void submitModify(e)}
        >
          <FormGrid>
            <FormField label={t("vmForm.cpus")}>
              <input
                type="number"
                min={1}
                value={cpus}
                disabled={busy != null}
                onChange={(e) => setCpus(Number(e.target.value))}
              />
            </FormField>
            <FormField label={t("vmDetail.memory")}>
              <input
                value={memory}
                disabled={busy != null}
                onChange={(e) => setMemory(e.target.value)}
              />
            </FormField>
          </FormGrid>
          <FormActions
            busy={busy != null}
            submitLabel={busy === "modify" ? t("common.saveBusy") : t("common.save")}
            cancelLabel={t("common.cancel")}
            onCancel={() => setPanel(null)}
          />
        </Card>
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
            diskGib: 8,
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
        <Card className="terminal-card">
          <ActionRow align="between">
            <h3>{t("vmDetail.consoleTitle")}</h3>
            <Button tone="secondary" onClick={() => setPanel(null)}>
              {t("common.close")}
            </Button>
          </ActionRow>
          <Terminal mode="attach" vmId={vmId} />
        </Card>
      ) : null}

      {panel === "shell" ? (
        <Card className="terminal-card">
          <ActionRow align="between">
            <h3>{t("vmDetail.shellTitle")}</h3>
            <Button tone="secondary" onClick={() => setPanel(null)}>
              {t("common.close")}
            </Button>
          </ActionRow>
          <Terminal mode="exec" vmId={vmId} cmd={["/bin/bash"]} />
        </Card>
      ) : null}

      <div className="dash-grid">
        <Card title={t("vmDetail.overview")} titleAs="h3">
          {detailQuery.isLoading ? (
            <LoadingState message={t("common.loading")} />
          ) : (
            <DescriptionList
              items={[
                {
                  label: t("vmDetail.bundle"),
                  value: <Mono>{vm?.bundle ?? t("common.emDash")}</Mono>,
                },
                {
                  label: t("vmDetail.managedBy"),
                  value: vm?.["managed-by"] ?? t("common.emDash"),
                },
                {
                  label: t("vmDetail.roles"),
                  value: vm?.roles?.join(", ") || t("common.emDash"),
                },
                {
                  label: t("vmDetail.ips"),
                  value: (
                    <Mono>
                      {(inspect?.networks ?? [])
                        .map((n) => (n.name ? `${n.name}:${n.ip ?? "?"}` : n.ip))
                        .filter(Boolean)
                        .join(", ") || t("common.emDash")}
                    </Mono>
                  ),
                },
                {
                  label: t("vmDetail.serialLog"),
                  value: <Mono>{inspect?.logs?.serial ?? t("common.emDash")}</Mono>,
                },
                {
                  label: t("vmDetail.agent"),
                  value:
                    typeof inspect?.agent?.state === "string"
                      ? inspect.agent.state
                      : t("common.emDash"),
                },
              ]}
            />
          )}
        </Card>

        <Card title={t("vmDetail.mounts")} titleAs="h3">
          {mountsQuery.isLoading ? (
            <LoadingState message={t("common.loading")} />
          ) : mounts.length === 0 ? (
            <EmptyState card={false} message={t("vmDetail.noMounts")} />
          ) : (
            <DataTable>
              <thead>
                <tr>
                  <th>{t("mount.tag")}</th>
                  <th>{t("mount.target")}</th>
                  <th>{t("mount.source")}</th>
                  <th>{t("mount.readOnly")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {mounts.map((mount) => (
                  <tr key={`${mount.name}:${mount.target}`}>
                    <td>
                      <strong>{mount.name}</strong>
                    </td>
                    <td className="mono">{mount.target}</td>
                    <td className="mono">{mount.source}</td>
                    <td>{mount.read_only ? "ro" : "rw"}</td>
                    <td>
                      <ActionRow align="end" gap="sm">
                        <Button
                          tone="secondary"
                          disabled={busy != null}
                          onClick={() => setPending({ kind: "unmount", mount })}
                        >
                          {t("vmDetail.unmount")}
                        </Button>
                      </ActionRow>
                    </td>
                  </tr>
                ))}
              </tbody>
            </DataTable>
          )}
        </Card>
      </div>

      {inspect?.warnings?.length ? (
        <Card title={t("vmDetail.warnings")} titleAs="h3">
          <ul>
            {inspect.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </Card>
      ) : null}

      <ConfirmDialog
        open={pending?.kind === "delete"}
        title={t("vmDetail.deleteTitle")}
        message={t("vmDetail.deleteMessage", { id: vmId })}
        confirmLabel={t("dialog.delete")}
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
        title={t("vmDetail.unmountTitle")}
        message={
          pending?.kind === "unmount"
            ? t("vmDetail.unmountMessage", {
                name: pending.mount.name,
                target: pending.mount.target,
              })
            : ""
        }
        confirmLabel={t("vmDetail.unmount")}
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
        title={t("vmDetail.replaceTitle")}
        message={t("vmDetail.replaceMessage", { id: vmId })}
        confirmLabel={t("vmDetail.replaceConfirm")}
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
