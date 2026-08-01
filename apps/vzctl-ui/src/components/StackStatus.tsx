import { StackVmList } from "@/components/StackVmList";
import type { StackInventory, StackPhase } from "@/lib/stackStatus";

export function StackStatusCard({
  title,
  path,
  openedAt,
  phase,
  label,
  inventory,
  loading,
}: {
  title: string;
  path: string;
  openedAt?: string | null;
  phase: StackPhase;
  label: string;
  inventory: StackInventory | null;
  loading?: boolean;
}) {
  const vms = inventory?.vms;
  const items = inventory?.items ?? [];

  return (
    <div className={`card stack-status phase-${phase}`}>
      <div className="stack-status-head">
        <div className="stack-status-identity">
          <h2 className="stack-status-title">{title}</h2>
          <p className="path stack-status-path">{path}</p>
          {openedAt ? (
            <p className="muted stack-status-meta">Zuletzt geöffnet: {openedAt}</p>
          ) : null}
        </div>
        <span className={`stack-pill phase-${phase}`}>
          {loading ? "…" : label}
        </span>
      </div>

      {vms ? (
        <p className="muted stack-status-summary">
          {vms.running}/{vms.desired} running
          {vms.starting > 0 ? ` · ${vms.starting} starting` : ""}
          {vms.stopping > 0 ? ` · ${vms.stopping} stopping` : ""}
          {vms.stopped > 0 ? ` · ${vms.stopped} stopped` : ""}
          {vms.missing > 0 ? ` · ${vms.missing} missing` : ""}
        </p>
      ) : (
        <p className="muted stack-status-summary">
          {loading ? "Status wird geladen…" : "Kein Stack-Inventar"}
        </p>
      )}

      <StackVmList items={items} stackPath={path} />
    </div>
  );
}
