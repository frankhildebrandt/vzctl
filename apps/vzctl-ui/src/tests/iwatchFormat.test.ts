import { describe, expect, it } from "vitest";
import type { IwatchLine } from "@/lib/guestLogs";
import {
  formatBodyHtml,
  levelClass,
  visibleColumns,
  visibleFieldPairs,
} from "@/lib/iwatchFormat";

const line: IwatchLine = {
  index: 1,
  source: "stdout",
  ts: "2026-08-01T12:34:56.789Z",
  level: "WARN",
  text: 'msg="slow query" component=api timeout=5s',
  fields: {
    component: "api",
    msg: "slow query",
    timeout: "5s",
  },
};

describe("levelClass", () => {
  it("maps warn and error levels", () => {
    expect(levelClass("WARN")).toBe("warn");
    expect(levelClass("error")).toBe("error");
    expect(levelClass("info")).toBe("info");
  });
});

describe("visibleFieldPairs", () => {
  it("skips hidden and empty fields", () => {
    expect(
      visibleFieldPairs(line, ["component", "msg", "timeout"], {
        raw: true,
        timeout: true,
      }),
    ).toEqual([
      { key: "component", value: "api" },
      { key: "msg", value: "slow query" },
    ]);
  });
});

describe("formatBodyHtml", () => {
  it("renders structured fields when raw is hidden", () => {
    const html = formatBodyHtml(line, ["component", "msg"], { raw: true });
    expect(html).toContain('class="vm-logs-k">component</span>');
    expect(html).toContain("slow query");
  });

  it("renders raw text when raw is visible", () => {
    const html = formatBodyHtml(line, ["component", "msg"], {});
    expect(html).toContain("slow query");
    expect(html).not.toContain("vm-logs-k");
  });
});

describe("visibleColumns", () => {
  it("builds grid columns with trimmed timestamp", () => {
    const cols = visibleColumns(line, ["component", "msg"], { raw: true });
    expect(cols.ts).toBe("12:34:56");
    expect(cols.source).toBe("stdout");
    expect(cols.levelClass).toBe("warn");
  });
});
