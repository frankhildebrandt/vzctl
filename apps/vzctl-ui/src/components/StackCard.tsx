import { Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState, type ReactNode } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  VmnetOrphanDialog,
  type VmnetOrphanChoice,
} from "@/components/VmnetOrphanDialog";
import {
  IconButton,
  IconPlay,
  IconPurge,
  IconStop,
} from "@/components/IconButton";
import { StackVmList } from "@/components/StackVmList";
import {
  Card,
  FieldError,
  Muted,
  PathText,
  SectionTitle,
  StackPhasePill,
} from "@/components/ui";
import { useT } from "@/lib/i18n";
import {
  formatOpenedAt,
  type Project,
  projectKeys,
} from "@/lib/projects";
import {
  deriveStackStatus,
  parseStackInventory,
  type StackPhase,
} from "@/lib/stackStatus";
import { queryKeys, runVzctl } from "@/lib/vzctl";
import {
  parseVmnetOrphanError,
  suggestReplacementCidr,
  type VmnetOrphanInfo,
} from "@/lib/vmnetOrphan";
import {
  recoverOrphanByCidrChange,
  requestHostReboot,
} from "@/lib/vmnetOrphanRecovery";
import { vmKeys } from "@/lib/vms";

type ConfirmKind = "up" | "down" | "purge" | null;

