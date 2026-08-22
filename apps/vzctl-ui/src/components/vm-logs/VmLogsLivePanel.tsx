import { memo, useEffect, useRef, type MutableRefObject } from "react";
import { VmLogsVirtualStream } from "@/components/vm-logs/VmLogsVirtualStream";
import { useGuestLogStream } from "@/hooks/useGuestLogStream";
import type { IwatchStatus } from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import type { GuestLogFilters } from "@/lib/logStreamQuery";

export type VmLogsLiveStatus = {
  pendingLive: number;
  autoScroll: boolean;
  processStatus: IwatchStatus;
  observedFields: string[];
  groupValues: string[];
  streamError: string | null;
  selectedIndex: number;
};

export type VmLogsStreamActions = {
  connectNow: () => void;
  jumpLine: (delta: number) => void;
  selectLine: (index: number) => void;
  setAutoScroll: (value: boolean) => void;
};

type Props = {
  vmId: string;
  source: string;
  filters: GuestLogFilters;
  observedFields: string[];
  hiddenFields: HiddenFields;
  onStatusChange: (status: VmLogsLiveStatus) => void;
  onOpenDetail: (index: number) => void;
  actionsRef: MutableRefObject<VmLogsStreamActions>;
};

const STATUS_THROTTLE_MS = 250;

function isCriticalStatusChange(
  prev: VmLogsLiveStatus | null,
  next: VmLogsLiveStatus,
): boolean {
  if (!prev) return true;
  return (
    prev.pendingLive !== next.pendingLive ||
    prev.autoScroll !== next.autoScroll ||
    prev.streamError !== next.streamError ||
    prev.selectedIndex !== next.selectedIndex ||
    prev.processStatus.process !== next.processStatus.process ||
    prev.processStatus.commandTitle !== next.processStatus.commandTitle ||
    prev.processStatus.lastUrl !== next.processStatus.lastUrl ||
    prev.observedFields.join("\0") !== next.observedFields.join("\0") ||
    prev.groupValues.join("\0") !== next.groupValues.join("\0")
  );
}

function isBufferStatusChange(
  prev: VmLogsLiveStatus | null,
  next: VmLogsLiveStatus,
): boolean {
  if (!prev) return false;
  return (
    prev.processStatus.bufferLen !== next.processStatus.bufferLen ||
    prev.processStatus.bufferCap !== next.processStatus.bufferCap
  );
}

export const VmLogsLivePanel = memo(function VmLogsLivePanel({
  vmId,
  source,
  filters,
  observedFields,
  hiddenFields,
  onStatusChange,
  onOpenDetail,
  actionsRef,
}: Props) {
  const stream = useGuestLogStream({
    vmId,
    source,
    filters,
    observedFields,
    hiddenFields,
    enabled: Boolean(source),
  });

  const previousStatusRef = useRef<VmLogsLiveStatus | null>(null);
  const throttleTimerRef = useRef<number | null>(null);
  const pendingStatusRef = useRef<VmLogsLiveStatus | null>(null);

  useEffect(() => {
    actionsRef.current = {
      connectNow: stream.connectNow,
      jumpLine: stream.jumpLine,
      selectLine: stream.selectLine,
      setAutoScroll: stream.setAutoScroll,
    };
  }, [
    actionsRef,
    stream.connectNow,
    stream.jumpLine,
    stream.selectLine,
    stream.setAutoScroll,
  ]);

  useEffect(() => {
    const next: VmLogsLiveStatus = {
      pendingLive: stream.pendingLive,
      autoScroll: stream.autoScroll,
      processStatus: stream.processStatus,
      observedFields: stream.observedFields,
      groupValues: stream.groupValues,
      streamError: stream.streamError,
      selectedIndex: stream.selectedIndex,
    };
    const prev = previousStatusRef.current;

    const emit = (status: VmLogsLiveStatus) => {
      previousStatusRef.current = status;
      onStatusChange(status);
    };

    if (isCriticalStatusChange(prev, next)) {
      if (throttleTimerRef.current != null) {
        window.clearTimeout(throttleTimerRef.current);
        throttleTimerRef.current = null;
      }
      pendingStatusRef.current = null;
      emit(next);
      return;
    }

    if (!isBufferStatusChange(prev, next)) return;

    pendingStatusRef.current = next;
    if (throttleTimerRef.current != null) return;
    throttleTimerRef.current = window.setTimeout(() => {
      throttleTimerRef.current = null;
      const pending = pendingStatusRef.current;
      if (pending) {
        pendingStatusRef.current = null;
        emit(pending);
      }
    }, STATUS_THROTTLE_MS);
  }, [
    onStatusChange,
    stream.pendingLive,
    stream.autoScroll,
    stream.processStatus,
    stream.observedFields,
    stream.groupValues,
    stream.streamError,
    stream.selectedIndex,
  ]);

  useEffect(
    () => () => {
      if (throttleTimerRef.current != null) {
        window.clearTimeout(throttleTimerRef.current);
      }
    },
    [],
  );

  return (
    <VmLogsVirtualStream
      lineViews={stream.lineViews}
      selectedIndex={stream.selectedIndex}
      listRef={stream.listRef}
      scrollApiRef={stream.scrollApiRef}
      onScroll={stream.onScroll}
      onDisableFollow={stream.onDisableFollow}
      onSelectLine={stream.selectLine}
      onOpenDetail={onOpenDetail}
    />
  );
});
