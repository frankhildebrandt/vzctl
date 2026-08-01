import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { deleteNet, getDefaultNet, listNets, netKeys, type NetRecord } from "@/lib/nets";

export function NetworksPage() {
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
          <h1>Networks</h1>
          <p className="muted">
            Runtime-Netze aus dem Supervisor
            {defaultQuery.data
              ? ` · Default: ${defaultQuery.data.name} (${defaultQuery.data.cidr})`
              : ""}
          </p>
        </div>
      </header>

      {netsQuery.isError && (
        <p className="error">{String(netsQuery.error)}</p>
      )}
      {netsQuery.isLoading && <p className="muted">Lade Netze…</p>}

      {!netsQuery.isLoading && networks.length === 0 && (
        <p className="muted">Keine Netze registriert.</p>
      )}

      {networks.length > 0 && (
        <table className="data-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>CIDR</th>
              <th>Backend</th>
              <th>State</th>
              <th>Attachments</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {networks.map((net) => (
              <tr key={net.name}>
                <td>
                  <code>{net.name}</code>
                  {defaultQuery.data?.name === net.name ? (
                    <span className="badge">default</span>
                  ) : null}
                </td>
                <td>{net.cidr ?? "—"}</td>
                <td>{net.backend ?? "vmnet"}</td>
                <td>{net.runtime_state ?? "—"}</td>
                <td>{attachmentCount.get(net.name) ?? 0}</td>
                <td>
                  <button
                    type="button"
                    className="btn danger"
                    onClick={() => setPendingDelete(net)}
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <ConfirmDialog
        open={pendingDelete != null}
        title="Netz löschen?"
        message={
          pendingDelete
            ? `Netz „${pendingDelete.name}“ wirklich löschen? Attachments müssen vorher weg.`
            : ""
        }
        confirmLabel="Löschen"
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          if (pendingDelete) remove.mutate(pendingDelete.name);
        }}
      />
      {remove.isError && <p className="error">{String(remove.error)}</p>}
    </div>
  );
}
