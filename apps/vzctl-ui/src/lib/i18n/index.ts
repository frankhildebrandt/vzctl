export type { LocaleId, MessageParams } from "./types";
export type { MessageKey } from "./messages";
export {
  LOCALES,
  LOCALE_OPTIONS,
} from "./types";
export {
  applyDocumentLocale,
  detectSystemLocale,
  localeToBcp47,
  localeToHtmlLang,
} from "./detect";
export { catalogs } from "./messages";
export { createTranslator, translate } from "./t";
export { getT, useT, type TFunction } from "./useT";
