import { expect, test } from "@playwright/test";

test("loads the app, lazy topology and lazy terminal without bundle errors", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(() => {
    localStorage.setItem("vzctl.ui.settings.v1", JSON.stringify({ theme: "light", locale: "enUS" }));
  });
  await page.goto("/env?demo=1&path=%2Fdemo%2Fedge-dmz&tab=ops");
  await expect(page.getByRole("link", { name: "Topology", exact: true })).toBeVisible();
  // Graph and terminal code must not be needed for the operations screen.
  expect(await page.evaluate(() => performance.getEntriesByType("resource").some((r) => /\/(TopologyEditor|TerminalView)[-.]/.test(r.name)))).toBe(false);
  await page.getByRole("link", { name: "Debug", exact: true }).click();
  await page.getByRole("link", { name: "Topology", exact: true }).click();
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(4);
  await page.getByRole("link", { name: "Debug", exact: true }).click();
  await page.getByRole("link", { name: "Topology", exact: true }).click();
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(4);
  await page.goto("/vms/edge-dmz%252Fweb/shell?demo=1&stackPath=%2Fdemo%2Fedge-dmz");
  await expect(page.locator(".xterm").first()).toBeVisible();
  expect(errors).toEqual([]);
});
