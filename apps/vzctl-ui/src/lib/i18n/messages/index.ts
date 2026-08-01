import type { LocaleId } from "../types";
import { deDE, type MessageKey } from "./deDE";
import { enUS } from "./enUS";

export type { MessageKey };
export const catalogs: Record<LocaleId, Record<MessageKey, string>> = {
  deDE,
  enUS,
};
