import { beforeEach, describe, expect, it, vi } from "vitest";
import { parseApplyRecovery } from "@/lib/applyRecovery";
const api = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }));
vi.mock("@/lib/api", () => ({ api, encodeId: encodeURIComponent }));
vi.mock("@/lib/demo", () => ({ isDemoMode: () => false }));
import { runVzctl } from "@/lib/vzctl";

beforeEach(() => { vi.resetAllMocks(); });

describe("explicit apply recovery", () => {
  it.each([
    "incomplete journal abc-123 at await_agents; run `vzctl apply --resume` or `vzctl apply --abort`",
    "stack lease held by localhost:1234 until 2026-10-20T12:00:00Z",
  ])("does not abort or retry automatically for %s", async (message) => {
    api.get.mockResolvedValueOnce({ stacks: [{ id: "stack", path: "/stack" }] })
      .mockResolvedValueOnce({ status: "failed", result: { status: "fail", exit_code: 5, summary: { message } } });
    api.post.mockResolvedValueOnce({ jobId: "job" });
    await expect(runVzctl("/stack", "up")).rejects.toThrow(message);
    expect(api.post).toHaveBeenCalledTimes(1);
    expect(api.post).toHaveBeenCalledWith("/v1/stacks/stack/up", {});
  });
  it("extracts holder, PID and expiry without treating a conflict as an incomplete journal", () => {
    expect(parseApplyRecovery("Error: stack lease held by localhost:1234 until 2026-10-20T12:00:00Z"))
      .toEqual({ kind: "lease", holder: "localhost:1234", pid: 1234, expiresAt: "2026-10-20T12:00:00Z" });
  });
  it("extracts journal/step and rejects false-positive recovery hints", () => {
    expect(parseApplyRecovery("incomplete journal abc-123 at await_agents; run `vzctl apply --resume` or `vzctl apply --abort`"))
      .toEqual({ kind: "journal", journalId: "abc-123", step: "await_agents" });
    expect(parseApplyRecovery("no incomplete journal")).toBeNull();
    expect(parseApplyRecovery("help: apply --resume or --abort")).toBeNull();
    expect(parseApplyRecovery("desired config changed; run `vzctl apply --abort` before applying")?.kind).toBe("config-changed");
  });
});
