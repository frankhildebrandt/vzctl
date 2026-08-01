import { test, expect } from "@playwright/test";

async function seedProject(
  page: import("@playwright/test").Page,
  path: string,
  name: string,
) {
  await page.goto("/projects");
  await page.evaluate(
    async ({ path: p, name: n }) => {
      const { createProjectFlexible } = await import(
        "/src/features/persistence/projectIo.ts"
      );
      const { rememberProject } = await import("/src/lib/projects.ts");
      await createProjectFlexible(p, n);
      await rememberProject(p);
    },
    { path, name },
  );
}

test.describe("topology editor flows", () => {
  test("1-3: VM + Netz + Inspector", async ({ page }) => {
    await seedProject(page, "/tmp/e2e-flow", "e2e-flow");
    await page.goto("/env?path=%2Ftmp%2Fe2e-flow&tab=topology");
    await expect(page.getByLabel("Komponentenpalette")).toBeVisible();
    await page.locator(".topology-palette-item", { hasText: "Netzwerk" }).click();
    await page.locator(".topology-palette-item", { hasText: "Compute" }).click();
    await expect(page.getByRole("button", { name: "Undo" })).toBeEnabled();
    await expect(page.getByRole("button", { name: /Speichern/ })).toBeVisible();
  });

  test("tabs: Topologie und Betrieb", async ({ page }) => {
    await seedProject(page, "/tmp/e2e-tabs", "e2e-tabs");
    await page.goto("/env?path=%2Ftmp%2Fe2e-tabs");
    await expect(page.getByRole("tab", { name: "Betrieb" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.getByRole("tab", { name: "Topologie" }).click();
    await expect(page.getByLabel("Topologie-Canvas")).toBeVisible();
    await page.getByRole("tab", { name: "Betrieb" }).click();
    await expect(page.getByRole("tab", { name: "Betrieb" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("undo button exists after create", async ({ page }) => {
    await seedProject(page, "/tmp/e2e-undo", "e2e-undo");
    await page.goto("/env?path=%2Ftmp%2Fe2e-undo&tab=topology");
    await page.locator(".topology-palette-item", { hasText: "Compute" }).click();
    await expect(page.getByRole("button", { name: "Undo" })).toBeEnabled();
    await page.getByRole("button", { name: "Undo" }).click();
  });

  test("validation panel sichtbar", async ({ page }) => {
    await seedProject(page, "/tmp/e2e-val", "e2e-val");
    await page.goto("/env?path=%2Ftmp%2Fe2e-val&tab=topology");
    await expect(
      page.getByRole("heading", { name: /Validierung/ }),
    ).toBeVisible();
  });
});
