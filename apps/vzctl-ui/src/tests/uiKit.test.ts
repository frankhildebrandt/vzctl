import { describe, expect, it } from "vitest";
import { cx } from "@/components/ui/cx";

describe("ui/cx", () => {
  it("joins truthy class names", () => {
    expect(cx("a", "b")).toBe("a b");
  });

  it("skips falsy values", () => {
    expect(cx("card", false && "error-card", null, undefined, "ok")).toBe(
      "card ok",
    );
  });

  it("supports conditional tones", () => {
    const tone: "danger" | "ok" = "danger";
    expect(cx("badge", tone === "danger" && "danger")).toBe("badge danger");
  });
});

describe("ui kit design tokens (class contracts)", () => {
  it("keeps badge tone class names stable", () => {
    const toneClass = {
      neutral: undefined,
      ok: "ok",
      warn: "warn",
      danger: "danger",
    } as const;
    expect(cx("badge", toneClass.ok)).toBe("badge ok");
    expect(cx("badge", toneClass.neutral)).toBe("badge");
  });

  it("keeps button tone class names stable", () => {
    const toneClass = {
      primary: undefined,
      secondary: "secondary",
      danger: "danger",
      quiet: "secondary",
    } as const;
    expect(cx(toneClass.primary)).toBe("");
    expect(cx(toneClass.secondary)).toBe("secondary");
    expect(cx(toneClass.danger)).toBe("danger");
  });

  it("keeps card tone class names stable", () => {
    expect(cx("card", true && "error-card")).toBe("card error-card");
    expect(cx("card", true && "summary-card")).toBe("card summary-card");
  });

  it("keeps status pill class names stable", () => {
    const state = "running";
    expect(cx("vm-state", `state-${state}`)).toBe("vm-state state-running");
    expect(cx("stack-pill", "phase-failed")).toBe("stack-pill phase-failed");
  });
});
