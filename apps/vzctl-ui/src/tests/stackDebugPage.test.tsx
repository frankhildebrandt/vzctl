// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StackDebugPage } from "@/components/stack-debug/StackDebugPage";
import { useSettingsStore } from "@/store/settingsStore";
const get = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api", () => ({ api: { get }, encodeId: encodeURIComponent }));
let root: Root;
let host: HTMLDivElement;
let client: QueryClient;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  useSettingsStore.getState().setLocale("enUS");
  localStorage.setItem("vzctl.ui.projects.v1", JSON.stringify([{ path: "/fixture", name: "fixture", id: "fixture", openedAt: 1 }]));
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  get.mockReset();
});
afterEach(async () => { await act(async () => root.unmount()); client.clear(); host.remove(); localStorage.clear(); });
async function render() {
  await act(async () => {
    root.render(<QueryClientProvider client={client}><StackDebugPage stackPath="/fixture" /></QueryClientProvider>);
  });
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
}
it("shows both CLI failures returned with HTTP 200 instead of silently empty sources", async () => {
  get.mockImplementation(async (path: string) => ({ status: "fail", exit_code: 2,
    summary: { message: path.endsWith("/doctor") ? "doctor command usage failure" : "sources command usage failure" } }));
  await render();
  expect(host.textContent).toContain("sources command usage failure");
  expect(host.textContent).toContain("doctor command usage failure");
});
it("shows a doctor transport failure alongside usable sources", async () => {
  get.mockImplementation(async (path: string) => {
    if (path.endsWith("/doctor")) throw new Error("doctor connection unavailable");
    return { status: "ok", exit_code: 0, sources: { dataSources: { postgres: { default: "db", via: "vm", entries: ["db"] } } } };
  });
  await render();
  expect(host.querySelector("option")?.textContent).toContain("db");
  expect(host.textContent).toContain("doctor connection unavailable");
});
it("keeps successful doctor reports with missing prerequisites visible", async () => {
  get.mockImplementation(async (path: string) => path.endsWith("/doctor")
    ? { status: "ok", exit_code: 0, doctor: { ok: false, checks: [{ type: "postgres", status: "fail", message: "psql is missing in the guest" }] } }
    : { status: "ok", exit_code: 0, sources: { dataSources: {} } });
  await render();
  expect(host.textContent).toContain("psql is missing in the guest");
});
it("shows progress and successful tool checks when the user rechecks prerequisites", async () => {
  const report = { status: "ok", exit_code: 0, doctor: { ok: true, checks: [
    { type: "postgres", status: "ok", tool: "psql", vmId: "fixture/db" },
  ] } };
  get.mockImplementation(async (path: string) => path.endsWith("/doctor")
    ? report : { status: "ok", exit_code: 0, sources: { dataSources: {} } });
  await render();
  expect(host.textContent).toContain("Debug prerequisites ready");
  expect(host.textContent).toContain("psql");
  expect(host.textContent).toContain("fixture/db");
  let finish!: (value: typeof report) => void;
  get.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const button = [...host.querySelectorAll("button")].find((item) => item.textContent === "Check prerequisites")!;
  await act(async () => { button.click(); await new Promise((resolve) => setTimeout(resolve, 20)); });
  expect(button.disabled).toBe(true);
  expect(host.querySelector('[role="status"]')?.textContent).toContain("Checking prerequisites…");
  expect(host.textContent).not.toContain("Debug prerequisites ready");
  await act(async () => { finish(report); await new Promise((resolve) => setTimeout(resolve, 20)); });
  expect(button.disabled).toBe(false);
  expect(host.querySelector('[role="status"]')?.textContent).toContain("Debug prerequisites ready");
  get.mockRejectedValueOnce(new Error("recheck connection failed"));
  await act(async () => { button.click(); await new Promise((resolve) => setTimeout(resolve, 20)); });
  expect(host.textContent).toContain("recheck connection failed");
  expect(host.textContent).not.toContain("Debug prerequisites ready");
});
