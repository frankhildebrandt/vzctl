import { useMutation, useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { VmLogsDetailDialog } from "@/components/vm-logs/VmLogsDetailDialog";
import { VmLogsHelpDialog } from "@/components/vm-logs/VmLogsHelpDialog";
import { VmLogsShareDialog } from "@/components/vm-logs/VmLogsShareDialog";
import { VmLogsStream } from "@/components/vm-logs/VmLogsStream";
import { bumpMinLevel, VmLogsToolbar } from "@/components/vm-logs/VmLogsToolbar";
import { Alert, EmptyState, PageHeader } from "@/components/ui";
import { useGuestLogStream } from "@/hooks/useGuestLogStream";
import {
  fetchGuestLogStatus,
  listGuestServices,
  postGuestLogAction,
  type LogsQuery,
} from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import { useT } from "@/lib/i18n";
import { encodeVmIdParam } from "@/lib/vms";

type Props = {
  vmId: string;
  source?: string;
};

type PendingConfirm = "restart" | "truncate" | null;

export function VmLogsPage({ vmId, source: sourceParam }: Props) {
  const t = useT();
  const navigate = useNavigate();
  const queryInputRef = useRef<HTMLInputElement>(null);
  const lastEnterRef = useRef(0);

  const [q, setQ] = useState("");
  const [minLevel, setMinLevel] = useState("all");
  const [groupField, setGroupField] = useState("component");
  const [groupValue, setGroupValue] = useState("");
  const [fieldFilters, setFieldFilters] = useState<Record<string, string>>({});
  const [hiddenFields, setHiddenFields] = useState<HiddenFields>({ raw: true });
  const [showFieldVisibility, setShowFieldVisibility] = useState(false);
  const [source, setSource] = useState(sourceParam ?? "");
  const [pendingConfirm, setPendingConfirm] = useState<PendingConfirm>(null);
  const [detailIndex, setDetailIndex] = useState(-1);
  const [shareIndex, setShareIndex] = useState(-1);
  const [shareContext, setShareContext] = useState<number | undefined>();
  const [helpOpen, setHelpOpen] = useState(false);
  const [busyAction, setBusyAction] = useState<string | null>(null);

  const sourcesQuery = useQuery({
    queryKey: ["guest-services", vmId],
    queryFn: () => listGuestServices(vmId),
  });

  const sources = sourcesQuery.data ?? [];
  const selected =
    (source && sources.some((item) => item.name === source)
      ? source
      : sources[0]?.name) ?? "";

  const filters = useMemo(
    () => ({
      q,
      minLevel,
      groupField,
      groupValue,
      fieldFilters,
    }),
    [q, minLevel, groupField, groupValue, fieldFilters],
  );

  const logsQuery = useMemo(
    (): LogsQuery => ({
      q: q || undefined,
      minLevel: minLevel || undefined,
      groupField: groupField || undefined,
      groupValue: groupValue || undefined,
      filters: fieldFilters,
    }),
    [q, minLevel, groupField, groupValue, fieldFilters],
  );

  const statusQuery = useQuery({
    queryKey: ["guest-log-status", vmId, selected],
    queryFn: () => fetchGuestLogStatus(vmId, selected),
    enabled: Boolean(selected),
  });

  const stream = useGuestLogStream({
    vmId,
    source: selected,
    filters,
    enabled: Boolean(selected),
  });

  const observedFields =
    stream.observedFields.length > 0
      ? stream.observedFields
      : (statusQuery.data?.observedFields ?? []);

  const groupValues =
    stream.groupValues.length > 0
      ? stream.groupValues
      : (statusQuery.data?.groupValues ?? []);

  const processStatus = {
    ...statusQuery.data,
    ...stream.processStatus,
  };

  const actionMutation = useMutation({
    mutationFn: async (action: "/api/restart" | "/api/truncate" | "/api/separator" | "/api/open-url") => {
      setBusyAction(action.replace("/api/", ""));
      await postGuestLogAction(vmId, selected, action);
    },
    onSettled: () => {
      setBusyAction(null);
      if (pendingConfirm) setPendingConfirm(null);
    },
    onSuccess: (_data, action) => {
      if (action === "/api/restart" || action === "/api/truncate") {
        stream.connectNow();
      }
    },
  });

  const handleSourceChange = useCallback(
    (next: string) => {
      setSource(next);
      void navigate({
        to: "/vms/$vmId/logs",
        params: { vmId: encodeVmIdParam(vmId) },
        search: (prev) => ({ ...prev, source: next }),
      });
    },
    [navigate, vmId],
  );

  const jumpLine = useCallback(
    (delta: number) => {
      if (stream.lines.length === 0) return;
      let index = stream.lines.findIndex((line) => line.index === stream.selectedIndex);
      if (index < 0) index = delta > 0 ? -1 : stream.lines.length;
      index = Math.max(0, Math.min(stream.lines.length - 1, index + delta));
      const line = stream.lines[index];
      if (line.index == null) return;
      stream.selectLine(line.index);
      const node = stream.listRef.current?.querySelector(
        `[data-index="${line.index}"]`,
      );
      node?.scrollIntoView({ block: "nearest" });
    },
    [stream],
  );

  const openShare = useCallback(
    (context?: number) => {
      if (stream.selectedIndex < 0) return;
      setShareContext(context);
      setShareIndex(stream.selectedIndex);
    },
    [stream.selectedIndex],
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      const typing =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement;

      if (event.key === "Escape") {
        setDetailIndex(-1);
        setShareIndex(-1);
        setHelpOpen(false);
        queryInputRef.current?.blur();
        return;
      }
      if (event.key === "/" && !typing) {
        event.preventDefault();
        queryInputRef.current?.focus();
        return;
      }
      if (typing) return;

      if (event.key === "?") setHelpOpen((open) => !open);
      if (event.key === "r") setPendingConfirm("restart");
      if (event.key === "t") setPendingConfirm("truncate");
      if (event.key === "O") void actionMutation.mutate("/api/open-url");
      if (event.key === "+") {
        setMinLevel((current) => bumpMinLevel(current, 1));
        stream.connectNow();
      }
      if (event.key === "-") {
        setMinLevel((current) => bumpMinLevel(current, -1));
        stream.connectNow();
      }
      if (event.key === "n") jumpLine(1);
      if (event.key === "N") jumpLine(-1);
      if (event.key === "y") openShare(0);
      if (event.key === "Y") openShare(20);
      if (event.key === "Enter") {
        const now = Date.now();
        if (now - lastEnterRef.current < 600) {
          void actionMutation.mutate("/api/separator");
        }
        lastEnterRef.current = now;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [actionMutation, jumpLine, openShare, stream]);

  return (
    <section className="vm-logs-page">
      <PageHeader layout="detail" title={t("vmLogs.title")} />
      {sourcesQuery.isError ? (
        <Alert title={t("common.error")}>{String(sourcesQuery.error)}</Alert>
      ) : null}
      {sources.length === 0 && !sourcesQuery.isLoading ? (
        <EmptyState
          title={t("vmLogs.emptyTitle")}
          message={t("vmLogs.emptyHint")}
        />
      ) : (
        <div className="vm-logs-panel">
          <VmLogsToolbar
            sources={sources}
            selectedSource={selected}
            onSourceChange={handleSourceChange}
            q={q}
            onQChange={setQ}
            onQCommit={stream.connectNow}
            queryInputRef={queryInputRef}
            minLevel={minLevel}
            onMinLevelChange={setMinLevel}
            onBumpLevel={(delta) => setMinLevel((current) => bumpMinLevel(current, delta))}
            groupField={groupField}
            onGroupFieldChange={(value) => {
              setGroupField(value);
              setGroupValue("");
            }}
            groupValue={groupValue}
            onGroupValueChange={setGroupValue}
            fieldFilters={fieldFilters}
            onFieldFilterChange={(field, value) =>
              setFieldFilters((current) => ({ ...current, [field]: value }))
            }
            onFieldFilterRename={(oldField, nextField) =>
              setFieldFilters((current) => {
                const next = { ...current };
                const value = next[oldField];
                delete next[oldField];
                if (nextField) next[nextField] = value;
                return next;
              })
            }
            onFieldFilterRemove={(field) =>
              setFieldFilters((current) => {
                const next = { ...current };
                delete next[field];
                return next;
              })
            }
            onAddFieldFilter={() =>
              setFieldFilters((current) => ({ ...current, msg: current.msg ?? "" }))
            }
            observedFields={observedFields}
            groupValues={groupValues}
            processStatus={processStatus}
            pendingLive={stream.pendingLive}
            autoScroll={stream.autoScroll}
            onAutoScrollChange={(value) => {
              stream.setAutoScroll(value);
              if (value) stream.connectNow();
            }}
            hiddenFields={hiddenFields}
            onHiddenFieldToggle={(field, visible) =>
              setHiddenFields((current) => {
                const next = { ...current };
                if (visible) delete next[field];
                else next[field] = true;
                return next;
              })
            }
            showFieldVisibility={showFieldVisibility}
            onToggleFieldVisibility={() => setShowFieldVisibility((open) => !open)}
            onRestart={() => setPendingConfirm("restart")}
            onTruncate={() => setPendingConfirm("truncate")}
            onSeparator={() => void actionMutation.mutate("/api/separator")}
            onOpenUrl={() => void actionMutation.mutate("/api/open-url")}
            onHelp={() => setHelpOpen(true)}
            busyAction={busyAction}
          />
          {stream.streamError ? (
            <Alert title={t("common.error")}>{stream.streamError}</Alert>
          ) : null}
          <VmLogsStream
            lines={stream.lines}
            observedFields={observedFields}
            hiddenFields={hiddenFields}
            selectedIndex={stream.selectedIndex}
            listRef={stream.listRef}
            onScroll={stream.onScroll}
            onSelectLine={stream.selectLine}
            onOpenDetail={setDetailIndex}
          />
        </div>
      )}

      <ConfirmDialog
        open={pendingConfirm === "restart"}
        title={t("vmLogs.restartTitle")}
        message={t("vmLogs.restartConfirm", { source: selected })}
        confirmLabel={t("vmLogs.restart")}
        tone="default"
        busy={actionMutation.isPending}
        error={actionMutation.error ? String(actionMutation.error) : null}
        onConfirm={() => actionMutation.mutate("/api/restart")}
        onCancel={() => {
          if (!actionMutation.isPending) setPendingConfirm(null);
        }}
      />
      <ConfirmDialog
        open={pendingConfirm === "truncate"}
        title={t("vmLogs.truncateTitle")}
        message={t("vmLogs.truncateConfirm")}
        confirmLabel={t("vmLogs.truncate")}
        tone="default"
        busy={actionMutation.isPending}
        error={actionMutation.error ? String(actionMutation.error) : null}
        onConfirm={() => actionMutation.mutate("/api/truncate")}
        onCancel={() => {
          if (!actionMutation.isPending) setPendingConfirm(null);
        }}
      />

      <VmLogsDetailDialog
        open={detailIndex >= 0}
        vmId={vmId}
        source={selected}
        index={detailIndex}
        query={logsQuery}
        onClose={() => setDetailIndex(-1)}
      />
      <VmLogsShareDialog
        open={shareIndex >= 0}
        vmId={vmId}
        source={selected}
        index={shareIndex}
        query={logsQuery}
        context={shareContext}
        onClose={() => {
          setShareIndex(-1);
          setShareContext(undefined);
        }}
      />
      <VmLogsHelpDialog open={helpOpen} onClose={() => setHelpOpen(false)} />
    </section>
  );
}
