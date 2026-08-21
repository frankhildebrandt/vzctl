import { useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
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
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import {
  createVm,
  deleteVm,
  encodeVmIdParam,
  inspectVm,
  isRunning,
  listMounts,
  modifyVm,
  stopVm,
  unmountVm,
  vmKeys,
  type CreateVmInput,
  type VmMount,
} from "@/lib/vms";

function vmOverviewSearch(stackPath?: string) {
  return stackPath ? { stackPath } : {};
}

type PendingConfirm =
  | { kind: "unmount"; mount: VmMount }
  | { kind: "replace"; input: CreateVmInput }
  | null;

export function VmOverviewPage({ vmId }: { vmId: string }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingConfirm>(null);

  const detailQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
    refetchInterval: 5000,
  });

  const mountsQuery = useQuery({
    queryKey: vmKeys.mounts(vmId),
    queryFn: () => listMounts(vmId),
  });

  const inspect = detailQuery.data;
  const vm = inspect?.vm;
  const mounts = mountsQuery.data ?? [];

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: vmKeys.detail(vmId) });
    await queryClient.invalidateQueries({ queryKey: vmKeys.mounts(vmId) });
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

  return (
    <>
      {error ? <Alert title={t("common.error")}>{error}</Alert> : null}

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
    </>
  );
}

export function VmModifyPage({
  vmId,
  stackPath,
}: {
  vmId: string;
  stackPath?: string;
}) {
  const t = useT();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [cpus, setCpus] = useState(2);
  const [memory, setMemory] = useState("1024");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const detailQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
  });

  useEffect(() => {
    const resources = detailQuery.data?.resources;
    if (resources) {
      setCpus(resources.cpus);
      setMemory(String(resources.memory_mib));
    }
  }, [detailQuery.data?.resources]);

  async function submitModify(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
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
      await queryClient.invalidateQueries({ queryKey: vmKeys.detail(vmId) });
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      {message ? <p className="ok-banner">{message}</p> : null}
      {error ? <Alert title={t("common.error")}>{error}</Alert> : null}
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
              disabled={busy}
              onChange={(e) => setCpus(Number(e.target.value))}
            />
          </FormField>
          <FormField label={t("vmDetail.memory")}>
            <input
              value={memory}
              disabled={busy}
              onChange={(e) => setMemory(e.target.value)}
            />
          </FormField>
        </FormGrid>
        <FormActions
          busy={busy}
          submitLabel={busy ? t("common.saveBusy") : t("common.save")}
          cancelLabel={t("common.cancel")}
          onCancel={() =>
            void navigate({
              to: "/vms/$vmId",
              params: { vmId: encodeVmIdParam(vmId) },
              search: vmOverviewSearch(stackPath),
            })
          }
        />
      </Card>
    </>
  );
}

export function VmMountPage({
  vmId,
  stackPath,
}: {
  vmId: string;
  stackPath?: string;
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  return (
    <VmMountForm
      vmId={vmId}
      onDone={async () => {
        await queryClient.invalidateQueries({ queryKey: vmKeys.mounts(vmId) });
        await navigate({
          to: "/vms/$vmId",
          params: { vmId: encodeVmIdParam(vmId) },
          search: vmOverviewSearch(stackPath),
        });
      }}
      onCancel={() =>
        void navigate({
          to: "/vms/$vmId",
          params: { vmId: encodeVmIdParam(vmId) },
          search: vmOverviewSearch(stackPath),
        })
      }
    />
  );
}

export function VmReplacePage({
  vmId,
  stackPath,
}: {
  vmId: string;
  stackPath?: string;
}) {
  const t = useT();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingConfirm>(null);

  const detailQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
  });

  const inspect = detailQuery.data;
  const running = isRunning(inspect?.vm?.state);

  async function replaceVm(input: CreateVmInput) {
    setBusy(true);
    setError(null);
    try {
      if (running) {
        await stopVm(vmId);
      }
      await deleteVm(vmId, true);
      await createVm(input);
      setPending(null);
      await queryClient.invalidateQueries({ queryKey: vmKeys.detail(vmId) });
      await queryClient.invalidateQueries({ queryKey: vmKeys.list() });
      await navigate({
        to: "/vms/$vmId",
        params: { vmId: encodeVmIdParam(vmId) },
        search: vmOverviewSearch(stackPath),
      });
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      {error ? <Alert title={t("common.error")}>{error}</Alert> : null}
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
        onCancel={() =>
          void navigate({
            to: "/vms/$vmId",
            params: { vmId: encodeVmIdParam(vmId) },
            search: vmOverviewSearch(stackPath),
          })
        }
      />
      <ConfirmDialog
        open={pending?.kind === "replace"}
        title={t("vmDetail.replaceTitle")}
        message={t("vmDetail.replaceMessage", { id: vmId })}
        confirmLabel={t("vmDetail.replaceConfirm")}
        busy={busy}
        onCancel={() => {
          if (!busy) setPending(null);
        }}
        onConfirm={() => {
          if (pending?.kind === "replace") void replaceVm(pending.input);
        }}
      />
    </>
  );
}
