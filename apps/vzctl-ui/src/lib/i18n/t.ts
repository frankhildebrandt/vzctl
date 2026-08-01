import { catalogs, type MessageKey } from "./messages";
import type { LocaleId, MessageParams } from "./types";

const MISSING = (key: string) => `[missing:${key}]`;

export function translate(
  locale: LocaleId,
  key: MessageKey,
  params?: MessageParams,
): string {
  const template = catalogs[locale][key] ?? catalogs.enUS[key] ?? MISSING(key);
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

/** Non-React helper using a fixed locale (e.g. stores, pure helpers). */
export function createTranslator(locale: LocaleId) {
  return (key: MessageKey, params?: MessageParams) =>
    translate(locale, key, params);
}
