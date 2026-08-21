import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Alert, Button, Card, EmptyState, FormField, PageHeader } from "@/components/ui";
import { isDemoMode } from "@/lib/demo";
import {
  fetchGuestLogStatus,
  fetchGuestLogs,
  guestServiceApiPath,
  listGuestServices,
  restartGuestProcess,
  type IwatchLine,
} from "@/lib/guestLogs";
import { useT } from "@/lib/i18n";

const WINDOW = 400;
const DEBOUNCE_MS = 500;

type Props = {
  vmId: string;
  source?: string;
};

export function VmLogsPage({ vmId, source: sourceParam }: Props) {
  const t = useT();
  const [q, setQ] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  const [minLevel, setMinLevel] = useState("");
  const [groupField, setGroupField] = useState("");
  const [groupValue, setGroupValue] = useState("");
  const [fieldFilters, setFieldFilters] = useState<Record<string, string>>({});
  const [lines, setLines] = useState<IwatchLine[]>([]);
  const [source, setSource] = useState(sourceParam ?? "");
  const [streamError, setStreamError] = useState<string | null>(null);
  const [confirmRestart, setConfirmRestart] = useState(false);

  useEffect(() => {
    const handle = window.setTimeout(() => setDebouncedQ(q), DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [q]);

  const sourcesQuery = useQuery({
    queryKey: ["guest-services", vmId],
    queryFn: () => listGuestServices(vmId),
  });

  const sources = sourcesQuery.data ?? [];
  const selected =
    (source && sources.some((item) => item.name === source)
      ? source
      : sources[0]?.name) ?? "";

  const query = useMemo(
    () => ({
      q: debouncedQ || undefined,
      minLevel: minLevel || undefined,
      groupField: groupField || undefined,
      groupValue: groupValue || undefined,
      filters: fieldFilters,
      limit: WINDOW,
      tail: WINDOW,
    }),
    [debouncedQ, minLevel, groupField, groupValue, fieldFilters],
  );

  const statusQuery = useQuery({
    queryKey: ["guest-log-status", vmId, selected],
    queryFn: () => fetchGuestLogStatus(vmId, selected),
    enabled: Boolean(selected),
  });

  useEffect(() => {
    if (!selected) {
      setLines([]);
      return;
    }
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    setStreamError(null);
    const channel = `vzctl-guest-log-${vmId}-${selected}`;

    void (async () => {
      try {
        const snapshot = await fetchGuestLogs(vmId, selected, query);
        if (!cancelled) setLines(snapshot.slice(-WINDOW));
      } catch (err) {
        if (!cancelled) setStreamError(String(err));
      }
      if (cancelled || isDemoMode()) return;
      try {
        const path = guestServiceApiPath(vmId, selected, "/api/logs/sse", query);
        await invoke("subscribe_guest_logs", {
          pathAndQuery: path,
          channel,
        });
        unlisten = await listen<{ event?: string; data?: IwatchLine }>(
          channel,
          (event) => {
            if (event.payload.event && event.payload.event !== "line") return;
            const line = event.payload.data;
            if (!line?.text) return;
            setLines((current) => [...current, line].slice(-WINDOW));
          },
        );
      } catch (err) {
        if (!cancelled) setStreamError(String(err));
      }
    })();

    return () => {
      cancelled = true;
      void unlisten?.();
    };
  }, [vmId, selected, query]);

  const observed = statusQuery.data?.observedFields ?? [];
  const groupValues = statusQuery.data?.groupValues ?? [];

  const restartMutation = useMutation({
    mutationFn: () => restartGuestProcess(vmId, selected),
    onSuccess: () => {
      setConfirmRestart(false);
      void statusQuery.refetch();
    },
  });

  return (
    <section>
      <PageHeader
        layout="detail"
        title={t("vmLogs.title")}
        subtitle={t("vmLogs.subtitle", { vm: vmId })}
        actions={
          selected ? (
            <Button
              tone="secondary"
              disabled={restartMutation.isPending}
              onClick={() => {
                restartMutation.reset();
                setConfirmRestart(true);
              }}
            >
              {restartMutation.isPending
                ? t("vmLogs.restartBusy")
                : t("vmLogs.restart")}
            </Button>
          ) : null
        }
      />
      {sourcesQuery.isError ? (
        <Alert title={t("common.error")}>{String(sourcesQuery.error)}</Alert>
      ) : null}
      {sources.length === 0 && !sourcesQuery.isLoading ? (
        <EmptyState
          title={t("vmLogs.emptyTitle")}
          message={t("vmLogs.emptyHint")}
        />
      ) : (
        <Card>
          <div className="vm-logs-filters">
            {sources.length > 1 ? (
              <FormField label={t("vmLogs.source")}>
                <select
                  value={selected}
                  onChange={(event) => setSource(event.target.value)}
                  aria-label={t("vmLogs.source")}
                >
                  {sources.map((item) => (
                    <option key={item.name} value={item.name}>
                      {item.name}
                    </option>
                  ))}
                </select>
              </FormField>
            ) : null}
            <FormField label={t("vmLogs.query")}>
              <input
                value={q}
                onChange={(event) => setQ(event.target.value)}
                placeholder={t("vmLogs.queryPlaceholder")}
                aria-label={t("vmLogs.query")}
              />
            </FormField>
            <FormField label={t("vmLogs.minLevel")}>
              <select
                value={minLevel}
                onChange={(event) => setMinLevel(event.target.value)}
                aria-label={t("vmLogs.minLevel")}
              >
                <option value="">{t("vmLogs.anyLevel")}</option>
                <option value="debug">debug</option>
                <option value="info">info</option>
                <option value="warn">warn</option>
                <option value="error">error</option>
              </select>
            </FormField>
            <FormField label={t("vmLogs.groupField")}>
              <input
                value={groupField}
                onChange={(event) => setGroupField(event.target.value)}
                placeholder={statusQuery.data?.groupField ?? "component"}
                aria-label={t("vmLogs.groupField")}
              />
            </FormField>
            <FormField label={t("vmLogs.groupValue")}>
              <select
                value={groupValue}
                onChange={(event) => setGroupValue(event.target.value)}
                aria-label={t("vmLogs.groupValue")}
              >
                <option value="">{t("vmLogs.anyGroup")}</option>
                {groupValues.map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            </FormField>
          </div>
          {observed.length > 0 ? (
            <div className="vm-logs-filters">
              {observed.slice(0, 6).map((field) => (
                <FormField key={field} label={field}>
                  <input
                    value={fieldFilters[field] ?? ""}
                    onChange={(event) =>
                      setFieldFilters((current) => ({
                        ...current,
                        [field]: event.target.value,
                      }))
                    }
                    aria-label={field}
                  />
                </FormField>
              ))}
            </div>
          ) : null}
          {streamError ? <Alert title={t("common.error")}>{streamError}</Alert> : null}
          <pre className="console-log vm-logs-stream" aria-label={t("vmLogs.stream")}>
            {lines.length === 0
              ? t("vmLogs.waiting")
              : lines
                  .map((line) => {
                    const prefix = [line.source, line.level].filter(Boolean).join(" ");
                    return prefix ? `${prefix} ${line.text}` : line.text;
                  })
                  .join("\n")}
          </pre>
        </Card>
      )}
      <ConfirmDialog
        open={confirmRestart}
        title={t("vmLogs.restartTitle")}
        message={t("vmLogs.restartConfirm", { source: selected })}
        confirmLabel={t("vmLogs.restart")}
        tone="default"
        busy={restartMutation.isPending}
        error={restartMutation.error ? String(restartMutation.error) : null}
        onConfirm={() => restartMutation.mutate()}
        onCancel={() => {
          if (!restartMutation.isPending) setConfirmRestart(false);
        }}
      />
    </section>
  );
}
