import { translate, type MessageKey, type MessageParams } from "@/lib/i18n";
import { useSettingsStore } from "@/store/settingsStore";

export type TFunction = (
  key: MessageKey,
  params?: MessageParams,
) => string;

export function useT(): TFunction {
  const locale = useSettingsStore((s) => s.locale);
  return (key: MessageKey, params?: MessageParams) =>
    translate(locale, key, params);
}

/** Read current locale translator outside React (event handlers, helpers). */
export function getT(): TFunction {
  const locale = useSettingsStore.getState().locale;
  return (key, params) => translate(locale, key, params);
}
