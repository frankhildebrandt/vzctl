import type { LocaleId } from "./types";

/** Map host/browser language to a supported UI locale. */
export function detectSystemLocale(
  language = typeof navigator !== "undefined" ? navigator.language : "en",
): LocaleId {
  const primary = language.trim().toLowerCase().split("-")[0] ?? "en";
  return primary === "de" ? "deDE" : "enUS";
}

export function localeToHtmlLang(locale: LocaleId): string {
  return locale === "deDE" ? "de" : "en";
}

export function localeToBcp47(locale: LocaleId): string {
  return locale === "deDE" ? "de-DE" : "en-US";
}

export function applyDocumentLocale(locale: LocaleId): void {
  if (typeof document === "undefined") return;
  document.documentElement.lang = localeToHtmlLang(locale);
}
