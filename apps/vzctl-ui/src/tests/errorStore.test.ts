import { afterEach, describe, expect, it } from "vitest";
import { ApiError } from "@/lib/api";
import { createTranslator } from "@/lib/i18n";
import {
  formatAllErrorsForClipboard,
  formatErrorForClipboard,
  reportError,
  useErrorStore,
} from "@/store/errorStore";
import { useSettingsStore } from "@/store/settingsStore";

const t = createTranslator("deDE");

afterEach(() => {
  useErrorStore.getState().clear();
  useSettingsStore.setState({ locale: "deDE" });
});

describe("errorStore", () => {
  it("reports ApiError with request meta", () => {
    const err = new ApiError(503, "unavailable", "control plane down", { reason: "sock" }, {
      method: "GET",
      path: "/v1/vms",
    });
    const entry = reportError(err, { source: "query", queryKey: ["vzctl", "vms"] });
    expect(entry).not.toBeNull();
    expect(entry!.message).toBe("control plane down");
    expect(entry!.code).toBe("unavailable");
    expect(entry!.status).toBe(503);
    expect(entry!.method).toBe("GET");
    expect(entry!.path).toBe("/v1/vms");
    expect(entry!.queryKey).toBe(JSON.stringify(["vzctl", "vms"]));
    expect(useErrorStore.getState().errors).toHaveLength(1);
  });

  it("dedupes identical errors within 2s", () => {
    const err = new Error("boom");
    expect(reportError(err, { source: "ui" })).not.toBeNull();
    expect(reportError(err, { source: "ui" })).toBeNull();
    expect(useErrorStore.getState().errors).toHaveLength(1);
  });

  it("formats clipboard text with context", () => {
    const err = new ApiError(404, "not_found", "missing", { id: "x" }, {
      method: "GET",
      path: "/v1/vms/x",
    });
    const entry = reportError(err, { source: "mutation", mutationKey: ["delete"] })!;
    const text = formatErrorForClipboard(entry);
    expect(text).toContain(`${t("errors.clipboard.message")}: missing`);
    expect(text).toContain(`${t("errors.clipboard.code")}: not_found`);
    expect(text).toContain(`${t("errors.clipboard.status")}: 404`);
    expect(text).toContain(`${t("errors.clipboard.request")}: GET /v1/vms/x`);
    expect(text).toContain('"id": "x"');
    expect(text).toContain(
      `${t("errors.clipboard.source")}: ${t("errors.source.mutation")}`,
    );
  });

  it("formats all errors", () => {
    reportError(new Error("a"), { source: "ui" });
    // Force different message so no dedupe
    reportError(new Error("b"), { source: "api" });
    const text = formatAllErrorsForClipboard(useErrorStore.getState().errors);
    expect(text).toContain(t("errors.clipboard.separator", { i: 1, n: 2 }));
    expect(text).toContain(`${t("errors.clipboard.message")}: b`);
    expect(text).toContain(`${t("errors.clipboard.message")}: a`);
  });

  it("clears history", () => {
    reportError(new Error("x"), { source: "ui" });
    useErrorStore.getState().clear();
    expect(useErrorStore.getState().errors).toHaveLength(0);
  });
});
