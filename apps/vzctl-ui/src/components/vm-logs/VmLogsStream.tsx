import { memo, useCallback, type MouseEvent, type RefObject } from "react";
import { VmLogLine } from "@/components/vm-logs/VmLogLine";
import { useT } from "@/lib/i18n";
import type { LogLineView } from "@/lib/logLineViews";

type Props = {
  lineViews: LogLineView[];
  selectedIndex: number;
  listRef: RefObject<HTMLUListElement | null>;
  onScroll: () => void;
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

export const VmLogsStream = memo(function VmLogsStream({
  lineViews,
  selectedIndex,
  listRef,
  onScroll,
  onSelectLine,
  onOpenDetail,
}: Props) {
  const t = useT();

  const onClick = useCallback(
    (event: MouseEvent<HTMLUListElement>) => {
      const index = lineIndexFromTarget(event.target);
      if (index == null) return;
      if (event.detail >= 2) onOpenDetail(index);
      else onSelectLine(index);
    },
    [onOpenDetail, onSelectLine],
  );

  return (
    <ul
      ref={listRef}
      className="vm-logs-list"
      aria-label={t("vmLogs.stream")}
      onScroll={onScroll}
      onClick={onClick}
    >
      {lineViews.length === 0 ? (
        <li className="vm-logs-empty">{t("vmLogs.waiting")}</li>
      ) : (
        lineViews.map((view) => (
          <VmLogLine
            key={view.key}
            view={view}
            selected={selectedIndex === view.index}
          />
        ))
      )}
    </ul>
  );
});
