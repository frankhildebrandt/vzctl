import type { RefObject } from "react";
import { memo } from "react";
import { Button } from "@/components/ui";
import { IWATCH_LEVELS, type IwatchLevel, type IwatchStatus } from "@/lib/guestLogs";
import type { HiddenFields } from "@/lib/iwatchFormat";
import { useT } from "@/lib/i18n";

type Source = { name: string };

type Props = {
  sources: Source[];
  selectedSource: string;
  onSourceChange: (source: string) => void;
  q: string;
  onQChange: (value: string) => void;
  onQCommit: () => void;
  queryInputRef: RefObject<HTMLInputElement | null>;
  minLevel: string;
  onMinLevelChange: (value: string) => void;
  onBumpLevel: (delta: number) => void;
  groupField: string;
  onGroupFieldChange: (value: string) => void;
  groupValue: string;
  onGroupValueChange: (value: string) => void;
  fieldFilters: Record<string, string>;
  onFieldFilterChange: (field: string, value: string) => void;
  onFieldFilterRename: (oldField: string, nextField: string) => void;
  onFieldFilterRemove: (field: string) => void;
  onAddFieldFilter: () => void;
  observedFields: string[];
  groupValues: string[];
  processStatus: IwatchStatus;
  pendingLive: number;
  autoScroll: boolean;
  onAutoScrollChange: (value: boolean) => void;
  hiddenFields: HiddenFields;
  onHiddenFieldToggle: (field: string, visible: boolean) => void;
  showFieldVisibility: boolean;
  onToggleFieldVisibility: () => void;
  onRestart: () => void;
  onTruncate: () => void;
  onSeparator: () => void;
  onOpenUrl: () => void;
  onHelp: () => void;
  busyAction: string | null;
};

function groupFieldOptions(observedFields: string[]): string[] {
  const fields = ["component", ...observedFields.filter((field) => field !== "component")];
  return [...new Set(fields)];
}

function visibilityFields(observedFields: string[]): string[] {
  return ["raw", "source", ...observedFields];
}

