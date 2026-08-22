import { Button } from "@/components/ui";
import { useT } from "@/lib/i18n";
import type { SystemdUnitType } from "@/lib/systemd";

export type SystemdStateFilter = "all" | "running" | "inactive";

type Props = {
  unitType: SystemdUnitType;
  onUnitTypeChange: (type: SystemdUnitType) => void;
  counts: Record<SystemdUnitType, number | undefined>;
  query: string;
  onQueryChange: (value: string) => void;
  stateFilter: SystemdStateFilter;
  onStateFilterChange: (value: SystemdStateFilter) => void;
};

const UNIT_TYPES: SystemdUnitType[] = ["service", "timer", "socket"];

export function VmServicesToolbar({
  unitType,
  onUnitTypeChange,
  counts,
  query,
  onQueryChange,
  stateFilter,
  onStateFilterChange,
}: Props) {
  const t = useT();

  const typeLabel: Record<SystemdUnitType, string> = {
    service: t("systemd.tab.services"),
    timer: t("systemd.tab.timers"),
    socket: t("systemd.tab.sockets"),
  };

  return (
    <div className="systemd-toolbar">
      <div className="segmented-control" role="tablist" aria-label={t("systemd.title")}>
        {UNIT_TYPES.map((type) => {
          const count = counts[type];
          const label =
            count == null ? typeLabel[type] : `${typeLabel[type]} (${count})`;
          return (
            <button
              key={type}
              type="button"
              role="tab"
              aria-selected={unitType === type}
              className={unitType === type ? "is-active" : undefined}
              onClick={() => onUnitTypeChange(type)}
            >
              {label}
            </button>
          );
        })}
      </div>

      <div className="systemd-toolbar-filters">
        <input
          type="search"
          className="systemd-search"
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder={t("systemd.searchPlaceholder")}
          aria-label={t("systemd.searchPlaceholder")}
        />
        <div className="segmented-control segmented-control-compact">
          {(
            [
              ["all", t("systemd.filterAll")],
              ["running", t("systemd.filterRunning")],
              ["inactive", t("systemd.filterInactive")],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              className={stateFilter === value ? "is-active" : undefined}
              onClick={() => onStateFilterChange(value)}
            >
              {label}
            </button>
          ))}
        </div>
        {query || stateFilter !== "all" ? (
          <Button
            tone="quiet"
            className="systemd-clear"
            onClick={() => {
              onQueryChange("");
              onStateFilterChange("all");
            }}
          >
            {t("systemd.clearFilters")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
