import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/demo", () => ({ isDemoMode: () => false }));
import { api, ApiError } from "@/lib/api";

beforeEach(() => vi.resetAllMocks());
describe("text API error contract", () => {
  it("preserves structured server errors for optional sidecars", async () => {
    mocks.invoke.mockResolvedValue({ status: 500, body: JSON.stringify({
      error: { code: "internal", message: "not found: /test/.vzctl/diagram.json" },
    }) });
    await expect(api.getText("/v1/stacks/test/diagram")).rejects.toMatchObject({
      status: 500, code: "internal", message: "not found: /test/.vzctl/diagram.json",
      request: { method: "GET", path: "/v1/stacks/test/diagram" },
    });
  });
  it("preserves plain errors and distinguishes transport failure", async () => {
    mocks.invoke.mockResolvedValueOnce({ status: 503, body: "temporarily unavailable" });
    await expect(api.getText("/config")).rejects.toMatchObject({ status: 503, message: "temporarily unavailable" });
    mocks.invoke.mockRejectedValueOnce("socket unavailable");
    await expect(api.getText("/config")).rejects.toBeInstanceOf(ApiError);
  });
});
