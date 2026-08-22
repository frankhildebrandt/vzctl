import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isDemoMode } from "@/lib/demo";
import {
  fetchGuestLogs,
  guestServiceApiPath,
  type IwatchLine,
  type IwatchStatus,
  type LogsQuery,
} from "@/lib/guestLogs";

export const MAX_WINDOW = 400;
export const PAGE_SIZE = 100;
export const FILTER_DEBOUNCE_MS = 500;

type SsePayload = {
  event?: string;
  data?: IwatchLine | IwatchStatus | string[];
};

export type GuestLogFilters = {
  q: string;
  minLevel: string;
  groupField: string;
  groupValue: string;
  fieldFilters: Record<string, string>;
};

type UseGuestLogStreamInput = {
  vmId: string;
  source: string;
  filters: GuestLogFilters;
  enabled?: boolean;
};

type UseGuestLogStreamResult = {
  lines: IwatchLine[];
  selectedIndex: number;
  autoScroll: boolean;
  pendingLive: number;
  processStatus: IwatchStatus;
  observedFields: string[];
  groupValues: string[];
  streamError: string | null;
  loadingOlder: boolean;
  setAutoScroll: (value: boolean) => void;
  selectLine: (index: number) => void;
  loadOlder: () => Promise<void>;
  connectNow: () => void;
  listRef: React.RefObject<HTMLUListElement | null>;
  onScroll: () => void;
  trimBack: () => void;
};

function buildQuery(
  filters: GuestLogFilters,
  extra?: Partial<LogsQuery>,
): LogsQuery {
  return {
    q: filters.q || undefined,
    minLevel: filters.minLevel || undefined,
    groupField: filters.groupField || undefined,
    groupValue: filters.groupValue || undefined,
    filters: filters.fieldFilters,
    tail: MAX_WINDOW,
    ...extra,
  };
}

