import { applyDocumentLocale, detectSystemLocale } from "@/lib/i18n/detect";
import { LOCALES, type LocaleId } from "@/lib/i18n/types";

export const SETTINGS_STORAGE_KEY = "vzctl.ui.settings.v1";

export const THEMES = [
  "light",
  "dark",
  "crt",
  "hot-pink",
  "business-gray",
] as const;

export type ThemeId = (typeof THEMES)[number];

export type AppSettings = {
  theme: ThemeId;
  locale: LocaleId;
};

export const THEME_IDS = THEMES;

export type ThemeOptionId = ThemeId;

/** Theme option metadata without translated strings — labels via i18n keys. */
export const THEME_OPTION_IDS: ThemeId[] = [...THEMES];

export const THEME_LABEL_KEYS = {
  light: "settings.theme.light.label",
  dark: "settings.theme.dark.label",
  crt: "settings.theme.crt.label",
  "hot-pink": "settings.theme.hotPink.label",
  "business-gray": "settings.theme.businessGray.label",
} as const;

export const THEME_DESCRIPTION_KEYS = {
  light: "settings.theme.light.description",
  dark: "settings.theme.dark.description",
  crt: "settings.theme.crt.description",
  "hot-pink": "settings.theme.hotPink.description",
  "business-gray": "settings.theme.businessGray.description",
} as const;

export const DEFAULT_SETTINGS: AppSettings = {
  theme: "light",
  locale: detectSystemLocale(),
};

const LIGHT_THEMES: ReadonlySet<ThemeId> = new Set(["light", "business-gray"]);

export function isThemeId(value: unknown): value is ThemeId {
  return typeof value === "string" && (THEMES as readonly string[]).includes(value);
}

export function isLocaleId(value: unknown): value is LocaleId {
  return typeof value === "string" && (LOCALES as readonly string[]).includes(value);
}

export function loadSettings(): AppSettings {
  const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
  if (!raw) return { ...DEFAULT_SETTINGS, locale: detectSystemLocale() };
  try {
    const parsed = JSON.parse(raw) as unknown;
    return normalizeSettings(parsed);
  } catch {
    return { ...DEFAULT_SETTINGS, locale: detectSystemLocale() };
  }
}

export function saveSettings(settings: AppSettings): void {
  localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(settings));
}

export function applyTheme(theme: ThemeId): void {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = LIGHT_THEMES.has(theme)
    ? "light"
    : "dark";
}

export function normalizeSettings(value: unknown): AppSettings {
  if (!value || typeof value !== "object") {
    return { ...DEFAULT_SETTINGS, locale: detectSystemLocale() };
  }
  const record = value as Record<string, unknown>;
  return {
    theme: isThemeId(record.theme) ? record.theme : DEFAULT_SETTINGS.theme,
    locale: isLocaleId(record.locale) ? record.locale : detectSystemLocale(),
  };
}

export function applyLocale(locale: LocaleId): void {
  applyDocumentLocale(locale);
}
