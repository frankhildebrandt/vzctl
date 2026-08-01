import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
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
    <div className="page">
      <header className="page-header">
        <div>
          <h1>{t("networks.title")}</h1>
          <p className="muted">
            {t("networks.subtitle")}
            {defaultQuery.data
              ? t("networks.default", {
                  name: defaultQuery.data.name,
                  cidr: defaultQuery.data.cidr ?? "",
                })
              : ""}
          </p>
        </div>
      </header>

      {netsQuery.isError && (
        <p className="error">{String(netsQuery.error)}</p>
      )}
      {netsQuery.isLoading && <p className="muted">{t("networks.loading")}</p>}

      {!netsQuery.isLoading && networks.length === 0 && (
        <p className="muted">{t("networks.empty")}</p>
      )}

      {networks.length > 0 && (
        <table className="data-table">
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
                  <code>{net.name}</code>
                  {defaultQuery.data?.name === net.name ? (
                    <span className="badge">{t("networks.badgeDefault")}</span>
                  ) : null}
                </td>
                <td>{net.cidr ?? t("common.emDash")}</td>
                <td>{net.backend ?? "vmnet"}</td>
                <td>{net.runtime_state ?? t("common.emDash")}</td>
                <td>{attachmentCount.get(net.name) ?? 0}</td>
                <td>
                  <button
                    type="button"
                    className="btn danger"
                    onClick={() => setPendingDelete(net)}
                  >
                    {t("networks.delete")}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
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
      {remove.isError && <p className="error">{String(remove.error)}</p>}
    </div>
  );
}
