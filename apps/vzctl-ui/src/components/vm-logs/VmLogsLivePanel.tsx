import { memo, useEffect, type MutableRefObject } from "react";
import { VmLogsStream } from "@/components/vm-logs/VmLogsStream";
import { useGuestLogStream } from "@/hooks/useGuestLogStream";
import type { GuestLogFilters } from "@/hooks/useGuestLogStream";
import type { IwatchStatus } from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";

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
    onStatusChange({
      pendingLive: stream.pendingLive,
      autoScroll: stream.autoScroll,
      processStatus: stream.processStatus,
      observedFields: stream.observedFields,
      groupValues: stream.groupValues,
      streamError: stream.streamError,
      selectedIndex: stream.selectedIndex,
    });
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

  return (
    <VmLogsStream
      lineViews={stream.lineViews}
      selectedIndex={stream.selectedIndex}
      listRef={stream.listRef}
      onScroll={stream.onScroll}
      onSelectLine={stream.selectLine}
      onOpenDetail={onOpenDetail}
    />
  );
});
