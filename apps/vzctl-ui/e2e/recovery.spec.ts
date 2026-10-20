import { expect, test } from "@playwright/test";

test("requires explicit recovery and preserves failure through background status refresh", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("vzctl.ui.settings.v1", JSON.stringify({ theme: "light", locale: "enUS" })));
  await page.goto("/env?demo=1&path=%2Fdemo%2Fedge-dmz&tab=ops");
  await expect(page.getByRole("button", { name: "Up", exact: true })).toBeEnabled();
  await page.evaluate(async () => {
    const { api } = await import("/src/lib/api.ts");
    const originalGet = api.get;
    const originalPost = api.post;
    const state = { mutations: [] as Array<{ path: string; body: unknown }>, polls: 0 };
    (window as unknown as { recoveryTest: typeof state }).recoveryTest = state;
    api.post = async (path, body) => {
      if (/\/(up|apply)$/.test(path)) {
        state.mutations.push({ path, body });
        return { jobId: "recovery-test" } as never;
      }
      return originalPost(path, body);
    };
    api.get = async (path) => {
      if (path === "/v1/jobs/recovery-test") return {
        status: "failed", result: { status: "fail", exit_code: 5, summary: { message: "incomplete journal test-journal at await_agents; run `vzctl apply --resume` or `vzctl apply --abort`" } },
      } as never;
      const result = await originalGet(path);
      if (path.endsWith("/status")) {
        state.polls += 1;
        return { ...(result as object), testPoll: state.polls } as never;
      }
      return result;
    };
  });
  await page.getByRole("button", { name: "Up", exact: true }).click();
  await expect(page.getByRole("button", { name: "Run up", exact: true })).toBeFocused();
  await page.getByRole("button", { name: "Run up", exact: true }).click();
  await expect(page.getByRole("button", { name: "Resume (--resume)", exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as unknown as { recoveryTest: { polls: number } }).recoveryTest.polls), { timeout: 12_000 }).toBeGreaterThan(0);
  await expect(page.getByRole("button", { name: "Resume (--resume)", exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { recoveryTest: { mutations: unknown[] } }).recoveryTest.mutations.length)).toBe(1);
  await page.getByRole("button", { name: "Abort only", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await page.evaluate(() => (window as unknown as { recoveryTest: { mutations: unknown[] } }).recoveryTest.mutations.length)).toBe(1);
  await page.getByRole("button", { name: "Resume (--resume)", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { recoveryTest: { mutations: unknown[] } }).recoveryTest.mutations.length)).toBe(2);
  expect(await page.evaluate(() => (window as unknown as { recoveryTest: { mutations: Array<{ body: unknown }> } }).recoveryTest.mutations[1].body)).toEqual({ resume: true });
});

test("Enter on an opened purge dialog cancels; purge requires explicit confirmation", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("vzctl.ui.settings.v1", JSON.stringify({ theme: "light", locale: "enUS" })));
  await page.goto("/env?demo=1&path=%2Fdemo%2Fedge-dmz&tab=ops");
  await expect(page.getByRole("button", { name: "Remove stack", exact: true })).toBeEnabled();
  await page.evaluate(async () => {
    const { api } = await import("/src/lib/api.ts");
    const originalGet = api.get;
    const state = { mutations: [] as Array<{ path: string; body: unknown }> };
    (window as unknown as { purgeTest: typeof state }).purgeTest = state;
    // Intercept every write: even explicit confirmation stays entirely in the mock.
    api.post = async (path, body) => {
      state.mutations.push({ path, body });
      return { jobId: "purge-test" } as never;
    };
    api.get = async (path) => path === "/v1/jobs/purge-test"
      ? { status: "succeeded", result: { status: "ok", exit_code: 0 } } as never
      : originalGet(path);
  });
  const mutations = () => page.evaluate(() => (window as unknown as {
    purgeTest: { mutations: Array<{ path: string; body: unknown }> };
  }).purgeTest.mutations);
  const openPurge = page.getByRole("toolbar").getByRole("button", { name: "Remove stack", exact: true });
  await openPurge.click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  expect(await mutations()).toEqual([]);

  await openPurge.click();
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeFocused();
  await dialog.getByRole("button", { name: "Remove stack", exact: true }).click();
  // Success may also remember the project; count only the destructive request.
  const purges = async () => (await mutations()).filter(({ path }) => path.endsWith("/down"));
  await expect.poll(async () => (await purges()).length).toBe(1);
  expect((await purges())[0]).toEqual({
    path: expect.stringMatching(/\/down$/),
    body: { purge: true },
  });
});
