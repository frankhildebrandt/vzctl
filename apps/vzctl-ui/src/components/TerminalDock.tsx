import {
  createContext,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { Terminal } from "@/components/Terminal";
import { cx } from "@/components/ui/cx";
import {
  useTerminalStore,
  type TerminalKind,
  type TerminalTab,
} from "@/store/terminalStore";

export type ShellSurface = {
  vmId: string;
  kind: TerminalKind;
  activeTabId: string | null;
  stage: HTMLElement | null;
};

type SurfaceContextValue = {
  surface: ShellSurface | null;
  setSurface: (surface: ShellSurface | null) => void;
};

const SurfaceContext = createContext<SurfaceContextValue>({
  surface: null,
  setSurface: () => {},
});

export function useShellSurface() {
  return useContext(SurfaceContext);
}

/** Keeps terminal instances mounted across VM navigation. */
export function TerminalSessionRoot({ children }: { children: ReactNode }) {
  const [surface, setSurface] = useState<ShellSurface | null>(null);
  const value = useMemo(() => ({ surface, setSurface }), [surface]);
  return (
    <SurfaceContext.Provider value={value}>
      {children}
      <TerminalHold />
    </SurfaceContext.Provider>
  );
}

function TerminalHold() {
  const tabs = useTerminalStore((state) => state.tabs);
  const { surface } = useShellSurface();
  return (
    <div className="terminal-hold" aria-hidden>
      {tabs.map((tab) => (
        <DockedTerminal key={tab.id} tab={tab} surface={surface} />
      ))}
    </div>
  );
}

function DockedTerminal({
  tab,
  surface,
}: {
  tab: TerminalTab;
  surface: ShellSurface | null;
}) {
  const visible =
    surface != null &&
    surface.stage != null &&
    surface.vmId === tab.vmId &&
    surface.kind === tab.kind &&
    (tab.kind === "console" || surface.activeTabId === tab.id);
  const node = (
    <div className={cx("terminal-dock-slot", visible && "is-visible")}>
      <Terminal
        mode={tab.kind === "console" ? "attach" : "exec"}
        vmId={tab.vmId}
        cmd={tab.cmd.length > 0 ? tab.cmd : ["/bin/bash"]}
        active={visible}
      />
    </div>
  );
  if (visible && surface?.stage) {
    return createPortal(node, surface.stage);
  }
  return node;
}
