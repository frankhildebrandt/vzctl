export const LOCALES = ["deDE", "enUS"] as const;

export type LocaleId = (typeof LOCALES)[number];

export const LOCALE_OPTIONS: Array<{
  id: LocaleId;
  label: string;
  description: string;
}> = [
  {
    id: "deDE",
    label: "Deutsch",
    description: "de-DE",
  },
  {
    id: "enUS",
    label: "English",
    description: "en-US",
  },
];

export type MessageParams = Record<string, string | number>;

/** Flat message catalog — both locales must implement the same keys. */
export type MessageCatalog = Record<string, string>;
