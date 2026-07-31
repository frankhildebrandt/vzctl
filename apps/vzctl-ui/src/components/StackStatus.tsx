import type { StackInventory, StackPhase } from "@/lib/stackStatus";

export function StackStatusCard({
  phase,
  label,
  inventory,
  loading,
}: {
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
        <div>
          <p className="stack-status-kicker">Stack</p>
          <h2 className="stack-status-label">{loading ? "…" : label}</h2>
        </div>
        <span className={`stack-pill phase-${phase}`}>{loading ? "…" : label}</span>
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

      {items.length > 0 ? (
        <ul className="stack-vm-list">
          {items.map((item) => (
            <li key={item.id} className={`stack-vm state-${item.state}`}>
              <span className="stack-vm-id">{item.id}</span>
              <span className="stack-vm-state">{item.state}</span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