export function StackCard({
  project,
}: {
  project: Project;
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const [confirm, setConfirm] = useState<ConfirmKind>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyMode, setBusyMode] = useState<"up" | "down" | null>(null);
  const [orphan, setOrphan] = useState<VmnetOrphanInfo | null>(null);
  const [orphanBusy, setOrphanBusy] = useState(false);
  const [orphanError, setOrphanError] = useState<string | null>(null);

  const statusQuery = useQuery({
    queryKey: queryKeys.status(project.path),
    queryFn: () => runVzctl(project.path, "status"),
    refetchInterval: 8000,
    retry: false,
  });

  const inventory = useMemo(
    () => parseStackInventory(statusQuery.data ?? null),
    [statusQuery.data],
  );

  const stack = useMemo(
    () =>
      deriveStackStatus({
        inventory,
        applyActive: busyMode != null,
        applyMode: busyMode,
        applyFailed: false,
        t,
      }),
    [inventory, busyMode, t],
  );

  const suggestedCidr = useMemo(() => {
    if (!orphan) return "";
    try {
      return suggestReplacementCidr(orphan.cidr, [orphan.cidr]);
    } catch {
      return "10.78.0.0/24";
    }
  }, [orphan]);

  const mutate = useMutation({
    mutationFn: async (opts: {
      command: "up" | "down";
      force?: boolean;
      purge?: boolean;
    }) => runVzctl(project.path, opts.command, opts),
    onMutate: ({ command }) => {
      setError(null);
      setOrphan(null);
      setOrphanError(null);
      setBusyMode(command);
    },
    onSuccess: async () => {
      setConfirm(null);
      setOrphan(null);
      setBusyMode(null);
      await queryClient.invalidateQueries({
        queryKey: queryKeys.status(project.path),
      });
      await queryClient.invalidateQueries({ queryKey: vmKeys.all });
      await queryClient.invalidateQueries({ queryKey: projectKeys.all });
    },
    onError: (err) => {
      setBusyMode(null);
      const info = parseVmnetOrphanError(err);
      if (info) {
        setConfirm(null);
        setOrphan(info);
        setError(null);
        return;
      }
      setError(String(err));
    },
  });

  const busy = mutate.isPending || orphanBusy;
  const phase: StackPhase = stack.phase;
  const vms = stack.inventory?.vms;
  const items = stack.inventory?.items ?? [];
  const stackId =
    stack.inventory?.stack_id ??
    (typeof stack.inventory?.project === "string"
      ? String(stack.inventory.project)
      : null);

  function confirmCopy(): {
    title: string;
    message: string;
    confirmLabel: string;
    tone: "danger" | "default";
  } {
    switch (confirm) {
      case "up":
        return {
          title: t("stackCard.confirmUpTitle"),
          message: t("stackCard.confirmUpMessage", { name: project.name }),
          confirmLabel: t("stackCard.confirmUpLabel"),
          tone: "default",
        };
      case "down":
        return {
          title: t("stackCard.confirmDownTitle"),
          message: t("stackCard.confirmDownMessage", { name: project.name }),
          confirmLabel: t("stackCard.confirmDownLabel"),
          tone: "danger",
        };
      case "purge":
        return {
          title: t("stackCard.confirmPurgeTitle"),
          message: t("stackCard.confirmPurgeMessage", { name: project.name }),
          confirmLabel: t("stackCard.confirmPurgeLabel"),
          tone: "danger",
        };
      default:
        return {
          title: "",
          message: "",
          confirmLabel: t("dialog.confirmDefault"),
          tone: "default",
        };
    }
  }

  function runConfirmed() {
    if (confirm === "up") mutate.mutate({ command: "up", force: true });
    else if (confirm === "down") mutate.mutate({ command: "down" });
    else if (confirm === "purge")
      mutate.mutate({ command: "down", purge: true });
  }

  async function handleOrphanChoice(choice: VmnetOrphanChoice) {
    if (!orphan) return;
    setOrphanBusy(true);
    setOrphanError(null);
    try {
      if (choice === "reboot") {
        await requestHostReboot();
        setOrphan(null);
        return;
      }
      await recoverOrphanByCidrChange(project.path, orphan, suggestedCidr);
      setOrphan(null);
      mutate.mutate({ command: "up", force: true });
    } catch (err) {
      setOrphanError(String(err));
    } finally {
      setOrphanBusy(false);
    }
  }

  const copy = confirmCopy();

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
      : statusQuery.isError
        ? t("stackCard.statusUnreadable")
        : statusQuery.isFetching
          ? t("stackCard.statusLoading")
          : t("stackCard.noInventory");

  return (
    <>
      <Card as="article" className={`stack-card phase-${phase}`}>
        <div className="stack-card-head">
          <div className="stack-card-title">
            <p className="stack-status-kicker">{t("stackCard.kicker")}</p>
            <Link
              to="/env"
              search={{ path: project.path }}
              className="stack-card-name"
            >
              {project.name}
            </Link>
            {stackId ? <PathText className="stack-card-id">{stackId}</PathText> : null}
            <Muted className="stack-card-meta">
              {formatOpenedAt(project.openedAt)}
            </Muted>
          </div>
          <StackPhasePill phase={phase} loading={statusQuery.isFetching && !inventory}>
            {stack.label}
          </StackPhasePill>
        </div>

        <Muted className="stack-status-summary">{vmSummary}</Muted>

        <StackVmList items={items} stackPath={project.path} />

        <FieldError message={error && confirm == null && orphan == null ? error : null} />

        <div
          className="stack-card-actions"
          role="toolbar"
          aria-label={t("stackCard.toolbarAria")}
        >
          <IconButton
            label={t("stackCard.start")}
            showLabel
            disabled={busy}
            tone="primary"
            onClick={() => {
              setError(null);
              setConfirm("up");
            }}
          >
            <IconPlay />
          </IconButton>
          <IconButton
            label={t("stackCard.stop")}
            showLabel
            disabled={busy}
            tone="danger"
            onClick={() => {
              setError(null);
              setConfirm("down");
            }}
          >
            <IconStop />
          </IconButton>
          <IconButton
            label={t("stackCard.purge")}
            showLabel
            disabled={busy}
            tone="danger"
            onClick={() => {
              setError(null);
              setConfirm("purge");
            }}
          >
            <IconPurge />
          </IconButton>
          <Link
            to="/env"
            search={{ path: project.path }}
            className="stack-card-open"
          >
            {t("stackCard.details")}
          </Link>
        </div>
      </Card>

      <ConfirmDialog
        open={confirm != null}
        title={copy.title}
        message={copy.message}
        confirmLabel={copy.confirmLabel}
        tone={copy.tone}
        busy={mutate.isPending}
        error={confirm != null ? error : null}
        onCancel={() => {
          if (!mutate.isPending) {
            setConfirm(null);
            setError(null);
          }
        }}
        onConfirm={runConfirmed}
      />

      <VmnetOrphanDialog
        open={orphan != null}
        orphanedCidr={orphan?.cidr ?? ""}
        suggestedCidr={suggestedCidr}
        busy={orphanBusy || mutate.isPending}
        error={orphanError}
        onCancel={() => {
          if (!orphanBusy && !mutate.isPending) {
            setOrphan(null);
            setOrphanError(null);
          }
        }}
        onChoose={(choice) => void handleOrphanChoice(choice)}
      />
    </>
  );
}

export function StackCardsSection({
  title,
  projects,
  emptyHint,
}: {
  title?: string;
  projects: Project[];
  emptyHint?: ReactNode;
}) {
  const t = useT();
  const sectionTitle = title ?? t("projects.title");

  if (projects.length === 0) {
    return (
      <Card>
        {sectionTitle ? <h2>{sectionTitle}</h2> : null}
        <Muted>
          {emptyHint ?? (
            <>
              {t("stackCard.emptyTitle")}{" "}
              <Link to="/projects">{t("stackCard.emptyLink")}</Link>
            </>
          )}
        </Muted>
      </Card>
    );
  }

  return (
    <section className="stack-cards-section">
      {sectionTitle ? <SectionTitle>{sectionTitle}</SectionTitle> : null}
      <div className="stack-cards">
        {projects.map((project) => (
          <StackCard key={project.path} project={project} />
        ))}
      </div>
    </section>
  );
}
