import { memo } from "react";
import { cx } from "@/components/ui/cx";
import type { LogLineView } from "@/lib/logLineViews";

type Props = {
  view: LogLineView;
  selected: boolean;
};

export const VmLogLine = memo(function VmLogLine({ view, selected }: Props) {
  return (
    <div
      role="listitem"
      className={cx(
        "vm-logs-line",
        view.levelClass,
        view.hideSource && "no-source",
        selected && "is-selected",
      )}
      data-index={view.index}
    >
      <span className="vm-logs-ts">{view.ts}</span>
      {view.source != null ? (
        <span className="vm-logs-source">{view.source}</span>
      ) : null}
      <span
        className="vm-logs-body"
        dangerouslySetInnerHTML={{ __html: view.bodyHtml }}
      />
    </div>
  );
});
