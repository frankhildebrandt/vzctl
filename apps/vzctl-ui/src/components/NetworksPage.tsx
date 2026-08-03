import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  Alert,
  Badge,
  Button,
  DataTable,
  EmptyState,
  FieldError,
  LoadingState,
  Mono,
  PageHeader,
  TableCard,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import { deleteNet, getDefaultNet, listNets, netKeys, type NetRecord } from "@/lib/nets";

export function NetworksPage() {
  const t = useT();
  const queryClient = useQueryClient();
  const [pendingDelete, setPendingDelete] = useState<NetRecord | null>(null);

  const netsQuery = useQuery({
    queryKey: netKeys.list(),
    queryFn: listNets,
    refetchInterval: 8_000,
  });
  const defaultQuery = useQuery({
    queryKey: netKeys.default(),
    queryFn: getDefaultNet,
    refetchInterval: 8_000,
  });

  const remove = useMutation({
    mutationFn: async (name: string) => deleteNet(name),
    onSuccess: async () => {
      setPendingDelete(null);
      await queryClient.invalidateQueries({ queryKey: netKeys.all });
    },
  });

  const networks = netsQuery.data?.networks ?? [];
  const attachments = netsQuery.data?.attachments ?? [];
  const attachmentCount = useMemo(() => {
    const map = new Map<string, number>();
    for (const row of attachments) {
      const key = row.network_name ?? row.network;
      if (!key) continue;
      map.set(key, (map.get(key) ?? 0) + 1);
    }
    return map;
  }, [attachments]);

  return (
    <section>
      <PageHeader
        title={t("networks.title")}
        subtitle={
          <>
            {t("networks.subtitle")}
            {defaultQuery.data
              ? t("networks.default", {
                  name: defaultQuery.data.name,
                  cidr: defaultQuery.data.cidr ?? "",
                })
              : ""}
          </>
        }
      />

      {netsQuery.isError && (
        <Alert title={t("common.error")}>{String(netsQuery.error)}</Alert>
      )}
      {netsQuery.isLoading && <LoadingState message={t("networks.loading")} />}

      {!netsQuery.isLoading && networks.length === 0 && (
        <EmptyState message={t("networks.empty")} />
      )}

      {networks.length > 0 && (
        <TableCard>
          <DataTable>
            <thead>
              <tr>
                <th>{t("networks.col.name")}</th>
                <th>{t("networks.col.cidr")}</th>
                <th>{t("networks.col.backend")}</th>
                <th>{t("networks.col.state")}</th>
                <th>{t("networks.col.attachments")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {networks.map((net) => (
                <tr key={net.name}>
                  <td>
                    <Mono>{net.name}</Mono>
                    {defaultQuery.data?.name === net.name ? (
                      <Badge>{t("networks.badgeDefault")}</Badge>
                    ) : null}
                  </td>
                  <td>{net.cidr ?? t("common.emDash")}</td>
                  <td>{net.backend ?? "vmnet"}</td>
                  <td>{net.runtime_state ?? t("common.emDash")}</td>
                  <td>{attachmentCount.get(net.name) ?? 0}</td>
                  <td>
                    <Button tone="danger" onClick={() => setPendingDelete(net)}>
                      {t("networks.delete")}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </DataTable>
        </TableCard>
      )}

      <ConfirmDialog
        open={pendingDelete != null}
        title={t("networks.deleteTitle")}
        message={
          pendingDelete
            ? t("networks.deleteMessage", { name: pendingDelete.name })
            : ""
        }
        confirmLabel={t("dialog.delete")}
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          if (pendingDelete) remove.mutate(pendingDelete.name);
        }}
      />
      <FieldError message={remove.isError ? String(remove.error) : null} />
    </section>
  );
}
