import { beforeEach, describe, expect, it, vi } from "vitest";
import { serializeEnvironmentYaml } from "@/application/services/yaml";
import { ApiError } from "@/lib/api";
import { scaffoldFiles } from "@/application/services/scaffold";

const mocks = vi.hoisted(() => ({
  demo: false,
  getText: vi.fn(),
  putText: vi.fn(),
  post: vi.fn(),
}));
vi.mock("@/lib/demo", () => ({ isDemoMode: () => mocks.demo }));
vi.mock("@/lib/api", () => ({
  api: { getText: mocks.getText, putText: mocks.putText, post: mocks.post },
  encodeId: encodeURIComponent,
  ApiError: class extends Error { constructor(public status: number, public code: string, message: string) { super(message); } },
}));
vi.mock("@/lib/vzctl", () => ({ ensureStackId: async () => "test" }));
import { createProjectFlexible, loadProjectFlexible, saveProjectFlexible } from "@/features/persistence/projectIo";

beforeEach(() => {
  mocks.demo = false;
  vi.resetAllMocks();
  const memory = new Map<string, string>();
  vi.stubGlobal("sessionStorage", {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => memory.set(key, value),
  });
});

describe("project persistence", () => {
  it("propagates config write failure instead of reporting a memory-only save", async () => {
    const { env, diagram } = scaffoldFiles({ name: "test" });
    mocks.putText.mockRejectedValueOnce(new Error("Permission denied"));
    await expect(saveProjectFlexible("/test", env, diagram)).rejects.toThrow("Permission denied");
    expect(mocks.putText).toHaveBeenCalledTimes(1);
    expect(sessionStorage.getItem("vzctl.ui.topology.memory.v1")).toBeNull();
  });

  it("propagates diagram write failure after config was saved", async () => {
    const { env, diagram } = scaffoldFiles({ name: "test" });
    mocks.putText.mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("disk full"));
    await expect(saveProjectFlexible("/test", env, diagram)).rejects.toThrow("disk full");
    expect(mocks.putText).toHaveBeenCalledTimes(2);
  });

  it("does not invent a new project after a failed create", async () => {
    mocks.post.mockRejectedValueOnce(new Error("already exists"));
    await expect(createProjectFlexible("/test", "test")).rejects.toThrow("already exists");
    expect(mocks.putText).not.toHaveBeenCalled();
    expect(sessionStorage.getItem("vzctl.ui.topology.memory.v1")).toBeNull();
  });

  it("does not mask a real load error with an old demo snapshot", async () => {
    mocks.demo = true;
    await createProjectFlexible("/test", "test");
    mocks.demo = false;
    mocks.getText.mockRejectedValueOnce(new Error("unavailable"));
    await expect(loadProjectFlexible("/test")).rejects.toThrow("unavailable");
  });

  it("does not turn a diagram read failure into an empty layout", async () => {
    const { env } = scaffoldFiles({ name: "test" });
    mocks.getText.mockResolvedValueOnce(serializeEnvironmentYaml(env)).mockRejectedValueOnce(new Error("unavailable"));
    await expect(loadProjectFlexible("/test")).rejects.toThrow("unavailable");
  });

  it("allows a missing optional diagram", async () => {
    const { env } = scaffoldFiles({ name: "test" });
    mocks.getText.mockResolvedValueOnce(serializeEnvironmentYaml(env)).mockRejectedValueOnce(new ApiError(404, "not_found", "missing"));
    expect((await loadProjectFlexible("/test")).diagram.nodes).toEqual({});
  });

  it("accepts only the exact legacy missing-sidecar error", async () => {
    const { env } = scaffoldFiles({ name: "test" });
    mocks.getText.mockResolvedValueOnce(serializeEnvironmentYaml(env)).mockRejectedValueOnce(
      new ApiError(500, "internal", "not found: /test/.vzctl/diagram.json"),
    );
    expect((await loadProjectFlexible("/test")).diagram.nodes).toEqual({});
    mocks.getText.mockResolvedValueOnce(serializeEnvironmentYaml(env)).mockRejectedValueOnce(
      new ApiError(500, "internal", "not found: /another/file"),
    );
    await expect(loadProjectFlexible("/test")).rejects.toThrow("not found: /another/file");
  });

  it("keeps explicit demo create/save/load entirely in memory", async () => {
    mocks.demo = true;
    const { env, diagram } = await createProjectFlexible("/test", "test");
    env.spec.observability = { probes: [{ name: "web", from: "web", target: "https://web.test", expect: "http2xx" }] };
    await saveProjectFlexible("/test", env, diagram);
    expect((await loadProjectFlexible("/test")).env).toEqual(env);
    expect(mocks.post).not.toHaveBeenCalled();
    expect(mocks.putText).not.toHaveBeenCalled();
    expect(mocks.getText).not.toHaveBeenCalled();
  });
});
