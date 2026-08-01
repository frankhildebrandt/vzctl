import { describe, expect, it } from "vitest";
import {
  detectSystemLocale,
  localeToBcp47,
  localeToHtmlLang,
  translate,
} from "@/lib/i18n";
import { normalizeSettings } from "@/lib/settings";

describe("i18n", () => {
  it("detects German system locales as deDE", () => {
    expect(detectSystemLocale("de")).toBe("deDE");
    expect(detectSystemLocale("de-DE")).toBe("deDE");
    expect(detectSystemLocale("de-AT")).toBe("deDE");
  });

  it("falls back to enUS for non-German locales", () => {
    expect(detectSystemLocale("en")).toBe("enUS");
    expect(detectSystemLocale("en-US")).toBe("enUS");
    expect(detectSystemLocale("fr-FR")).toBe("enUS");
  });

  it("maps locale to html lang and BCP-47", () => {
    expect(localeToHtmlLang("deDE")).toBe("de");
    expect(localeToHtmlLang("enUS")).toBe("en");
    expect(localeToBcp47("deDE")).toBe("de-DE");
    expect(localeToBcp47("enUS")).toBe("en-US");
  });

  it("translates keys with interpolation in both locales", () => {
    expect(translate("deDE", "dashboard.runningCount", { n: 3 })).toBe(
      "(3 aktiv)",
    );
    expect(translate("enUS", "nav.errors")).toBe("Errors");
    expect(translate("deDE", "nav.errors")).toBe("Fehler");
    expect(translate("enUS", "dialog.busy", { label: "Delete" })).toBe(
      "Delete…",
    );
  });

  it("normalizes persisted locale in settings", () => {
    expect(normalizeSettings({ theme: "dark", locale: "enUS" }).locale).toBe(
      "enUS",
    );
    expect(normalizeSettings({ theme: "dark", locale: "xx" }).locale).toMatch(
      /^(deDE|enUS)$/,
    );
  });
});
