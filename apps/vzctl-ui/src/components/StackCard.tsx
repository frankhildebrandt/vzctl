import { Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState, type ReactNode } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  IconButton,
  IconPlay,
  IconPurge,
  IconStop,
} from "@/components/IconButton";
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
import { vmKeys } from "@/lib/vms";

type ConfirmKind = "up" | "down" | "purge" | null;

export function StackCard({
  project,
}: {
  project: Project;
}) {
  const queryClient = useQueryClient();
  const [confirm, setConfirm] = useState<ConfirmKind>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyMode, setBusyMode] = useState<"up" | "down" | null>(null);

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
      }),
    [inventory, busyMode],
  );

  const mutate = useMutation({
    mutationFn: async (opts: {
      command: "up" | "down";
      force?: boolean;
      purge?: boolean;
    }) => runVzctl(project.path, opts.command, opts),
    onMutate: ({ command }) => {
      setError(null);
      setBusyMode(command);
    },
    onSuccess: async () => {
      setConfirm(null);
      setBusyMode(null);
      await queryClient.invalidateQueries({
        queryKey: queryKeys.status(project.path),
      });
      await queryClient.invalidateQueries({ queryKey: vmKeys.all });
      await queryClient.invalidateQueries({ queryKey: projectKeys.all });
    },
    onError: (err) => {
      setBusyMode(null);
      setError(String(err));
    },
  });

  const busy = mutate.isPending;
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
          title: "Stack starten",
          message: `„${project.name}“ starten (up --force)?`,
          confirmLabel: "Starten",
          tone: "default",
        };
      case "down":
        return {
          title: "Stack stoppen",
          message: `„${project.name}“ stoppen? Ressourcen bleiben erhalten.`,
          confirmLabel: "Stoppen",
          tone: "danger",
        };
      case "purge":
        return {
          title: "Stack löschen",
          message: `„${project.name}“ stoppen und löschen (VMs, Netze, Ports, Ingress, OIDC, DNS)? Das Verzeichnis bleibt erhalten.`,
          confirmLabel: "Löschen",
          tone: "danger",
        };
      default:
        return {
          title: "",
          message: "",
          confirmLabel: "OK",
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

  const copy = confirmCopy();

  return (
    <>
      <article className={`card stack-card phase-${phase}`}>
        <div className="stack-card-head">
          <div className="stack-card-title">
            <p className="stack-status-kicker">Stack</p>
            <Link
              to="/env"
              search={{ path: project.path }}
              className="stack-card-name"
            >
              {project.name}
            </Link>
            {stackId ? <p className="path stack-card-id">{stackId}</p> : null}
            <p className="muted stack-card-meta">
              {formatOpenedAt(project.openedAt)}
            </p>
          </div>
          <span className={`stack-pill phase-${phase}`}>
            {statusQuery.isFetching && !inventory ? "…" : stack.label}
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
            {statusQuery.isError
              ? "Status nicht lesbar"
              : statusQuery.isFetching
                ? "Status wird geladen…"
                : "Kein Stack-Inventar"}
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

        {error && confirm == null ? <p className="form-error">{error}</p> : null}

        <div
          className="stack-card-actions"
          role="toolbar"
          aria-label="Stack-Aktionen"
        >
          <IconButton
            label="Starten"
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
            label="Stoppen"
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
            label="Löschen"
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
            Details →
          </Link>
        </div>
      </article>

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
    </>
  );
}

export function StackCardsSection({
  title = "Stacks",
  projects,
  emptyHint,
}: {
  title?: string;
  projects: Project[];
  emptyHint?: ReactNode;
}) {
  if (projects.length === 0) {
    return (
      <div className="card">
        {title ? <h2>{title}</h2> : null}
        <p className="muted">
          {emptyHint ?? (
            <>
              Noch keine Stacks.{" "}
              <Link to="/projects">Stack hinzufügen</Link>
            </>
          )}
        </p>
      </div>
    );
  }

  return (
    <section className="stack-cards-section">
      {title ? <h2 className="section-title">{title}</h2> : null}
      <div className="stack-cards">
        {projects.map((project) => (
          <StackCard key={project.path} project={project} />
        ))}
      </div>
    </section>
  );
}