/** Manage iwatch snapshot + SSE stream with follow/scroll parity to the original web UI. */
export function useGuestLogStream({
  vmId,
  source,
  filters,
  enabled = true,
}: UseGuestLogStreamInput): UseGuestLogStreamResult {
  const [lines, setLines] = useState<IwatchLine[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [autoScroll, setAutoScrollState] = useState(true);
  const [pendingLive, setPendingLive] = useState(0);
  const [processStatus, setProcessStatus] = useState<IwatchStatus>({});
  const [observedFields, setObservedFields] = useState<string[]>([]);
  const [groupValues, setGroupValues] = useState<string[]>([]);
  const [streamError, setStreamError] = useState<string | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [connectTick, setConnectTick] = useState(0);

  const listRef = useRef<HTMLUListElement>(null);
  const autoScrollRef = useRef(autoScroll);
  const pendingLiveRef = useRef(pendingLive);
  const loadingOlderRef = useRef(false);
  const debouncedFiltersRef = useRef(filters);

  const [debouncedFilters, setDebouncedFilters] = useState(filters);

  useEffect(() => {
    autoScrollRef.current = autoScroll;
  }, [autoScroll]);

  useEffect(() => {
    pendingLiveRef.current = pendingLive;
  }, [pendingLive]);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      debouncedFiltersRef.current = filters;
      setDebouncedFilters(filters);
    }, FILTER_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [filters]);

  const query = useMemo(
    () => buildQuery(debouncedFilters),
    [debouncedFilters],
  );

  const applyStatus = useCallback((status: IwatchStatus) => {
    setProcessStatus(status);
    if (status.groupValues) setGroupValues(status.groupValues);
    if (status.observedFields) setObservedFields(status.observedFields);
  }, []);

  const trimFront = useCallback((current: IwatchLine[]) => {
    if (current.length <= MAX_WINDOW) return current;
    return current.slice(current.length - MAX_WINDOW);
  }, []);

  const trimBack = useCallback(() => {
    setLines((current) => {
      if (current.length <= MAX_WINDOW) return current;
      return current.slice(0, MAX_WINDOW);
    });
  }, []);

  const setAutoScroll = useCallback((value: boolean) => {
    setAutoScrollState(value);
    autoScrollRef.current = value;
    if (value) setPendingLive(0);
  }, []);

  const selectLine = useCallback((index: number) => {
    setSelectedIndex(index);
  }, []);

  const connectNow = useCallback(() => {
    setConnectTick((tick) => tick + 1);
  }, []);

  const loadOlder = useCallback(async () => {
    if (!enabled || !source || loadingOlderRef.current || lines.length === 0) {
      return;
    }
    loadingOlderRef.current = true;
    setLoadingOlder(true);
    try {
      const before = lines[0]?.index;
      if (before == null) return;
      const olderQuery = buildQuery(debouncedFiltersRef.current, {
        before,
        limit: PAGE_SIZE,
        tail: undefined,
      });
      const older = await fetchGuestLogs(vmId, source, olderQuery);
      const filtered = older.filter(
        (line) => line.index != null && line.index < before,
      );
      if (filtered.length === 0) return;
      const list = listRef.current;
      const previousHeight = list?.scrollHeight ?? 0;
      setLines((current) => trimFront(filtered.concat(current)));
      requestAnimationFrame(() => {
        if (list) {
          list.scrollTop = list.scrollHeight - previousHeight;
        }
      });
    } catch (err) {
      setStreamError(String(err));
    } finally {
      loadingOlderRef.current = false;
      setLoadingOlder(false);
    }
  }, [enabled, source, lines, vmId, trimFront]);

  const onScroll = useCallback(() => {
    const list = listRef.current;
    if (!list) return;
    const nearBottom =
      list.scrollHeight - list.scrollTop - list.clientHeight < 40;
    if (!nearBottom) {
      if (autoScrollRef.current) {
        setAutoScrollState(false);
        autoScrollRef.current = false;
      }
    } else if (!autoScrollRef.current || pendingLiveRef.current > 0) {
      setAutoScrollState(true);
      autoScrollRef.current = true;
      if (pendingLiveRef.current > 0) {
        setPendingLive(0);
        setConnectTick((tick) => tick + 1);
      }
    }
    if (list.scrollTop === 0) {
      void loadOlder();
    }
  }, [loadOlder]);

  useEffect(() => {
    if (!enabled || !source) {
      setLines([]);
      setStreamError(null);
      return;
    }

    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    setStreamError(null);
    setLines([]);
    setPendingLive(0);
    setAutoScrollState(true);
    autoScrollRef.current = true;
    const channel = `vzctl-guest-log-${vmId}-${source}`;

    void (async () => {
      try {
        const snapshot = await fetchGuestLogs(vmId, source, query);
        if (!cancelled) setLines(snapshot.slice(-MAX_WINDOW));
      } catch (err) {
        if (!cancelled) setStreamError(String(err));
      }
      if (cancelled || isDemoMode()) return;
      try {
        const path = guestServiceApiPath(vmId, source, "/api/logs/sse", query);
        await invoke("subscribe_guest_logs", {
          pathAndQuery: path,
          channel,
        });
        unlisten = await listen<SsePayload>(channel, (event) => {
          const payload = event.payload;
          const kind = payload.event ?? "line";
          if (kind === "status") {
            applyStatus((payload.data as IwatchStatus) ?? {});
            return;
          }
          if (kind === "fields") {
            const fields = payload.data;
            if (Array.isArray(fields)) setObservedFields(fields);
            return;
          }
          if (kind !== "line") return;
          const line = payload.data as IwatchLine | undefined;
          if (!line?.text) return;
          if (!autoScrollRef.current) {
            setPendingLive((count) => count + 1);
            return;
          }
          setLines((current) => trimFront([...current, line]));
          requestAnimationFrame(() => {
            const list = listRef.current;
            if (list && autoScrollRef.current) {
              list.scrollTop = list.scrollHeight;
            }
          });
        });
      } catch (err) {
        if (!cancelled) setStreamError(String(err));
      }
    })();

    return () => {
      cancelled = true;
      void unlisten?.();
    };
  }, [vmId, source, query, enabled, connectTick, applyStatus, trimFront]);

  useEffect(() => {
    if (!autoScroll) return;
    const list = listRef.current;
    if (!list) return;
    list.scrollTop = list.scrollHeight;
  }, [lines, autoScroll]);

  return {
    lines,
    selectedIndex,
    autoScroll,
    pendingLive,
    processStatus,
    observedFields,
    groupValues,
    streamError,
    loadingOlder,
    setAutoScroll,
    selectLine,
    loadOlder,
    connectNow,
    listRef,
    onScroll,
    trimBack,
  };
}
