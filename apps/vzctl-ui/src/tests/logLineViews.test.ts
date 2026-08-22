import { describe, expect, it } from "vitest";
import type { IwatchLine } from "@/lib/guestLogs";
import {
  formatLogLineView,
  hiddenFieldsKey,
} from "@/lib/logLineViews";

const line: IwatchLine = {
  index: 3,
  session: 9,
  source: "stdout",
  ts: "2026-08-01T12:34:56.789Z",
  level: "warn",
  text: "msg=slow component=api",
  fields: { component: "api", msg: "slow" },
};

describe("formatLogLineView", () => {
  it("precomputes stable row fields", () => {
    const view = formatLogLineView(line, ["component", "msg"], { raw: true });
    expect(view.key).toBe("9-3");
    expect(view.ts).toBe("12:34:56");
    expect(view.levelClass).toBe("warn");
    expect(view.bodyHtml).toContain("component");
  });
});

describe("hiddenFieldsKey", () => {
  it("is order independent", () => {
    expect(hiddenFieldsKey({ raw: true, source: true })).toBe(
      hiddenFieldsKey({ source: true, raw: true }),
    );
  });
});
