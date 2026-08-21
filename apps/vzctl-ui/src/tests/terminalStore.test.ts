import { beforeEach, describe, expect, it } from "vitest";
import {
  MAX_SHELL_TABS,
  useTerminalStore,
} from "@/store/terminalStore";

describe("terminalStore", () => {
  beforeEach(() => {
    useTerminalStore.setState({ tabs: [], activeByVm: {} });
  });

  it("creates the first shell tab and reuses it", () => {
    const first = useTerminalStore.getState().ensureShell("demo/web");
    const again = useTerminalStore.getState().ensureShell("demo/web");
    expect(again).toBe(first);
    expect(useTerminalStore.getState().tabs).toHaveLength(1);
  });

  it("adds numbered shell tabs and activates the new one", () => {
    useTerminalStore.getState().ensureShell("demo/web");
    const second = useTerminalStore.getState().addShellTab("demo/web");
    const { tabs, activeByVm } = useTerminalStore.getState();
    expect(tabs.map((tab) => tab.title)).toEqual(["1", "2"]);
    expect(activeByVm["demo/web"]).toBe(second);
  });

  it("keeps a replacement tab when the last shell tab closes", () => {
    const first = useTerminalStore.getState().ensureShell("demo/web");
    useTerminalStore.getState().closeTab(first);
    const { tabs } = useTerminalStore.getState();
    expect(tabs).toHaveLength(1);
    expect(tabs[0].id).not.toBe(first);
    expect(tabs[0].title).toBe("1");
  });

  it("caps shell tabs per VM", () => {
    useTerminalStore.getState().ensureShell("demo/web");
    for (let i = 1; i < MAX_SHELL_TABS; i += 1) {
      expect(useTerminalStore.getState().addShellTab("demo/web")).not.toBeNull();
    }
    expect(useTerminalStore.getState().addShellTab("demo/web")).toBeNull();
    expect(
      useTerminalStore.getState().tabs.filter((tab) => tab.kind === "shell"),
    ).toHaveLength(MAX_SHELL_TABS);
  });

  it("keeps console sessions independent of shell tabs", () => {
    useTerminalStore.getState().ensureShell("demo/web");
    const consoleId = useTerminalStore.getState().ensureConsole("demo/web");
    expect(useTerminalStore.getState().ensureConsole("demo/web")).toBe(
      consoleId,
    );
    expect(useTerminalStore.getState().tabs).toHaveLength(2);
  });

  it("exposes a stable tabs snapshot between unused reads", () => {
    const first = useTerminalStore.getState().tabs;
    const second = useTerminalStore.getState().tabs;
    expect(second).toBe(first);
  });
});
