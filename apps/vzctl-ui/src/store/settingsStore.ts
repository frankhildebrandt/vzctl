import { create } from "zustand";
import {
  applyTheme,
  loadSettings,
  saveSettings,
  type AppSettings,
  type ThemeId,
} from "@/lib/settings";

type SettingsStore = AppSettings & {
  setTheme: (theme: ThemeId) => void;
};

const initial = loadSettings();
applyTheme(initial.theme);

function persist(partial: Partial<AppSettings>, get: () => SettingsStore): void {
  const { theme } = { ...get(), ...partial };
  saveSettings({ theme });
}

export const useSettingsStore = create<SettingsStore>((set, get) => ({
  theme: initial.theme,

  setTheme: (theme) => {
    if (get().theme === theme) return;
    applyTheme(theme);
    set({ theme });
    persist({ theme }, get);
  },
}));
