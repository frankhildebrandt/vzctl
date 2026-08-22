import type { IwatchLine } from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import { visibleColumns } from "@/lib/iwatchFormat";
import { cx } from "@/components/ui/cx";

type Props = {
  line: IwatchLine;
  observedFields: string[];
  hiddenFields: HiddenFields;
  selected: boolean;
  onSelect: () => void;
  onOpenDetail: () => void;
};

export function VmLogLine({
  line,
  observedFields,
  hiddenFields,
  selected,
  onSelect,
  onOpenDetail,
}: Props) {
  const cols = visibleColumns(line, observedFields, hiddenFields);
  const index = line.index ?? -1;

  return (
    <li
      className={cx(
        "vm-logs-line",
        cols.levelClass,
        cols.source == null && "no-source",
        selected && "is-selected",
      )}
      data-index={index}
      onClick={onSelect}
      onDoubleClick={onOpenDetail}
    >
      <span className="vm-logs-ts">{cols.ts}</span>
      {cols.source != null ? (
        <span className="vm-logs-source">{cols.source}</span>
      ) : null}
      <span
        className="vm-logs-body"
        dangerouslySetInnerHTML={{ __html: cols.body }}
      />
    </li>
  );
}
