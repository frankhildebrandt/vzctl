import { expect, test } from "@playwright/test";

test("keeps same-name networks separate and deletes only the explicitly confirmed project ID", async ({ page, browserName }) => {
  await page.addInitScript(() => localStorage.setItem("vzctl.ui.settings.v1", JSON.stringify({ theme: "light", locale: "enUS" })));
  await page.goto("/projects?demo=1");
  await page.evaluate(async () => {
    const { api } = await import("/src/lib/api.ts");
    const get = api.get;
    (window as any).networkDeletes = [];
    api.delete = async (path) => { (window as any).networkDeletes.push(path); return {} as never; };
    api.get = async (path) => {
      if (path === "/v1/nets") return { networks: [
        { id: "alpha/lan", name: "lan", project: "alpha", cidr: "10.80.0.0/24" },
        { id: "beta/lan", name: "lan", project: "beta", cidr: "10.90.0.0/24" },
      ], attachments: [
        { network_id: "alpha/lan", network: "lan", vm_id: "alpha/a" },
        { network_id: "beta/lan", network: "lan", vm_id: "beta/b" },
        { network_id: "beta/lan", network: "lan", vm_id: "beta/c" },
      ] } as never;
      if (path === "/v1/nets/default") return { id: "alpha/lan", name: "lan", cidr: "10.80.0.0/24" } as never;
      return get(path);
    };
    await (window as any).__TSR_ROUTER__.navigate({ to: "/networks" });
  });
  const alpha = page.locator("tbody tr").filter({ hasText: "alpha/lan" });
  const beta = page.locator("tbody tr").filter({ hasText: "beta/lan" });
  await expect(alpha.locator("td").nth(4)).toHaveText("1");
  await expect(beta.locator("td").nth(4)).toHaveText("2");
  await expect(alpha).toContainText("default");
  await expect(beta).not.toContainText("default");
  await beta.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("alertdialog")).toContainText("beta/lan");
  await page.screenshot({ path: `/tmp/vzctl-network-scopes-${browserName}.png` });
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel" }).click();
  expect(await page.evaluate(() => (window as any).networkDeletes)).toEqual([]);
  await beta.getByRole("button", { name: "Delete", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).networkDeletes)).toEqual(["/v1/nets/beta%2Flan?project=beta"]);
});
