import { useVirtualizer } from "@tanstack/react-virtual";
import {
  memo,
  useCallback,
  useEffect,
  type MouseEvent,
  type MutableRefObject,
  type RefObject,
} from "react";
import { VmLogLine } from "@/components/vm-logs/VmLogLine";
import { useT } from "@/lib/i18n";
import type { LogScrollApi } from "@/hooks/useGuestLogStream";
import type { LogLineView } from "@/lib/logLineViews";

const ESTIMATED_LINE_HEIGHT = 22;

type Props = {
  lineViews: LogLineView[];
  selectedIndex: number;
  listRef: RefObject<HTMLDivElement | null>;
  scrollApiRef: MutableRefObject<LogScrollApi | null>;
  onScroll: () => void;
  onDisableFollow: () => void;
  onSelectLine: (index: number) => void;
  onOpenDetail: (index: number) => void;
};

function lineIndexFromTarget(target: EventTarget | null): number | null {
  if (!(target instanceof Element)) return null;
  const row = target.closest<HTMLElement>("[data-index]");
  if (!row) return null;
  const index = Number(row.dataset.index);
  return Number.isFinite(index) ? index : null;
}

export const VmLogsVirtualStream = memo(function VmLogsVirtualStream({
  lineViews,
  selectedIndex,
  listRef,
  scrollApiRef,
  onScroll,
  onDisableFollow,
  onSelectLine,
  onOpenDetail,
}: Props) {
  const t = useT();

  const virtualizer = useVirtualizer({
    count: lineViews.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => ESTIMATED_LINE_HEIGHT,
    overscan: 12,
    getItemKey: (index) => lineViews[index]?.key ?? index,
  });

  useEffect(() => {
    scrollApiRef.current = {
      scrollToBottom: () => {
        if (lineViews.length === 0) return;
        virtualizer.scrollToIndex(lineViews.length - 1, { align: "end" });
      },
      scrollToIndex: (index: number) => {
        if (index < 0 || index >= lineViews.length) return;
        virtualizer.scrollToIndex(index, { align: "auto" });
      },
      preserveScrollAfterPrepend: (previousScrollHeight: number) => {
        const list = listRef.current;
        if (!list) return;
        requestAnimationFrame(() => {
          const next = listRef.current;
          if (!next) return;
          next.scrollTop += next.scrollHeight - previousScrollHeight;
        });
      },
    };
    return () => {
      scrollApiRef.current = null;
    };
  }, [lineViews.length, listRef, scrollApiRef, virtualizer]);

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;

    const onWheel = (event: WheelEvent) => {
      if (event.deltaY < 0) onDisableFollow();
    };

    let touchY: number | null = null;
    const onTouchStart = (event: TouchEvent) => {
      touchY = event.touches[0]?.clientY ?? null;
    };
    const onTouchMove = (event: TouchEvent) => {
      if (touchY == null) return;
      const y = event.touches[0]?.clientY;
      if (y != null && y > touchY + 8) onDisableFollow();
      touchY = y ?? touchY;
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.key === "PageUp" ||
        event.key === "Home" ||
        (event.key === "ArrowUp" && event.altKey)
      ) {
        onDisableFollow();
      }
    };

    list.addEventListener("wheel", onWheel, { passive: true });
    list.addEventListener("touchstart", onTouchStart, { passive: true });
    list.addEventListener("touchmove", onTouchMove, { passive: true });
    list.addEventListener("keydown", onKeyDown);
    return () => {
      list.removeEventListener("wheel", onWheel);
      list.removeEventListener("touchstart", onTouchStart);
      list.removeEventListener("touchmove", onTouchMove);
      list.removeEventListener("keydown", onKeyDown);
    };
  }, [listRef, onDisableFollow, lineViews.length]);

  const onClick = useCallback(
    (event: MouseEvent<HTMLDivElement>) => {
      const index = lineIndexFromTarget(event.target);
      if (index == null) return;
      if (event.detail >= 2) onOpenDetail(index);
      else onSelectLine(index);
    },
    [onOpenDetail, onSelectLine],
  );

  if (lineViews.length === 0) {
    return (
      <div
        ref={listRef}
        className="vm-logs-list"
        role="list"
        aria-label={t("vmLogs.stream")}
        onScroll={onScroll}
      >
        <div className="vm-logs-empty">{t("vmLogs.waiting")}</div>
      </div>
    );
  }

  const items = virtualizer.getVirtualItems();

  return (
    <div
      ref={listRef}
      className="vm-logs-list"
      role="list"
      aria-label={t("vmLogs.stream")}
      onScroll={onScroll}
      onClick={onClick}
    >
      <div
        className="vm-logs-virtual-inner"
        style={{ height: virtualizer.getTotalSize() }}
      >
        {items.map((item) => {
          const view = lineViews[item.index];
          if (!view) return null;
          return (
            <div
              key={view.key}
              ref={virtualizer.measureElement}
              data-index={item.index}
              className="vm-logs-virtual-row"
              style={{ transform: `translateY(${item.start}px)` }}
            >
              <VmLogLine view={view} selected={selectedIndex === view.index} />
            </div>
          );
        })}
      </div>
    </div>
  );
});
