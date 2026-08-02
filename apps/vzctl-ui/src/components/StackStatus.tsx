import { StackVmList } from "@/components/StackVmList";
import { Card, Muted, PathText, StackPhasePill } from "@/components/ui";
import { useT } from "@/lib/i18n";
import { formatOpenedAt } from "@/lib/projects";
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
  openedAt?: number | null;
  phase: StackPhase;
  label: string;
  inventory: StackInventory | null;
  loading?: boolean;
}) {
  const t = useT();
  const vms = inventory?.vms;
  const items = inventory?.items ?? [];

  const vmSummary =
    vms != null
      ? t("stackCard.summary", {
          running: vms.running,
          desired: vms.desired,
          starting:
            vms.starting > 0
              ? t("stackCard.summaryStarting", { n: vms.starting })
              : "",
          stopping:
            vms.stopping > 0
              ? t("stackCard.summaryStopping", { n: vms.stopping })
              : "",
          stopped:
            vms.stopped > 0
              ? t("stackCard.summaryStopped", { n: vms.stopped })
              : "",
          missing:
            vms.missing > 0
              ? t("stackCard.summaryMissing", { n: vms.missing })
              : "",
        })
      : loading
        ? t("stackCard.statusLoading")
        : t("stackCard.noInventory");

  return (
    <Card className={`stack-status phase-${phase}`}>
      <div className="stack-status-head">
        <div className="stack-status-identity">
          <h2 className="stack-status-title">{title}</h2>
          <PathText className="stack-status-path">{path}</PathText>
          {openedAt != null ? (
            <Muted className="stack-status-meta">
              {t("stack.lastOpened", { date: formatOpenedAt(openedAt) })}
            </Muted>
          ) : null}
        </div>
        <StackPhasePill phase={phase} loading={loading}>
          {label}
        </StackPhasePill>
      </div>

      <Muted className="stack-status-summary">{vmSummary}</Muted>

      <StackVmList items={items} stackPath={path} />
    </Card>
  );
}
