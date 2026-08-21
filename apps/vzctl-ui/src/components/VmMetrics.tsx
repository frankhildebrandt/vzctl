import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { ChromeSidebarNotice } from "@/components/Chrome";
import { useT } from "@/lib/i18n";
import { fetchVmStats, vmKeys } from "@/lib/vms";

const HISTORY = 60;

type Series = {
  cpu: number[];
  ram: number[];
  iops: number[];
};

function push(values: number[], next: number): number[] {
  const out = values.length >= HISTORY ? values.slice(1) : values.slice();
  out.push(next);
  return out;
}

function Sparkline({
  label,
  display,
  values,
  title,
}: {
  label: string;
  display: string;
  values: number[];
  title?: string;
}) {
  const width = 72;
  const height = 16;
  const max = Math.max(...values, 1);
  const points =
    values.length < 2
      ? `0,${height} ${width},${height}`
      : values
          .map((value, index) => {
            const x = (index / Math.max(values.length - 1, 1)) * width;
            const y = height - (value / max) * (height - 2) - 1;
            return `${x.toFixed(1)},${y.toFixed(1)}`;
          })
          .join(" ");

  return (
    <div className="vm-metric" title={title ?? `${label} ${display}`}>
      <div className="vm-metric-head">
        <span className="vm-metric-label">{label}</span>
        <span className="vm-metric-value">{display}</span>
      </div>
      <svg
        className="vm-sparkline"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        aria-hidden
      >
        <polyline
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          points={points}
        />
      </svg>
    </div>
  );
}

function formatPercent(value: number | null): string {
  if (value == null) return "—";
  return `${value.toFixed(0)}%`;
}

function formatIops(read: number | null, write: number | null): string {
  if (read == null && write == null) return "—";
  const r = read ?? 0;
  const w = write ?? 0;
  return `${(r + w).toFixed(1)}/s`;
}

/** Compact CPU/RAM/IOPS next to the VM title; old-agent hint goes to the sidebar. */
export function VmMetrics({
  vmId,
  running,
}: {
  vmId: string;
  running: boolean;
}) {
  const t = useT();
  const [series, setSeries] = useState<Series>({ cpu: [], ram: [], iops: [] });

  const statsQuery = useQuery({
    queryKey: vmKeys.stats(vmId),
    queryFn: () => fetchVmStats(vmId),
    enabled: running,
    refetchInterval: running ? 1000 : false,
    retry: false,
  });

  const stats = statsQuery.data;

  useEffect(() => {
    setSeries({ cpu: [], ram: [], iops: [] });
  }, [vmId]);

  useEffect(() => {
    if (!stats) return;
    setSeries((prev) => ({
      cpu: push(prev.cpu, stats.cpu.percent ?? 0),
      ram: push(prev.ram, stats.memory.percent),
      iops: push(
        prev.iops,
        (stats.disk.read_iops ?? 0) + (stats.disk.write_iops ?? 0),
      ),
    }));
  }, [stats]);

  const display = useMemo(() => {
    if (!stats) {
      return {
        cpu: "—",
        ram: "—",
        iops: "—",
        ramTitle: undefined as string | undefined,
      };
    }
    return {
      cpu: formatPercent(stats.cpu.percent),
      ram: formatPercent(stats.memory.percent),
      iops: formatIops(stats.disk.read_iops, stats.disk.write_iops),
      ramTitle: `${t("vmDetail.metricsRam")} ${stats.memory.used_mib} / ${stats.memory.total_mib} MiB`,
    };
  }, [stats, t]);

  const showAgentHint = running && statsQuery.isError && !stats;

  return (
    <>
      {showAgentHint ? (
        <ChromeSidebarNotice>
          <p className="sidebar-hint">{t("vmDetail.agentUpgradeHint")}</p>
        </ChromeSidebarNotice>
      ) : null}
      <div className="vm-metrics">
        <Sparkline
          label={t("vmDetail.metricsCpu")}
          display={display.cpu}
          values={series.cpu}
        />
        <Sparkline
          label={t("vmDetail.metricsRam")}
          display={display.ram}
          values={series.ram}
          title={display.ramTitle}
        />
        <Sparkline
          label={t("vmDetail.metricsIops")}
          display={display.iops}
          values={series.iops}
        />
      </div>
    </>
  );
}