export const VmLogsToolbar = memo(function VmLogsToolbar({
  sources,
  selectedSource,
  onSourceChange,
  q,
  onQChange,
  onQCommit,
  queryInputRef,
  minLevel,
  onMinLevelChange,
  onBumpLevel,
  groupField,
  onGroupFieldChange,
  groupValue,
  onGroupValueChange,
  fieldFilters,
  onFieldFilterChange,
  onFieldFilterRename,
  onFieldFilterRemove,
  onAddFieldFilter,
  observedFields,
  groupValues,
  processStatus,
  pendingLive,
  autoScroll,
  onAutoScrollChange,
  hiddenFields,
  onHiddenFieldToggle,
  showFieldVisibility,
  onToggleFieldVisibility,
  onRestart,
  onTruncate,
  onSeparator,
  onOpenUrl,
  onHelp,
  busyAction,
}: Props) {
  const t = useT();
  const bufferLabel = `${processStatus.bufferLen ?? 0}/${processStatus.bufferCap ?? 0}`;
  const statusLabel = processStatus.process ?? "idle";
  const filterKeys = Object.keys(fieldFilters);

  return (
    <div className="vm-logs-toolbar">
      <div className="vm-logs-toolbar-row">
        {sources.length > 1 ? (
          <label className="vm-logs-control">
            <span className="vm-logs-label">{t("vmLogs.source")}</span>
            <select
              value={selectedSource}
              onChange={(event) => onSourceChange(event.target.value)}
              aria-label={t("vmLogs.source")}
            >
              {sources.map((item) => (
                <option key={item.name} value={item.name}>
                  {item.name}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        <label className="vm-logs-control vm-logs-query">
          <span className="vm-logs-label">{t("vmLogs.query")}</span>
          <input
            ref={queryInputRef}
            value={q}
            onChange={(event) => onQChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                onQCommit();
              }
            }}
            placeholder={t("vmLogs.queryPlaceholder")}
            aria-label={t("vmLogs.query")}
          />
        </label>
        <label className="vm-logs-control">
          <span className="vm-logs-label">{t("vmLogs.minLevel")}</span>
          <div className="vm-logs-level">
            <select
              value={minLevel || "all"}
              onChange={(event) => onMinLevelChange(event.target.value)}
              aria-label={t("vmLogs.minLevel")}
            >
              {IWATCH_LEVELS.map((level) => (
                <option key={level} value={level}>
                  {level}
                </option>
              ))}
            </select>
            <button type="button" className="vm-logs-icon-btn" onClick={() => onBumpLevel(-1)}>
              -
            </button>
            <button type="button" className="vm-logs-icon-btn" onClick={() => onBumpLevel(1)}>
              +
            </button>
          </div>
        </label>
        <span className="vm-logs-status" title={t("vmLogs.buffer")}>
          <span className="vm-logs-process">{statusLabel}</span>
          <span className="vm-logs-buffer">
            {bufferLabel}
            {pendingLive > 0 ? ` +${pendingLive}` : ""}
          </span>
        </span>
        <div className="vm-logs-actions">
          <Button
            tone="secondary"
            disabled={busyAction != null}
            onClick={onRestart}
          >
            {busyAction === "restart" ? t("vmLogs.restartBusy") : t("vmLogs.restart")}
          </Button>
          <Button tone="secondary" disabled={busyAction != null} onClick={onTruncate}>
            {t("vmLogs.truncate")}
          </Button>
          <Button tone="secondary" disabled={busyAction != null} onClick={onSeparator}>
            {t("vmLogs.separator")}
          </Button>
          <Button tone="secondary" disabled={busyAction != null} onClick={onOpenUrl}>
            {t("vmLogs.openUrl")}
          </Button>
          <Button tone="secondary" onClick={onToggleFieldVisibility}>
            {t("vmLogs.fields")}
          </Button>
          <Button tone="secondary" onClick={onHelp}>
            ?
          </Button>
        </div>
      </div>

      <div className="vm-logs-toolbar-row">
        <label className="vm-logs-control">
          <span className="vm-logs-label">{t("vmLogs.groupField")}</span>
          <select
            value={groupField}
            onChange={(event) => onGroupFieldChange(event.target.value)}
            aria-label={t("vmLogs.groupField")}
          >
            {groupFieldOptions(observedFields).map((field) => (
              <option key={field} value={field}>
                {field}
              </option>
            ))}
          </select>
        </label>
        <label className="vm-logs-control">
          <span className="vm-logs-label">{t("vmLogs.groupValue")}</span>
          <select
            value={groupValue}
            onChange={(event) => onGroupValueChange(event.target.value)}
            aria-label={t("vmLogs.groupValue")}
          >
            <option value="">{t("vmLogs.anyGroup")}</option>
            {groupValues.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="vm-logs-check">
          <input
            type="checkbox"
            checked={autoScroll}
            onChange={(event) => onAutoScrollChange(event.target.checked)}
          />
          {t("vmLogs.follow")}
        </label>
        <Button tone="secondary" onClick={onAddFieldFilter}>
          {t("vmLogs.addFilter")}
        </Button>
      </div>

      {filterKeys.length > 0 ? (
        <div className="vm-logs-field-filters">
          {filterKeys.map((field) => (
            <div key={field} className="vm-logs-field-row">
              <input
                value={field}
                onChange={(event) => onFieldFilterRename(field, event.target.value)}
                aria-label={field}
              />
              <input
                value={fieldFilters[field] ?? ""}
                onChange={(event) => onFieldFilterChange(field, event.target.value)}
                aria-label={`${field} value`}
              />
              <button
                type="button"
                className="vm-logs-icon-btn"
                onClick={() => onFieldFilterRemove(field)}
              >
                ×
              </button>
            </div>
          ))}
        </div>
      ) : null}

      {showFieldVisibility ? (
        <div className="vm-logs-field-visibility">
          {visibilityFields(observedFields).map((field) => (
            <label key={field} className="vm-logs-check">
              <input
                type="checkbox"
                checked={!hiddenFields[field]}
                onChange={(event) => onHiddenFieldToggle(field, event.target.checked)}
              />
              {field}
            </label>
          ))}
        </div>
      ) : null}
    </div>
  );
});

export function bumpMinLevel(current: string, delta: number): IwatchLevel {
  const index = Math.max(
    0,
    Math.min(IWATCH_LEVELS.length - 1, IWATCH_LEVELS.indexOf((current || "all") as IwatchLevel) + delta),
  );
  return IWATCH_LEVELS[index];
}
