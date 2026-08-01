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
};

export const THEME_OPTIONS: Array<{
  id: ThemeId;
  label: string;
  description: string;
}> = [
  {
    id: "light",
    label: "Light",
    description: "Helles Papier mit Teal-Akzenten",
  },
  {
    id: "dark",
    label: "Dark",
    description: "Dunkle Oberfläche, ruhig für lange Sessions",
  },
  {
    id: "crt",
    label: "CRT Hacker",
    description: "Phosphorgrün, Scanlines, Terminal-Feel",
  },
  {
    id: "hot-pink",
    label: "Hot Pink",
    description: "Dunkler Grund, knalliges Pink",
  },
  {
    id: "business-gray",
    label: "Business Gray",
    description: "Nüchternes Grau, clean und seriös",
  },
];

export const DEFAULT_SETTINGS: AppSettings = {
  theme: "light",
};

const LIGHT_THEMES: ReadonlySet<ThemeId> = new Set(["light", "business-gray"]);

export function isThemeId(value: unknown): value is ThemeId {
  return typeof value === "string" && (THEMES as readonly string[]).includes(value);
}

export function loadSettings(): AppSettings {
  const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
  if (!raw) return { ...DEFAULT_SETTINGS };
  try {
    const parsed = JSON.parse(raw) as unknown;
    return normalizeSettings(parsed);
  } catch {
    return { ...DEFAULT_SETTINGS };
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
  if (!value || typeof value !== "object") return { ...DEFAULT_SETTINGS };
  const record = value as Record<string, unknown>;
  return {
    theme: isThemeId(record.theme) ? record.theme : DEFAULT_SETTINGS.theme,
  };
}
