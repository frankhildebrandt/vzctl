// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NetworksPage } from "@/components/NetworksPage";
import { useSettingsStore } from "@/store/settingsStore";
const api = vi.hoisted(() => ({ get: vi.fn(), delete: vi.fn() }));
vi.mock("@/lib/api", () => ({ api, encodeId: encodeURIComponent }));
let root: Root;
let host: HTMLDivElement;
let client: QueryClient;
let snapshot: { networks: object[]; attachments: object[] };
let defaultNet: object | null;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  useSettingsStore.getState().setLocale("enUS");
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  snapshot = {
    networks: [
      { id: "alpha/lan", name: "lan", project: "alpha", cidr: "10.80.0.0/24" },
      { id: "beta/lan", name: "lan", project: "beta", cidr: "10.90.0.0/24" },
    ],
    attachments: [
      { network_id: "alpha/lan", network: "lan", vm_id: "alpha/a" },
      { network_id: "beta/lan", network: "lan", vm_id: "beta/b" },
      { network_id: "beta/lan", network: "lan", vm_id: "beta/c" },
    ],
  };
  defaultNet = { id: "alpha/lan", name: "lan", cidr: "10.80.0.0/24" };
  api.get.mockReset().mockImplementation(async (path) => path === "/v1/nets" ? snapshot : defaultNet);
  api.delete.mockReset().mockResolvedValue({});
});
afterEach(async () => { await act(async () => root.unmount()); client.clear(); host.remove(); });
async function render() {
  await act(async () => root.render(<QueryClientProvider client={client}><NetworksPage /></QueryClientProvider>));
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
}
const rows = () => [...host.querySelectorAll("tbody tr")];
async function click(button: HTMLButtonElement) { await act(async () => button.click()); }
function dialogButton(label: string) {
  return [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find((b) => b.textContent === label)!;
}
it("joins counts/default by ID and confirms the project before encoded-ID deletion", async () => {
  await render();
  expect(rows()[0].textContent).toContain("alpha/lan");
  expect(rows()[1].textContent).toContain("beta/lan");
  expect(rows()[0].querySelectorAll("td")[4].textContent).toBe("1");
  expect(rows()[1].querySelectorAll("td")[4].textContent).toBe("2");
  expect(rows()[0].textContent).toContain("default");
  expect(rows()[1].textContent).not.toContain("default");
  await click(rows()[1].querySelector("button")!);
  expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("beta/lan");
  await click(dialogButton("Cancel"));
  expect(api.delete).not.toHaveBeenCalled();
  await click(rows()[1].querySelector("button")!);
  await click(dialogButton("Delete"));
  expect(api.delete).toHaveBeenCalledExactlyOnceWith("/v1/nets/beta%2Flan?project=beta");
});
it("fails closed for ambiguous legacy names and does not mark project nets as a global default", async () => {
  snapshot.networks = snapshot.networks.map(({ id: _id, ...net }: any) => net);
  snapshot.attachments = [{ network: "lan", vm_id: "alpha/a" }];
  defaultNet = { name: "lan", cidr: "10.80.0.0/24" };
  await render();
  for (const row of rows()) {
    expect(row.querySelector("button")?.disabled).toBe(true);
    expect(row.querySelectorAll("td")[4].textContent).toBe("—");
    expect(row.textContent).not.toContain("default");
  }
  expect(api.delete).not.toHaveBeenCalled();
});
it("retains safe unique legacy global names and scoped attachment references", async () => {
  snapshot = { networks: [{ name: "lan", cidr: "10.80.0.0/24" }], attachments: [{ network_name: "lan" }] };
  defaultNet = { name: "lan", cidr: "10.80.0.0/24" };
  await render();
  expect(rows()[0].textContent).toContain("default");
  expect(rows()[0].querySelectorAll("td")[4].textContent).toBe("1");
  await click(rows()[0].querySelector("button")!);
  await click(dialogButton("Delete"));
  expect(api.delete).toHaveBeenCalledExactlyOnceWith("/v1/nets/lan");
});
it("never retries a failed ID deletion using a short network name", async () => {
  api.delete.mockRejectedValue(new Error("network ID unavailable"));
  await render();
  await click(rows()[0].querySelector("button")!);
  await click(dialogButton("Delete"));
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
  expect(api.delete).toHaveBeenCalledExactlyOnceWith("/v1/nets/alpha%2Flan?project=alpha");
  expect(host.textContent).toContain("network ID unavailable");
});
it("uses explicit legacy attachment projects but never guesses from VM IDs", async () => {
  snapshot.attachments = [{ network: "lan", project: "alpha", vm_id: "alpha/a" },
    { network_id: "unknown/lan", network: "lan", vm_id: "beta/b" }];
  defaultNet = { name: "lan", cidr: "10.80.0.0/24" };
  await render();
  expect(rows()[0].querySelectorAll("td")[4].textContent).toBe("1");
  expect(rows()[1].querySelectorAll("td")[4].textContent).toBe("0");
  expect(rows().some((row) => row.textContent?.includes("default"))).toBe(false);
});
it("blocks a missing ID even when its namesake has a canonical ID", async () => {
  snapshot.networks[0] = { name: "lan", project: "alpha", cidr: "10.80.0.0/24" };
  await render();
  expect(rows()[0].querySelector("button")?.disabled).toBe(true);
  expect(rows()[1].querySelector("button")?.disabled).toBe(false);
});

it("does not delete a replacement owner after the confirmation was opened", async () => {
  snapshot = { networks: [{ name: "lan", project: "alpha" }], attachments: [] };
  await render();
  await click(rows()[0].querySelector("button")!);
  await act(async () => { client.setQueryData(["nets", "list"], {
    networks: [{ name: "lan", project: "beta" }], attachments: [],
  }); });
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
  expect(rows()[0].textContent).toContain("beta/lan");
  await click(dialogButton("Delete"));
  expect(api.delete).not.toHaveBeenCalled();
});
