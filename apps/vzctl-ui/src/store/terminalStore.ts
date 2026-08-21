import { create } from "zustand";

export const MAX_SHELL_TABS = 8;

export type TerminalKind = "shell" | "console";

export type TerminalTab = {
  id: string;
  vmId: string;
  kind: TerminalKind;
  title: string;
  cmd: string[];
};

type TerminalStore = {
  tabs: TerminalTab[];
  activeByVm: Record<string, string>;
  ensureShell: (vmId: string) => string;
  ensureConsole: (vmId: string) => string;
  addShellTab: (vmId: string) => string | null;
  closeTab: (id: string) => void;
  setActive: (vmId: string, tabId: string) => void;
};

let seq = 0;

function nextId(kind: TerminalKind): string {
  seq += 1;
  return `${kind}-${Date.now()}-${seq}`;
}

function shellTabs(tabs: TerminalTab[], vmId: string): TerminalTab[] {
  return tabs.filter((tab) => tab.vmId === vmId && tab.kind === "shell");
}

function consoleTab(tabs: TerminalTab[], vmId: string): TerminalTab | undefined {
  return tabs.find((tab) => tab.vmId === vmId && tab.kind === "console");
}

function nextShellTitle(tabs: TerminalTab[], vmId: string): string {
  const used = new Set(shellTabs(tabs, vmId).map((tab) => tab.title));
  let n = 1;
  while (used.has(String(n))) n += 1;
  return String(n);
}

export const useTerminalStore = create<TerminalStore>((set, get) => ({
  tabs: [],
  activeByVm: {},

  ensureShell: (vmId) => {
    const existing = shellTabs(get().tabs, vmId);
    if (existing.length > 0) {
      const active = get().activeByVm[vmId];
      if (active && existing.some((tab) => tab.id === active)) return active;
      set((state) => ({
        activeByVm: { ...state.activeByVm, [vmId]: existing[0].id },
      }));
      return existing[0].id;
    }
    const tab: TerminalTab = {
      id: nextId("shell"),
      vmId,
      kind: "shell",
      title: "1",
      cmd: ["/bin/bash"],
    };
    set((state) => ({
      tabs: [...state.tabs, tab],
      activeByVm: { ...state.activeByVm, [vmId]: tab.id },
    }));
    return tab.id;
  },

  ensureConsole: (vmId) => {
    const existing = consoleTab(get().tabs, vmId);
    if (existing) return existing.id;
    const tab: TerminalTab = {
      id: nextId("console"),
      vmId,
      kind: "console",
      title: "console",
      cmd: [],
    };
    set((state) => ({ tabs: [...state.tabs, tab] }));
    return tab.id;
  },

  addShellTab: (vmId) => {
    const current = shellTabs(get().tabs, vmId);
    if (current.length >= MAX_SHELL_TABS) return null;
    const tab: TerminalTab = {
      id: nextId("shell"),
      vmId,
      kind: "shell",
      title: nextShellTitle(get().tabs, vmId),
      cmd: ["/bin/bash"],
    };
    set((state) => ({
      tabs: [...state.tabs, tab],
      activeByVm: { ...state.activeByVm, [vmId]: tab.id },
    }));
    return tab.id;
  },

  closeTab: (id) => {
    const tab = get().tabs.find((entry) => entry.id === id);
    if (!tab || tab.kind !== "shell") return;
    const remaining = shellTabs(get().tabs, tab.vmId).filter(
      (entry) => entry.id !== id,
    );
    const fallback =
      remaining[0] ??
      ({
        id: nextId("shell"),
        vmId: tab.vmId,
        kind: "shell" as const,
        title: "1",
        cmd: ["/bin/bash"],
      } satisfies TerminalTab);
    const nextTabs = get()
      .tabs.filter((entry) => entry.id !== id)
      .concat(remaining.length === 0 ? [fallback] : []);
    set((state) => ({
      tabs: nextTabs,
      activeByVm: {
        ...state.activeByVm,
        [tab.vmId]:
          state.activeByVm[tab.vmId] === id
            ? fallback.id
            : state.activeByVm[tab.vmId],
      },
    }));
  },

  setActive: (vmId, tabId) => {
    const tab = get().tabs.find((entry) => entry.id === tabId);
    if (!tab || tab.vmId !== vmId || tab.kind !== "shell") return;
    set((state) => ({
      activeByVm: { ...state.activeByVm, [vmId]: tabId },
    }));
  },
}));
