import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  ActionRow,
  Alert,
  Button,
  EmptyState,
  LoadingState,
  PageHeader,
  StatusPill,
  TableCard,
  Toolbar,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import {
  fetchSystemdStatus,
  isUnitActive,
  listSystemdUnits,
  restartSystemdUnit,
  startSystemdUnit,
  stopSystemdUnit,
  systemdKeys,
  type SystemdUnit,
  type SystemdUnitType,
  unitStatusLabel,
} from "@/lib/systemd";
import { inspectVm, isRunning, vmKeys } from "@/lib/vms";

type PendingConfirm =
  | { kind: "stop"; unit: SystemdUnit }
  | { kind: "restart"; unit: SystemdUnit }
  | null;

const UNIT_TYPES: SystemdUnitType[] = ["service", "timer", "socket"];

export function VmServicesPage({
  vmId,
}: {
  vmId: string;
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const [unitType, setUnitType] = useState<SystemdUnitType>("service");
  const [busyUnit, setBusyUnit] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingConfirm>(null);

  const vmQuery = useQuery({
    queryKey: vmKeys.detail(vmId),
    queryFn: () => inspectVm(vmId),
    refetchInterval: 5000,
  });

  const running = isRunning(vmQuery.data?.vm.state);

  const statusQuery = useQuery({
    queryKey: systemdKeys.status(vmId),
    queryFn: () => fetchSystemdStatus(vmId),
    enabled: running,
    refetchInterval: 15000,
  });

  const unitsQuery = useQuery({
    queryKey: systemdKeys.units(vmId, unitType),
    queryFn: () => listSystemdUnits(vmId, unitType),
    enabled: running && statusQuery.data?.available === true,
    refetchInterval: 5000,
  });

  const invalidate = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: systemdKeys.units(vmId, unitType) }),
      queryClient.invalidateQueries({ queryKey: systemdKeys.status(vmId) }),
    ]);
  };

  const lifecycle = useMutation({
    mutationFn: async (input: { action: "start" | "stop" | "restart"; unit: string }) => {
      setBusyUnit(input.unit);
      setError(null);
      if (input.action === "start") {
        await startSystemdUnit(vmId, input.unit);
      } else if (input.action === "stop") {
        await stopSystemdUnit(vmId, input.unit);
      } else {
        await restartSystemdUnit(vmId, input.unit);
      }
    },
    onSuccess: async (_data, input) => {
      setMessage(
        t("systemd.actionDone", {
          action: input.action,
          unit: input.unit,
        }),
      );
      await invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyUnit(null),
  });

  const units = unitsQuery.data ?? [];
  const systemdAvailable = statusQuery.data?.available === true;

  const typeLabel = useMemo(
    () => ({
      service: t("systemd.tab.services"),
      timer: t("systemd.tab.timers"),
      socket: t("systemd.tab.sockets"),
    }),
    [t],
  );

  if (!running) {
    return (
      <>
        <PageHeader title={t("systemd.title")} />
        <EmptyState message={t("systemd.vmNotRunning")} />
      </>
    );
  }

  if (statusQuery.isLoading) {
    return (
      <>
        <PageHeader title={t("systemd.title")} />
        <LoadingState message={t("common.loading")} />
      </>
    );
  }

  if (statusQuery.isError) {
    return (
      <>
        <PageHeader title={t("systemd.title")} />
        <Alert title={t("common.error")}>{String(statusQuery.error)}</Alert>
      </>
    );
  }

  if (!systemdAvailable) {
    return (
      <>
        <PageHeader title={t("systemd.title")} />
        <EmptyState message={t("systemd.unavailable")} />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={t("systemd.title")}
        subtitle={
          statusQuery.data?.version
            ? t("systemd.version", { version: statusQuery.data.version })
            : undefined
        }
      />

      {error ? <Alert title={t("common.error")}>{error}</Alert> : null}
      {message ? <p className="ok-banner">{message}</p> : null}

      <Toolbar>
        {UNIT_TYPES.map((type) => (
          <Button
            key={type}
            tone={unitType === type ? "primary" : "secondary"}
            onClick={() => setUnitType(type)}
          >
            {typeLabel[type]}
          </Button>
        ))}
      </Toolbar>

      {unitsQuery.isLoading ? (
        <LoadingState message={t("common.loading")} />
      ) : unitsQuery.isError ? (
        <Alert title={t("common.error")}>{String(unitsQuery.error)}</Alert>
      ) : units.length === 0 ? (
        <EmptyState message={t("systemd.empty")} />
      ) : (
        <TableCard>
          <table>
            <thead>
              <tr>
                <th>{t("systemd.col.unit")}</th>
                <th>{t("systemd.col.status")}</th>
                <th>{t("systemd.col.description")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {units.map((unit) => {
                const active = isUnitActive(unit);
                const busy = busyUnit === unit.name;
                return (
                  <tr key={unit.name}>
                    <td>{unit.name}</td>
                    <td>
                      <StatusPill state={active ? "running" : "stopped"}>
                        {unitStatusLabel(unit)}
                      </StatusPill>
                    </td>
                    <td>{unit.description || "—"}</td>
                    <td>
                      <ActionRow>
                        {active ? (
                          <>
                            <Button
                              tone="secondary"
                              disabled={busy || lifecycle.isPending}
                              onClick={() => setPending({ kind: "stop", unit })}
                            >
                              {busy ? t("common.ellipsis") : t("systemd.stop")}
                            </Button>
                            <Button
                              tone="secondary"
                              disabled={busy || lifecycle.isPending}
                              onClick={() =>
                                setPending({ kind: "restart", unit })
                              }
                            >
                              {busy ? t("common.ellipsis") : t("systemd.restart")}
                            </Button>
                          </>
                        ) : (
                          <Button
                            tone="secondary"
                            disabled={busy || lifecycle.isPending}
                            onClick={() =>
                              lifecycle.mutate({
                                action: "start",
                                unit: unit.name,
                              })
                            }
                          >
                            {busy ? t("common.ellipsis") : t("systemd.start")}
                          </Button>
                        )}
                      </ActionRow>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </TableCard>
      )}

      <ConfirmDialog
        open={pending != null}
        title={
          pending?.kind === "stop"
            ? t("systemd.stopTitle")
            : t("systemd.restartTitle")
        }
        message={
          pending
            ? t("systemd.confirmNamed", {
                action:
                  pending.kind === "stop"
                    ? t("systemd.stop")
                    : t("systemd.restart"),
                unit: pending.unit.name,
              })
            : ""
        }
        confirmLabel={
          pending?.kind === "stop" ? t("systemd.stop") : t("systemd.restart")
        }
        onCancel={() => setPending(null)}
        onConfirm={() => {
          if (!pending) return;
          const action = pending.kind === "stop" ? "stop" : "restart";
          lifecycle.mutate({ action, unit: pending.unit.name });
          setPending(null);
        }}
      />
    </>
  );
}
