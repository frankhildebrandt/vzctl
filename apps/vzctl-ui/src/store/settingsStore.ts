import { create } from "zustand";
import type { LocaleId } from "@/lib/i18n";
import {
  applyLocale,
  applyTheme,
  loadSettings,
  saveSettings,
  type AppSettings,
  type ThemeId,
} from "@/lib/settings";

type SettingsStore = AppSettings & {
  setTheme: (theme: ThemeId) => void;
  setLocale: (locale: LocaleId) => void;
};

const initial = loadSettings();
applyTheme(initial.theme);
applyLocale(initial.locale);

function persist(partial: Partial<AppSettings>, get: () => SettingsStore): void {
  const next = { ...get(), ...partial };
  saveSettings({ theme: next.theme, locale: next.locale });
}

export const useSettingsStore = create<SettingsStore>((set, get) => ({
  theme: initial.theme,
  locale: initial.locale,

  setTheme: (theme) => {
    if (get().theme === theme) return;
    applyTheme(theme);
    set({ theme });
    persist({ theme }, get);
  },

  setLocale: (locale) => {
    if (get().locale === locale) return;
    applyLocale(locale);
    set({ locale });
    persist({ locale }, get);
  },
}));
