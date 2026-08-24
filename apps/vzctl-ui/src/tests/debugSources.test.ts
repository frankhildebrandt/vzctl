import { describe, expect, it } from "vitest";
import { debugSourcesPath } from "@/lib/debugSources";

describe("debugSources", () => {
  it("builds REST path for stack id", () => {
    expect(debugSourcesPath("edge-dmz")).toBe(
      "/v1/stacks/edge-dmz/debug/sources",
    );
  });

  it("encodes slash in stack id", () => {
    expect(debugSourcesPath("lab/stack")).toContain("lab%2Fstack");
  });
});
