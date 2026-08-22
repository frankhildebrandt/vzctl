import type { RefObject } from "react";
import type { IwatchLine } from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import { VmLogLine } from "@/components/vm-logs/VmLogLine";
import { useT } from "@/lib/i18n";

type Props = {
  lines: IwatchLine[];
  observedFields: string[];
  hiddenFields: HiddenFields;
  selectedIndex: number;
  listRef: RefObject<HTMLUListElement | null>;
  onScroll: () => void;
  onSelectLine: (index: number) => void;
  onOpenDetail: (index: number) => void;
};

export function VmLogsStream({
  lines,
  observedFields,
  hiddenFields,
  selectedIndex,
  listRef,
  onScroll,
  onSelectLine,
  onOpenDetail,
}: Props) {
  const t = useT();

  return (
    <ul
      ref={listRef}
      className="vm-logs-list"
      aria-label={t("vmLogs.stream")}
      onScroll={onScroll}
    >
      {lines.length === 0 ? (
        <li className="vm-logs-empty">{t("vmLogs.waiting")}</li>
      ) : (
        lines.map((line) => {
          const index = line.index ?? -1;
          return (
            <VmLogLine
              key={`${line.session ?? 0}-${index}`}
              line={line}
              observedFields={observedFields}
              hiddenFields={hiddenFields}
              selected={selectedIndex === index}
              onSelect={() => onSelectLine(index)}
              onOpenDetail={() => onOpenDetail(index)}
            />
          );
        })
      )}
    </ul>
  );
}
