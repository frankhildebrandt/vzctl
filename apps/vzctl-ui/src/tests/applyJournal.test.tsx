// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ApplyJournalPanel } from "@/components/ApplyJournalPanel";
import { useSettingsStore } from "@/store/settingsStore";
const run = vi.hoisted(() => vi.fn());
vi.mock("@/lib/vzctl", () => ({ runVzctl: run }));
let root: Root, host: HTMLDivElement;
const journal = { id: "7", generation: 7, status: "failed", step: "ensure_controllers", mode: "apply", error: "MinIO provisioning failed (operation: account.create)", updated_at: "2026-10-20T13:00:00Z" };
const report = (data: unknown, ok = true) => JSON.stringify({ sections: { apply: { ok, data } } });
beforeEach(() => {
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
 useSettingsStore.getState().setLocale("enUS");
 host = document.createElement("div"); document.body.append(host); root = createRoot(host);
 run.mockReset().mockResolvedValue("{}");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const render = async (raw: string) => { await act(async () => root.render(<ApplyJournalPanel raw={raw} path="/lab" />)); };
const button = (name: string) => [...host.querySelectorAll("button")].find(b => b.textContent === name);
it("loads a persisted CLI failure without replaying it and uses existing explicit resume", async () => {
 await render(report({ journal, lease: null }));
 expect(host.textContent).toContain("Last operation failed");
 expect(host.textContent).toContain("ensure_controllers");
 expect(host.textContent).toContain("account.create");
 expect(host.textContent).toContain("7");
 expect(run).not.toHaveBeenCalled();
 await act(async () => button("Resume (--resume)")!.click());
 expect(run).toHaveBeenCalledExactlyOnceWith("/lab", "apply", { resume: true });
});
it("shows lease details and blocks recovery while another operation may own it", async () => {
 await render(report({ journal, lease: { holder: "host:123", expires_at: "2099-01-01T00:00:00Z" } }));
 expect(host.textContent).toContain("host:123");
 expect(button("Resume (--resume)")).toBeUndefined();
 expect(button("Abort only")).toBeUndefined();
 expect(run).not.toHaveBeenCalled();
});
it("retains the journal error after a failed explicit resume", async () => {
 run.mockRejectedValue(new Error("stack lease held by host:456 until later"));
 await render(report({ journal, lease: null }));
 await act(async () => button("Resume (--resume)")!.click());
 expect(host.textContent).toContain("account.create");
 expect(host.textContent).toContain("host:456");
});
it("distinguishes no journal from unavailable or malformed state", async () => {
 await render(report({ journal: null, lease: null })); expect(host.textContent).toBe("");
 await render(report(null, false)); expect(host.textContent).toContain("Operation history unavailable");
 await render(report({ journal: { ...journal, id: {} }, lease: null }));
 expect(host.textContent).toContain("Operation history unavailable");
 expect(run).not.toHaveBeenCalled();
});
it("shows pending/running journals, permits expired lease recovery, and never renders payload", async () => {
 await render(report({ journal: { ...journal, status: "running", error: null, payload: "PRIVATE-CREDENTIAL" }, lease: { holder: "host:123", expires_at: "2020-01-01T00:00:00Z" } }));
 expect(host.textContent).toContain("Unfinished operation");
 expect(host.textContent).not.toContain("PRIVATE-CREDENTIAL");
 expect(button("Resume (--resume)")).toBeDefined();
 await render(report({ journal: { ...journal, status: "pending" }, lease: { holder: "host:123", expires_at: "invalid" } }));
 expect(button("Resume (--resume)")).toBeUndefined();
 expect(run).not.toHaveBeenCalled();
});
