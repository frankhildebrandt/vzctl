import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { isDemoMode } from "@/lib/demo";
import {
  fetchGuestLogs,
  guestServiceApiPath,
  type IwatchLine,
  type IwatchStatus,
} from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import {
  formatLogLineViews,
  type LogLineView,
} from "@/lib/logLineViews";
import {
  buildLogsStreamQuery,
  serializeFilterQueryKey,
  TEXT_FILTER_DEBOUNCE_MS,
  type GuestLogFilters,
} from "@/lib/logStreamQuery";

export type { GuestLogFilters } from "@/lib/logStreamQuery";
export const MAX_WINDOW = 400;
export const PAGE_SIZE = 100;

type SsePayload = {
  event?: string;
  data?: IwatchLine | IwatchStatus | string[];
};

export type LogScrollApi = {
  scrollToBottom: () => void;
  scrollToIndex: (index: number) => void;
  preserveScrollAfterPrepend: (previousScrollHeight: number) => void;
};

type UseGuestLogStreamInput = {
  vmId: string;
  source: string;
  filters: GuestLogFilters;
  observedFields: string[];
  hiddenFields: HiddenFields;
  enabled?: boolean;
};

type UseGuestLogStreamResult = {
  lineViews: LogLineView[];
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
  jumpLine: (delta: number) => void;
  loadOlder: () => Promise<void>;
  connectNow: () => void;
  listRef: React.RefObject<HTMLDivElement | null>;
  scrollApiRef: React.MutableRefObject<LogScrollApi | null>;
  onScroll: () => void;
  onDisableFollow: () => void;
};

type LineStore = {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => LogLineView[];
  setFormat: (observedFields: string[], hiddenFields: HiddenFields) => void;
  replace: (lines: IwatchLine[]) => void;
  appendMany: (lines: IwatchLine[]) => void;
  prepend: (lines: IwatchLine[]) => void;
  clear: () => void;
};

function trimFront(lines: IwatchLine[]): IwatchLine[] {
  if (lines.length <= MAX_WINDOW) return lines;
  return lines.slice(lines.length - MAX_WINDOW);
}

function trimBack(lines: IwatchLine[]): IwatchLine[] {
  if (lines.length <= MAX_WINDOW) return lines;
  return lines.slice(0, MAX_WINDOW);
}

function createLineStore(): LineStore {
  let rawLines: IwatchLine[] = [];
  let views: LogLineView[] = [];
  let observedFields: string[] = [];
  let hiddenFields: HiddenFields = { raw: true };
  const listeners = new Set<() => void>();

  const emit = () => {
    for (const listener of listeners) listener();
  };

  const rebuild = () => {
    views = formatLogLineViews(rawLines, observedFields, hiddenFields);
  };

  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getSnapshot() {
      return views;
    },
    setFormat(nextObservedFields, nextHiddenFields) {
      observedFields = nextObservedFields;
      hiddenFields = nextHiddenFields;
      if (rawLines.length === 0) return;
      rebuild();
      emit();
    },
    replace(lines) {
      rawLines = lines;
      rebuild();
      emit();
    },
    appendMany(lines) {
      if (lines.length === 0) return;
      rawLines = trimFront(rawLines.concat(lines));
      rebuild();
      emit();
    },
    prepend(lines) {
      if (lines.length === 0) return;
      rawLines = trimBack(lines.concat(rawLines));
      rebuild();
      emit();
    },
    clear() {
      if (rawLines.length === 0 && views.length === 0) return;
      rawLines = [];
      views = [];
      emit();
    },
  };
}

