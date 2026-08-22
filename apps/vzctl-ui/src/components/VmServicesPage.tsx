import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { SystemdUnitDetailDialog } from "@/components/vm-services/SystemdUnitDetailDialog";
import { UnitStatusBadge } from "@/components/vm-services/UnitStatusBadge";
import {
  VmServicesToolbar,
  type SystemdStateFilter,
} from "@/components/vm-services/VmServicesToolbar";
import {
  ActionRow,
  Alert,
  Badge,
  Button,
  DataTable,
  EmptyState,
  LoadingState,
  PageHeader,
  TableCard,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import {
  fetchSystemdStatus,
  isUnitActive,
  listSystemdUnits,
  restartSystemdUnit,
  splitUnitName,
  startSystemdUnit,
  stopSystemdUnit,
  systemdKeys,
  type SystemdUnit,
  type SystemdUnitType,
} from "@/lib/systemd";
import { inspectVm, isRunning, vmKeys } from "@/lib/vms";

type PendingConfirm =
  | { kind: "stop"; unit: SystemdUnit }
  | { kind: "restart"; unit: SystemdUnit }
  | null;

const UNIT_TYPES: SystemdUnitType[] = ["service", "timer", "socket"];

function filterUnits(
  units: SystemdUnit[],
  query: string,
  stateFilter: SystemdStateFilter,
): SystemdUnit[] {
  const needle = query.trim().toLowerCase();
  let list = units;
  if (stateFilter === "running") {
    list = list.filter(isUnitActive);
  } else if (stateFilter === "inactive") {
    list = list.filter((unit) => !isUnitActive(unit));
  }
  if (needle) {
    list = list.filter(
      (unit) =>
        unit.name.toLowerCase().includes(needle) ||
        unit.description.toLowerCase().includes(needle),
    );
  }
  return [...list].sort((a, b) => {
    const aLive = isUnitActive(a);
    const bLive = isUnitActive(b);
    if (aLive !== bLive) return aLive ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

function countRunning(units: SystemdUnit[]): number {
  return units.reduce((sum, unit) => sum + (isUnitActive(unit) ? 1 : 0), 0);
}

export function VmServicesPage({ vmId }: { vmId: string }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [unitType, setUnitType] = useState<SystemdUnitType>("service");
  const [query, setQuery] = useState("");
  const [stateFilter, setStateFilter] = useState<SystemdStateFilter>("all");
  const [busyUnit, setBusyUnit] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingConfirm>(null);
  const [selectedUnit, setSelectedUnit] = useState<SystemdUnit | null>(null);

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
  const filteredUnits = useMemo(
    () => filterUnits(units, query, stateFilter),
    [units, query, stateFilter],
  );
  const runningCount = useMemo(() => countRunning(units), [units]);
  const systemdAvailable = statusQuery.data?.available === true;

  const tabCounts = useMemo(
    () =>
      Object.fromEntries(
        UNIT_TYPES.map((type) => [
          type,
          type === unitType ? units.length : undefined,
        ]),
      ) as Record<SystemdUnitType, number | undefined>,
    [unitType, units.length],
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
        <LoadingState message={t("common.loading")} card />
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
    <div className="systemd-page">
      <PageHeader
        title={t("systemd.title")}
        subtitle={t("systemd.subtitle")}
        actions={
          statusQuery.data?.version ? (
            <Badge tone="ok">
              {t("systemd.version", { version: statusQuery.data.version })}
            </Badge>
          ) : null
        }
      />

      <div className="systemd-stats">
        <div className="systemd-stat">
          <span className="systemd-stat-value">{units.length}</span>
          <span className="systemd-stat-label">{t("systemd.statTotal")}</span>
        </div>
        <div className="systemd-stat systemd-stat-running">
          <span className="systemd-stat-value">{runningCount}</span>
          <span className="systemd-stat-label">{t("systemd.statRunning")}</span>
        </div>
        <div className="systemd-stat">
          <span className="systemd-stat-value">{units.length - runningCount}</span>
          <span className="systemd-stat-label">{t("systemd.statInactive")}</span>
        </div>
      </div>

      {error ? <Alert title={t("common.error")}>{error}</Alert> : null}
      {message ? (
        <p className="systemd-toast" role="status">
          {message}
        </p>
      ) : null}

      <TableCard className="systemd-table-card">
        <VmServicesToolbar
          unitType={unitType}
          onUnitTypeChange={(type) => {
            setUnitType(type);
            setQuery("");
            setStateFilter("all");
          }}
          counts={tabCounts}
          query={query}
          onQueryChange={setQuery}
          stateFilter={stateFilter}
          onStateFilterChange={setStateFilter}
        />

        {unitsQuery.isLoading ? (
          <div className="systemd-table-body">
            <LoadingState message={t("common.loading")} />
          </div>
        ) : unitsQuery.isError ? (
          <div className="systemd-table-body">
            <Alert title={t("common.error")}>{String(unitsQuery.error)}</Alert>
          </div>
        ) : units.length === 0 ? (
          <div className="systemd-table-body">
            <EmptyState message={t("systemd.empty")} card={false} />
          </div>
        ) : filteredUnits.length === 0 ? (
          <div className="systemd-table-body">
            <EmptyState message={t("systemd.noMatches")} card={false} />
          </div>
        ) : (
          <DataTable>
            <thead>
              <tr>
                <th>{t("systemd.col.unit")}</th>
                <th>{t("systemd.col.status")}</th>
                <th>{t("systemd.col.description")}</th>
                <th className="systemd-col-actions">{t("systemd.col.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {filteredUnits.map((unit) => {
                const active = isUnitActive(unit);
                const busy = busyUnit === unit.name;
                const { base, suffix } = splitUnitName(unit.name);
                return (
                  <tr
                    key={unit.name}
                    data-active={active ? "true" : "false"}
                    className="systemd-row-clickable"
                    onClick={() => setSelectedUnit(unit)}
                  >
                    <td className="systemd-col-unit">
                      <span className="unit-name" title={unit.name}>
                        {base}
                        {suffix ? <span className="unit-name-suffix">{suffix}</span> : null}
                      </span>
                    </td>
                    <td>
                      <UnitStatusBadge unit={unit} />
                    </td>
                    <td className="systemd-col-description" title={unit.description}>
                      {unit.description || t("common.emDash")}
                    </td>
                    <td
                      className="systemd-col-actions"
                      onClick={(event) => event.stopPropagation()}
                    >
                      <ActionRow align="end" gap="sm" className="systemd-actions">
                        {active ? (
                          <>
                            <Button
                              tone="quiet"
                              className="systemd-action-btn"
                              disabled={busy || lifecycle.isPending}
                              onClick={() => setPending({ kind: "stop", unit })}
                            >
                              {busy ? t("common.ellipsis") : t("systemd.stop")}
                            </Button>
                            <Button
                              tone="secondary"
                              className="systemd-action-btn"
                              disabled={busy || lifecycle.isPending}
                              onClick={() => setPending({ kind: "restart", unit })}
                            >
                              {busy ? t("common.ellipsis") : t("systemd.restart")}
                            </Button>
                          </>
                        ) : (
                          <Button
                            tone="secondary"
                            className="systemd-action-btn"
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
          </DataTable>
        )}
      </TableCard>

      <SystemdUnitDetailDialog
        open={selectedUnit != null}
        vmId={vmId}
        unit={selectedUnit}
        onClose={() => setSelectedUnit(null)}
        onChanged={invalidate}
      />

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
    </div>
  );
}
