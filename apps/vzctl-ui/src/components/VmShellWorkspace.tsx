import { useLayoutEffect, useMemo, useState } from "react";
import { useShellSurface } from "@/components/TerminalDock";
import { useT } from "@/lib/i18n";
import { cx } from "@/components/ui/cx";
import {
  MAX_SHELL_TABS,
  useTerminalStore,
  type TerminalKind,
} from "@/store/terminalStore";

/** Tab bar + stage that portals live sessions from the app-level dock. */
export function VmShellWorkspace({
  vmId,
  kind,
  active,
}: {
  vmId: string;
  kind: TerminalKind;
  active: boolean;
}) {
  const t = useT();
  const { setSurface } = useShellSurface();
  const [stage, setStage] = useState<HTMLDivElement | null>(null);
  // Select `tabs` by reference. Filtering in the zustand selector returns a
  // new array every snapshot and trips React 19 error #185 on VM open.
  const allTabs = useTerminalStore((state) => state.tabs);
  const tabs = useMemo(
    () => allTabs.filter((tab) => tab.vmId === vmId && tab.kind === kind),
    [allTabs, vmId, kind],
  );
  const activeTabId = useTerminalStore((state) => state.activeByVm[vmId]);
  const ensureShell = useTerminalStore((state) => state.ensureShell);
  const ensureConsole = useTerminalStore((state) => state.ensureConsole);
  const addShellTab = useTerminalStore((state) => state.addShellTab);
  const closeTab = useTerminalStore((state) => state.closeTab);
  const setActive = useTerminalStore((state) => state.setActive);

  useLayoutEffect(() => {
    if (!active) return;
    if (kind === "shell") ensureShell(vmId);
    else ensureConsole(vmId);
  }, [active, kind, vmId, ensureShell, ensureConsole]);

  const currentId =
    kind === "console" ? (tabs[0]?.id ?? null) : (activeTabId ?? null);

  useLayoutEffect(() => {
    if (!active) return;
    setSurface({
      vmId,
      kind,
      activeTabId: currentId,
      stage,
    });
    return () => setSurface(null);
  }, [active, vmId, kind, currentId, stage, setSurface]);

  const canAdd = kind === "shell" && tabs.length < MAX_SHELL_TABS;

  return (
    <div className="shell-workspace">
      {kind === "shell" ? (
        <div className="shell-tabs" role="tablist" aria-label={t("vmDetail.shellTitle")}>
          {tabs.map((tab) => {
            const selected = tab.id === currentId;
            return (
              <div
                key={tab.id}
                className={cx("shell-tab", selected && "is-active")}
                role="tab"
                aria-selected={selected}
              >
                <button
                  type="button"
                  className="shell-tab-label"
                  onClick={() => setActive(vmId, tab.id)}
                >
                  {t("terminal.tab", { n: tab.title })}
                </button>
                <button
                  type="button"
                  className="shell-tab-close"
                  aria-label={t("terminal.closeTab")}
                  onClick={() => closeTab(tab.id)}
                >
                  ×
                </button>
              </div>
            );
          })}
          <button
            type="button"
            className="shell-tab-add"
            disabled={!canAdd}
            aria-label={t("terminal.newTab")}
            onClick={() => addShellTab(vmId)}
          >
            +
          </button>
        </div>
      ) : null}
      <div className="shell-stage" ref={setStage} />
    </div>
  );
}
