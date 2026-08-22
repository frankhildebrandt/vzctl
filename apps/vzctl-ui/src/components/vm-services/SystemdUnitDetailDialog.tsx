import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { UnitStatusBadge } from "@/components/vm-services/UnitStatusBadge";
import {
  ActionRow,
  Alert,
  Button,
  DescriptionList,
  Dialog,
  LoadingState,
  Mono,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import {
  fetchSystemdUnitDetail,
  isUnitActive,
  restartSystemdUnit,
  startSystemdUnit,
  stopSystemdUnit,
  systemdKeys,
  type SystemdUnit,
} from "@/lib/systemd";

type PendingConfirm =
  | { kind: "stop"; unit: SystemdUnit }
  | { kind: "restart"; unit: SystemdUnit }
  | null;

type Props = {
  open: boolean;
  vmId: string;
  unit: SystemdUnit | null;
  onClose: () => void;
  onChanged?: () => void | Promise<void>;
};

export function SystemdUnitDetailDialog({
  open,
  vmId,
  unit,
  onClose,
  onChanged,
}: Props) {
  const t = useT();
  const queryClient = useQueryClient();
  const [pending, setPending] = useState<PendingConfirm>(null);
  const [error, setError] = useState<string | null>(null);

  const detailQuery = useQuery({
    queryKey: systemdKeys.detail(vmId, unit?.name ?? ""),
    queryFn: () => fetchSystemdUnitDetail(vmId, unit!.name),
    enabled: open && unit != null,
  });

  const lifecycle = useMutation({
    mutationFn: async (input: { action: "start" | "stop" | "restart"; unit: string }) => {
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
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: systemdKeys.detail(vmId, input.unit),
        }),
        onChanged?.(),
      ]);
      await detailQuery.refetch();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setPending(null),
  });

  const detail = detailQuery.data;
  const summary = detail ?? unit;
  const active = summary ? isUnitActive(summary) : false;
  const busy = lifecycle.isPending;

  return (
    <>
      <Dialog
        open={open && unit != null}
        title={unit?.name ?? t("systemd.detailTitle")}
        onCancel={onClose}
        role="dialog"
        className="systemd-detail-dialog"
        actions={
          <>
            <Button tone="secondary" disabled={busy} onClick={onClose}>
              {t("dialog.close")}
            </Button>
            {summary ? (
              <ActionRow gap="sm">
                {active ? (
                  <>
                    <Button
                      tone="quiet"
                      disabled={busy}
                      onClick={() => setPending({ kind: "stop", unit: summary })}
                    >
                      {t("systemd.stop")}
                    </Button>
                    <Button
                      tone="secondary"
                      disabled={busy}
                      onClick={() => setPending({ kind: "restart", unit: summary })}
                    >
                      {t("systemd.restart")}
                    </Button>
                  </>
                ) : (
                  <Button
                    tone="primary"
                    disabled={busy}
                    onClick={() =>
                      lifecycle.mutate({ action: "start", unit: summary.name })
                    }
                  >
                    {t("systemd.start")}
                  </Button>
                )}
              </ActionRow>
            ) : null}
          </>
        }
      >
        {detailQuery.isLoading ? (
          <LoadingState message={t("common.loading")} />
        ) : detailQuery.isError ? (
          <Alert title={t("common.error")}>{String(detailQuery.error)}</Alert>
        ) : summary ? (
          <div className="systemd-detail-body">
            <div className="systemd-detail-status">
              <UnitStatusBadge unit={summary} />
            </div>
            {error ? <Alert title={t("common.error")}>{error}</Alert> : null}
            <DescriptionList
              stacked
              items={[
                {
                  key: "description",
                  label: t("systemd.col.description"),
                  value: summary.description || t("common.emDash"),
                },
                {
                  key: "type",
                  label: t("systemd.detail.type"),
                  value: summary.type,
                },
                {
                  key: "load",
                  label: t("systemd.detail.load"),
                  value: summary.load || t("common.emDash"),
                },
                {
                  key: "active",
                  label: t("systemd.detail.active"),
                  value: summary.active || t("common.emDash"),
                },
                {
                  key: "sub",
                  label: t("systemd.detail.sub"),
                  value: summary.sub || t("common.emDash"),
                },
                {
                  key: "unit_file",
                  label: t("systemd.detail.unitFile"),
                  value: detail?.unit_file || t("common.emDash"),
                },
                {
                  key: "fragment",
                  label: t("systemd.detail.fragment"),
                  value: detail?.fragment ? (
                    <Mono>{detail.fragment}</Mono>
                  ) : (
                    t("common.emDash")
                  ),
                },
              ]}
            />
          </div>
        ) : null}
      </Dialog>

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
        }}
      />
    </>
  );
}
