import type { SystemdUnit } from "@/lib/systemd";
import { unitStatusLabel, unitVisualState } from "@/lib/systemd";

export function UnitStatusBadge({ unit }: { unit: SystemdUnit }) {
  const state = unitVisualState(unit);
  return (
    <span className={`systemd-status systemd-status-${state}`}>
      {unitStatusLabel(unit)}
    </span>
  );
}
