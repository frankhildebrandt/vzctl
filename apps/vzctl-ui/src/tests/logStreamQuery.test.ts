import { describe, expect, it } from "vitest";
import {
  serializeFilterQueryKey,
  serializeSelectFilterKey,
  serializeTextFilterKey,
  type GuestLogFilters,
} from "@/lib/logStreamQuery";

const baseFilters: GuestLogFilters = {
  q: "",
  minLevel: "all",
  groupField: "component",
  groupValue: "",
  fieldFilters: {},
};

describe("serializeSelectFilterKey", () => {
  it("tracks select-based filters independently of text", () => {
    const a = serializeSelectFilterKey({
      ...baseFilters,
      groupValue: "api",
      minLevel: "warn",
    });
    const b = serializeSelectFilterKey({
      ...baseFilters,
      q: "ignored",
      fieldFilters: { component: "api" },
      groupValue: "api",
      minLevel: "warn",
    });
    expect(a).toBe(b);
  });
});

describe("serializeTextFilterKey", () => {
  it("serializes q and field filters with stable key order", () => {
    expect(
      serializeTextFilterKey({
        ...baseFilters,
        q: "heartbeat",
        fieldFilters: { component: "api", msg: "fail" },
      }),
    ).toBe(
      serializeTextFilterKey({
        ...baseFilters,
        q: "heartbeat",
        fieldFilters: { msg: "fail", component: "api" },
      }),
    );
  });

  it("omits empty field filter values", () => {
    expect(
      serializeTextFilterKey({
        ...baseFilters,
        fieldFilters: { component: "", msg: "fail" },
      }),
    ).toBe(
      serializeTextFilterKey({
        ...baseFilters,
        fieldFilters: { msg: "fail" },
      }),
    );
  });
});

describe("serializeFilterQueryKey", () => {
  it("combines select and text keys", () => {
    const filters: GuestLogFilters = {
      q: "err",
      minLevel: "warn",
      groupField: "component",
      groupValue: "api",
      fieldFilters: { component: "api" },
    };
    expect(serializeFilterQueryKey(filters)).toBe(
      `${serializeSelectFilterKey(filters)}\0${serializeTextFilterKey(filters)}`,
    );
  });
});
