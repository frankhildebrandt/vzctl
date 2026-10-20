// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ApplyRecoveryActions } from "@/components/ApplyRecoveryActions";
import { useSettingsStore } from "@/store/settingsStore";
const run = vi.hoisted(() => vi.fn());
vi.mock("@/lib/vzctl", () => ({ runVzctl: run }));
let root: Root;
let host: HTMLDivElement;
const raw = "incomplete journal abc-123 at await_agents; run `vzctl apply --resume` or `vzctl apply --abort`";
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  useSettingsStore.getState().setLocale("enUS");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  run.mockReset().mockResolvedValue("{}");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const click = async (label: string, within: ParentNode = document) => {
  const button = [...within.querySelectorAll("button")].find((node) => node.textContent === label);
  expect(button).toBeDefined();
  await act(async () => button!.click());
};
it("resumes only after explicit selection and keeps journal details", async () => {
  const done = vi.fn();
  await act(async () => root.render(<ApplyRecoveryActions raw={raw} path="/stack" onDone={done} />));
  expect(host.textContent).toContain("abc-123");
  expect(host.textContent).toContain("await_agents");
  expect(run).not.toHaveBeenCalled();
  await click("Resume (--resume)");
  expect(run).toHaveBeenCalledExactlyOnceWith("/stack", "apply", { resume: true });
  expect(done).toHaveBeenCalledOnce();
});
it("requires confirmation before abort, and cancel performs no command", async () => {
  await act(async () => root.render(<ApplyRecoveryActions raw={raw} path="/stack" />));
  await click("Abort only");
  expect(run).not.toHaveBeenCalled();
  await click("Cancel", document.querySelector('[role="alertdialog"]')!);
  expect(run).not.toHaveBeenCalled();
  await click("Abort only");
  await click("Abort only", document.querySelector('[role="alertdialog"]')!);
  expect(run).toHaveBeenCalledExactlyOnceWith("/stack", "apply", { abort: true });
});
it("shows a conflicting lease holder and offers no unsafe resume/abort", async () => {
  await act(async () => root.render(<ApplyRecoveryActions raw="stack lease held by host:456 until 2026-10-20T12:00:00Z" path="/stack" />));
  expect(host.textContent).toContain("host:456");
  expect(host.textContent).toContain("PID456");
  expect(host.querySelectorAll("button")).toHaveLength(0);
  expect(run).not.toHaveBeenCalled();
});
it("keeps the original journal and new failure visible when recovery fails", async () => {
  run.mockRejectedValue(new Error("stack lease held by host:789 until later"));
  const done = vi.fn();
  await act(async () => root.render(<ApplyRecoveryActions raw={raw} path="/stack" onDone={done} />));
  await click("Resume (--resume)");
  expect(host.textContent).toContain("abc-123");
  expect(host.textContent).toContain("host:789");
  expect(done).not.toHaveBeenCalled();
});