/** Manage iwatch snapshot + SSE stream with virtualized, store-backed line views. */
export function useGuestLogStream({
  vmId,
  source,
  filters,
  observedFields,
  hiddenFields,
  enabled = true,
}: UseGuestLogStreamInput): UseGuestLogStreamResult {
  const lineStoreRef = useRef<LineStore | null>(null);
  if (lineStoreRef.current == null) {
    lineStoreRef.current = createLineStore();
  }
  const lineStore = lineStoreRef.current;

  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [autoScroll, setAutoScrollState] = useState(true);
  const [pendingLive, setPendingLive] = useState(0);
  const [processStatus, setProcessStatus] = useState<IwatchStatus>({});
  const [streamObservedFields, setStreamObservedFields] = useState<string[]>([]);
  const [groupValues, setGroupValues] = useState<string[]>([]);
  const [streamError, setStreamError] = useState<string | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [connectTick, setConnectTick] = useState(0);
  const [debouncedTextFilters, setDebouncedTextFilters] = useState({
    q: filters.q,
    fieldFilters: filters.fieldFilters,
  });

  const listRef = useRef<HTMLDivElement>(null);
  const scrollApiRef = useRef<LogScrollApi | null>(null);
  const activeSubscriptionRef = useRef<string | null>(null);
  const autoScrollRef = useRef(autoScroll);
  const pendingLiveRef = useRef(pendingLive);
  const loadingOlderRef = useRef(false);
  const filtersRef = useRef(filters);
  const pendingLinesRef = useRef<IwatchLine[]>([]);
  const flushRafRef = useRef<number | null>(null);
  const scrollRafRef = useRef<number | null>(null);
  const ignoreScrollRef = useRef(false);

  const lineViews = useSyncExternalStore(
    lineStore.subscribe,
    lineStore.getSnapshot,
    lineStore.getSnapshot,
  );

  const effectiveFilters = useMemo(
    (): GuestLogFilters => ({
      q: debouncedTextFilters.q,
      minLevel: filters.minLevel,
      groupField: filters.groupField,
      groupValue: filters.groupValue,
      fieldFilters: debouncedTextFilters.fieldFilters,
    }),
    [
      debouncedTextFilters,
      filters.minLevel,
      filters.groupField,
      filters.groupValue,
    ],
  );

  const filterQueryKey = useMemo(
    () => serializeFilterQueryKey(effectiveFilters),
    [effectiveFilters],
  );

  const query = useMemo(
    () => buildLogsStreamQuery(effectiveFilters, { tail: MAX_WINDOW }),
    [effectiveFilters],
  );

  const effectiveObservedFields =
    streamObservedFields.length > 0 ? streamObservedFields : observedFields;

  useEffect(() => {
    filtersRef.current = filters;
  }, [filters]);

  useEffect(() => {
    autoScrollRef.current = autoScroll;
  }, [autoScroll]);

  useEffect(() => {
    pendingLiveRef.current = pendingLive;
  }, [pendingLive]);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      setDebouncedTextFilters({
        q: filters.q,
        fieldFilters: filters.fieldFilters,
      });
    }, TEXT_FILTER_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [filters.q, filters.fieldFilters]);

  useEffect(() => {
    lineStore.setFormat(effectiveObservedFields, hiddenFields);
  }, [effectiveObservedFields, hiddenFields, lineStore]);

  const applyStatus = useCallback((status: IwatchStatus) => {
    setProcessStatus((current) => {
      if (
        current.process === status.process &&
        current.bufferLen === status.bufferLen &&
        current.bufferCap === status.bufferCap &&
        current.commandTitle === status.commandTitle &&
        current.lastUrl === status.lastUrl
      ) {
        return current;
      }
      return status;
    });
    if (status.groupValues) {
      setGroupValues((current) =>
        current === status.groupValues ? current : (status.groupValues ?? current),
      );
    }
    if (status.observedFields) {
      setStreamObservedFields((current) => {
        const next = status.observedFields ?? current;
        if (
          current.length === next.length &&
          current.every((field, index) => field === next[index])
        ) {
          return current;
        }
        return next;
      });
    }
  }, []);

  const flushPendingLines = useCallback(() => {
    flushRafRef.current = null;
    const batch = pendingLinesRef.current;
    if (batch.length === 0) return;
    pendingLinesRef.current = [];
    lineStore.appendMany(batch);
    if (autoScrollRef.current) {
      requestAnimationFrame(() => {
        scrollApiRef.current?.scrollToBottom();
      });
    }
  }, [lineStore]);

  const scheduleFlush = useCallback(() => {
    if (flushRafRef.current != null) return;
    flushRafRef.current = requestAnimationFrame(flushPendingLines);
  }, [flushPendingLines]);

  const setAutoScroll = useCallback((value: boolean) => {
    setAutoScrollState(value);
    autoScrollRef.current = value;
    if (value) {
      const hadPending = pendingLiveRef.current > 0;
      setPendingLive(0);
      pendingLiveRef.current = 0;
      if (hadPending) {
        setConnectTick((tick) => tick + 1);
      } else {
        requestAnimationFrame(() => {
          scrollApiRef.current?.scrollToBottom();
        });
      }
    }
  }, []);

  const selectLine = useCallback((index: number) => {
    setSelectedIndex(index);
  }, []);

  const connectNow = useCallback(() => {
    setDebouncedTextFilters({
      q: filtersRef.current.q,
      fieldFilters: filtersRef.current.fieldFilters,
    });
    setConnectTick((tick) => tick + 1);
  }, []);

  const jumpLine = useCallback(
    (delta: number) => {
      const views = lineStore.getSnapshot();
      if (views.length === 0) return;
      let position = views.findIndex((view) => view.index === selectedIndex);
      if (position < 0) position = delta > 0 ? -1 : views.length;
      position = Math.max(0, Math.min(views.length - 1, position + delta));
      const view = views[position];
      setSelectedIndex(view.index);
      scrollApiRef.current?.scrollToIndex(position);
    },
    [lineStore, selectedIndex],
  );

  const loadOlder = useCallback(async () => {
    const views = lineStore.getSnapshot();
    if (!enabled || !source || loadingOlderRef.current || views.length === 0) {
      return;
    }
    loadingOlderRef.current = true;
    setLoadingOlder(true);
    try {
      const before = views[0]?.index;
      if (before == null) return;
      const olderQuery = buildLogsStreamQuery(filtersRef.current, {
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
      lineStore.prepend(filtered);
      scrollApiRef.current?.preserveScrollAfterPrepend(previousHeight);
    } catch (err) {
      setStreamError(String(err));
    } finally {
      loadingOlderRef.current = false;
      setLoadingOlder(false);
    }
  }, [enabled, source, vmId, lineStore]);

  const handleScroll = useCallback(() => {
    if (ignoreScrollRef.current) return;
    const list = listRef.current;
    if (!list) return;
    const distanceFromBottom =
      list.scrollHeight - list.scrollTop - list.clientHeight;
    const nearBottom = distanceFromBottom < 80;
    if (!nearBottom) {
      if (autoScrollRef.current) {
        setAutoScrollState(false);
        autoScrollRef.current = false;
      }
      if (list.scrollTop === 0) {
        void loadOlder();
      }
      return;
    }
    if (!autoScrollRef.current) {
      setAutoScrollState(true);
      autoScrollRef.current = true;
    }
    if (pendingLiveRef.current > 0) {
      setPendingLive(0);
      pendingLiveRef.current = 0;
      setConnectTick((tick) => tick + 1);
    }
  }, [loadOlder]);

  const onScroll = useCallback(() => {
    if (scrollRafRef.current != null) return;
    scrollRafRef.current = requestAnimationFrame(() => {
      scrollRafRef.current = null;
      handleScroll();
    });
  }, [handleScroll]);

  const onDisableFollow = useCallback(() => {
    if (!autoScrollRef.current) return;
    setAutoScrollState(false);
    autoScrollRef.current = false;
  }, []);

  useEffect(() => {
    if (!enabled || !source) {
      lineStore.clear();
      setStreamError(null);
      return;
    }

    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    const subscriptionId = crypto.randomUUID();
    const channel = `vzctl-guest-log-${vmId}-${source}-${subscriptionId}`;
    activeSubscriptionRef.current = subscriptionId;

    setStreamError(null);
    pendingLinesRef.current = [];
    lineStore.clear();
    setPendingLive(0);
    setAutoScrollState(true);
    autoScrollRef.current = true;

    void (async () => {
      try {
        const snapshot = await fetchGuestLogs(vmId, source, query);
        if (cancelled || activeSubscriptionRef.current !== subscriptionId) {
          return;
        }
        lineStore.replace(snapshot.slice(-MAX_WINDOW));
        requestAnimationFrame(() => {
          scrollApiRef.current?.scrollToBottom();
        });
      } catch (err) {
        if (!cancelled && activeSubscriptionRef.current === subscriptionId) {
          setStreamError(String(err));
        }
      }
      if (
        cancelled ||
        isDemoMode() ||
        activeSubscriptionRef.current !== subscriptionId
      ) {
        return;
      }
      try {
        const path = guestServiceApiPath(vmId, source, "/api/logs/sse", query);
        await invoke("subscribe_guest_logs", {
          subscriptionId,
          pathAndQuery: path,
          channel,
        });
        if (cancelled || activeSubscriptionRef.current !== subscriptionId) {
          void invoke("unsubscribe_guest_logs", { subscriptionId });
          return;
        }
        unlisten = await listen<SsePayload>(channel, (event) => {
          if (activeSubscriptionRef.current !== subscriptionId) return;
          const payload = event.payload;
          const kind = payload.event ?? "line";
          if (kind === "status") {
            applyStatus((payload.data as IwatchStatus) ?? {});
            return;
          }
          if (kind === "fields") {
            const fields = payload.data;
            if (Array.isArray(fields)) {
              setStreamObservedFields((current) => {
                if (
                  current.length === fields.length &&
                  current.every((field, index) => field === fields[index])
                ) {
                  return current;
                }
                return fields;
              });
            }
            return;
          }
          if (kind !== "line") return;
          const line = payload.data as IwatchLine | undefined;
          if (!line?.text) return;
          if (!autoScrollRef.current) {
            setPendingLive((count) => count + 1);
            return;
          }
          pendingLinesRef.current.push(line);
          scheduleFlush();
        });
      } catch (err) {
        if (!cancelled && activeSubscriptionRef.current === subscriptionId) {
          setStreamError(String(err));
        }
      }
    })();

    return () => {
      cancelled = true;
      if (activeSubscriptionRef.current === subscriptionId) {
        activeSubscriptionRef.current = null;
      }
      if (flushRafRef.current != null) {
        cancelAnimationFrame(flushRafRef.current);
        flushRafRef.current = null;
      }
      void invoke("unsubscribe_guest_logs", { subscriptionId });
      void unlisten?.();
    };
  }, [
    vmId,
    source,
    filterQueryKey,
    query,
    enabled,
    connectTick,
    applyStatus,
    lineStore,
    scheduleFlush,
  ]);

  useEffect(
    () => () => {
      if (flushRafRef.current != null) {
        cancelAnimationFrame(flushRafRef.current);
      }
      if (scrollRafRef.current != null) {
        cancelAnimationFrame(scrollRafRef.current);
      }
    },
    [],
  );

  return {
    lineViews,
    selectedIndex,
    autoScroll,
    pendingLive,
    processStatus,
    observedFields: effectiveObservedFields,
    groupValues,
    streamError,
    loadingOlder,
    setAutoScroll,
    selectLine,
    jumpLine,
    loadOlder,
    connectNow,
    listRef,
    scrollApiRef,
    onScroll,
    onDisableFollow,
  };
}
