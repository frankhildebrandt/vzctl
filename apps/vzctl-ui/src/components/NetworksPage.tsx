import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
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
import { deleteNet, getDefaultNet, listNets, netKeys, networkLabel, networkDeleteId, networkAttachmentCount, isDefaultNetwork, type NetRecord } from "@/lib/nets";

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
    mutationFn: async ({ id, project }: { id: string; project?: string }) => deleteNet(id, project),
    onSuccess: async () => {
      setPendingDelete(null);
      await queryClient.invalidateQueries({ queryKey: netKeys.all });
    },
  });

  const networks = netsQuery.data?.networks ?? [];
  const attachments = netsQuery.data?.attachments ?? [];
  const ambiguous = networks.some((net) => networkDeleteId(net, networks) === null);

  return (
    <section>
      <PageHeader
        title={t("networks.title")}
        subtitle={
          <>
            {t("networks.subtitle")}
            {defaultQuery.data
              ? t("networks.default", {
                  name: networkLabel(defaultQuery.data),
                  cidr: defaultQuery.data.cidr ?? "",
                })
              : ""}
          </>
        }
      />

      {netsQuery.isError && (
        <Alert title={t("common.error")}>{String(netsQuery.error)}</Alert>
      )}
      {ambiguous && <Alert title={t("networks.ambiguousTitle")}>{t("networks.ambiguousDesc")}</Alert>}
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
              {networks.map((net, index) => (
                <tr key={net.id ?? `${net.project ?? "global"}/${net.name}/${index}`}>
                  <td>
                    <Mono>{networkLabel(net)}</Mono>
                    {isDefaultNetwork(net, defaultQuery.data ?? null, networks) ? (
                      <Badge>{t("networks.badgeDefault")}</Badge>
                    ) : null}
                  </td>
                  <td>{net.cidr ?? t("common.emDash")}</td>
                  <td>{net.backend ?? "vmnet"}</td>
                  <td>{net.runtime_state ?? t("common.emDash")}</td>
                  <td>{networkAttachmentCount(net, { networks, attachments }) ?? t("common.emDash")}</td>
                  <td>
                    <Button tone="danger" disabled={remove.isPending || networkDeleteId(net, networks) === null} onClick={() => setPendingDelete(net)}>
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
            ? t("networks.deleteMessage", { name: networkLabel(pendingDelete) })
            : ""
        }
        confirmLabel={t("dialog.delete")}
        busy={remove.isPending}
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          const id = pendingDelete && networkDeleteId(pendingDelete, networks);
          if (id) remove.mutate({ id, project: pendingDelete?.project });
        }}
      />
      <FieldError message={remove.isError ? String(remove.error) : null} />
    </section>
  );
}
