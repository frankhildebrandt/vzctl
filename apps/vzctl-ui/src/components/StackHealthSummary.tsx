import { Badge, Muted } from "@/components/ui";
import { useT } from "@/lib/i18n";
import { HEALTH_LABEL_KEYS, type StackHealth } from "@/lib/stackHealth";

export function StackHealthSummary({ health }: { health: StackHealth }) {
  const t = useT();
  return <div className="stack-health-summary" role="status">
    <Badge tone={health.state === "stopped" ? "neutral" : health.state === "ok" ? "ok" : health.state === "critical" ? "danger" : "warn"}>
      {t(HEALTH_LABEL_KEYS[health.state])}
    </Badge>
    {health.stoppedVms ? <Muted>{t("stack.health.skippedVms", { count: health.stoppedVms })}</Muted> : null}
    {health.error ? <p>{health.error}</p> : null}
    {health.state === "unknown" && !health.error ? <Muted>{t("stack.health.unavailable")}</Muted> : null}
    {health.warnings.length > 0 ? <ul>
      {health.warnings.map((warning, index) => <li key={index}>
        {warning.vmId ? <strong>{warning.vmId}: </strong> : null}{warning.message}
        {warning.hint ? <Muted> {warning.hint}</Muted> : null}
      </li>)}
    </ul> : null}
  </div>;
}
