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
  type LogsQuery,
} from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import {
  formatLogLineView,
  formatLogLineViews,
  hiddenFieldsKey,
  type LogLineView,
} from "@/lib/logLineViews";

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
  listRef: React.RefObject<HTMLUListElement | null>;
  onScroll: () => void;
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

type LineStore = {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => LogLineView[];
  replace: (
    lines: IwatchLine[],
    observedFields: string[],
    hiddenFields: HiddenFields,
  ) => void;
  append: (
    line: IwatchLine,
    observedFields: string[],
    hiddenFields: HiddenFields,
  ) => void;
  appendMany: (
    lines: IwatchLine[],
    observedFields: string[],
    hiddenFields: HiddenFields,
  ) => void;
  prepend: (
    lines: IwatchLine[],
    observedFields: string[],
    hiddenFields: HiddenFields,
  ) => void;
  clear: () => void;
};

function createLineStore(): LineStore {
  let views: LogLineView[] = [];
  const listeners = new Set<() => void>();

  const emit = () => {
    for (const listener of listeners) listener();
  };

  const trimFront = () => {
    if (views.length > MAX_WINDOW) {
      views = views.slice(views.length - MAX_WINDOW);
    }
  };

  const trimBack = () => {
    if (views.length > MAX_WINDOW) {
      views = views.slice(0, MAX_WINDOW);
    }
  };

  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getSnapshot() {
      return views;
    },
    replace(lines, observedFields, hiddenFields) {
      views = formatLogLineViews(lines, observedFields, hiddenFields);
      emit();
    },
    append(line, observedFields, hiddenFields) {
      views = [...views, formatLogLineView(line, observedFields, hiddenFields)];
      trimFront();
      emit();
    },
    appendMany(lines, observedFields, hiddenFields) {
      if (lines.length === 0) return;
      views = [
        ...views,
        ...formatLogLineViews(lines, observedFields, hiddenFields),
      ];
      trimFront();
      emit();
    },
    prepend(lines, observedFields, hiddenFields) {
      views = [
        ...formatLogLineViews(lines, observedFields, hiddenFields),
        ...views,
      ];
      trimBack();
      emit();
    },
    clear() {
      if (views.length === 0) return;
      views = [];
      emit();
    },
  };
}

/** Manage iwatch snapshot + SSE stream with batched, store-backed line views. */
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

  const listRef = useRef<HTMLUListElement>(null);
  const autoScrollRef = useRef(autoScroll);
  const pendingLiveRef = useRef(pendingLive);
  const loadingOlderRef = useRef(false);
  const debouncedFiltersRef = useRef(filters);
  const pendingLinesRef = useRef<IwatchLine[]>([]);
  const flushRafRef = useRef<number | null>(null);
  const scrollRafRef = useRef<number | null>(null);
  const formatRef = useRef({
    observedFields,
    hiddenFields,
    hiddenKey: hiddenFieldsKey(hiddenFields),
  });
  const rawLinesRef = useRef<IwatchLine[]>([]);

  const [debouncedFilters, setDebouncedFilters] = useState(filters);

  const lineViews = useSyncExternalStore(
    lineStore.subscribe,
    lineStore.getSnapshot,
    lineStore.getSnapshot,
  );

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

  const effectiveObservedFields =
    streamObservedFields.length > 0 ? streamObservedFields : observedFields;

  useEffect(() => {
    formatRef.current = {
      observedFields: effectiveObservedFields,
      hiddenFields,
      hiddenKey: hiddenFieldsKey(hiddenFields),
    };
    if (rawLinesRef.current.length > 0) {
      lineStore.replace(
        rawLinesRef.current,
        effectiveObservedFields,
        hiddenFields,
      );
    }
  }, [effectiveObservedFields, hiddenFields, lineStore]);

  const applyStatus = useCallback((status: IwatchStatus) => {
    setProcessStatus(status);
    if (status.groupValues) setGroupValues(status.groupValues);
    if (status.observedFields) setStreamObservedFields(status.observedFields);
  }, []);

  const scrollToBottom = useCallback(() => {
    const list = listRef.current;
    if (list && autoScrollRef.current) {
      list.scrollTop = list.scrollHeight;
    }
  }, []);

  const flushPendingLines = useCallback(() => {
    flushRafRef.current = null;
    const batch = pendingLinesRef.current;
    if (batch.length === 0) return;
    pendingLinesRef.current = [];
    const { observedFields: fields, hiddenFields: hidden } = formatRef.current;
    lineStore.appendMany(batch, fields, hidden);
    rawLinesRef.current = rawLinesRef.current.concat(batch);
    if (rawLinesRef.current.length > MAX_WINDOW) {
      rawLinesRef.current = rawLinesRef.current.slice(-MAX_WINDOW);
    }
    requestAnimationFrame(scrollToBottom);
  }, [lineStore, scrollToBottom]);

  const scheduleFlush = useCallback(() => {
    if (flushRafRef.current != null) return;
    flushRafRef.current = requestAnimationFrame(flushPendingLines);
  }, [flushPendingLines]);

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

  const jumpLine = useCallback(
    (delta: number) => {
      const views = lineStore.getSnapshot();
      if (views.length === 0) return;
      let position = views.findIndex((view) => view.index === selectedIndex);
      if (position < 0) position = delta > 0 ? -1 : views.length;
      position = Math.max(0, Math.min(views.length - 1, position + delta));
      const view = views[position];
      setSelectedIndex(view.index);
      const node = listRef.current?.querySelector(
        `[data-index="${view.index}"]`,
      );
      node?.scrollIntoView({ block: "nearest" });
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
      const { observedFields: fields, hiddenFields: hidden } = formatRef.current;
      lineStore.prepend(filtered, fields, hidden);
      rawLinesRef.current = filtered.concat(rawLinesRef.current).slice(0, MAX_WINDOW);
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
  }, [enabled, source, vmId, lineStore]);

  const handleScroll = useCallback(() => {
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

  const onScroll = useCallback(() => {
    if (scrollRafRef.current != null) return;
    scrollRafRef.current = requestAnimationFrame(() => {
      scrollRafRef.current = null;
      handleScroll();
    });
  }, [handleScroll]);

  useEffect(() => {
    if (!enabled || !source) {
      lineStore.clear();
      rawLinesRef.current = [];
      setStreamError(null);
      return;
    }

    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    setStreamError(null);
    lineStore.clear();
    rawLinesRef.current = [];
    pendingLinesRef.current = [];
    setPendingLive(0);
    setAutoScrollState(true);
    autoScrollRef.current = true;
    const channel = `vzctl-guest-log-${vmId}-${source}`;

    void (async () => {
      try {
        const snapshot = await fetchGuestLogs(vmId, source, query);
        if (cancelled) return;
        const trimmed = snapshot.slice(-MAX_WINDOW);
        rawLinesRef.current = trimmed;
        const { observedFields: fields, hiddenFields: hidden } = formatRef.current;
        lineStore.replace(trimmed, fields, hidden);
        requestAnimationFrame(scrollToBottom);
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
            if (Array.isArray(fields)) setStreamObservedFields(fields);
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
        if (!cancelled) setStreamError(String(err));
      }
    })();

    return () => {
      cancelled = true;
      if (flushRafRef.current != null) {
        cancelAnimationFrame(flushRafRef.current);
        flushRafRef.current = null;
      }
      void unlisten?.();
    };
  }, [
    vmId,
    source,
    query,
    enabled,
    connectTick,
    applyStatus,
    lineStore,
    scheduleFlush,
    scrollToBottom,
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
    onScroll,
  };
}
