import type { IwatchLine } from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import { visibleColumns } from "@/lib/iwatchFormat";

export type LogLineView = {
  key: string;
  index: number;
  ts: string;
  source: string | null;
  bodyHtml: string;
  levelClass: string;
  hideSource: boolean;
};

export function formatLogLineView(
  line: IwatchLine,
  observedFields: string[],
  hiddenFields: HiddenFields,
): LogLineView {
  const cols = visibleColumns(line, observedFields, hiddenFields);
  const index = line.index ?? -1;
  return {
    key: `${line.session ?? 0}-${index}`,
    index,
    ts: cols.ts,
    source: cols.source,
    bodyHtml: cols.body,
    levelClass: cols.levelClass,
    hideSource: cols.source == null,
  };
}

export function formatLogLineViews(
  lines: readonly IwatchLine[],
  observedFields: string[],
  hiddenFields: HiddenFields,
): LogLineView[] {
  return lines.map((line) => formatLogLineView(line, observedFields, hiddenFields));
}

export function hiddenFieldsKey(hiddenFields: HiddenFields): string {
  return Object.keys(hiddenFields).sort().join("\0");
}
