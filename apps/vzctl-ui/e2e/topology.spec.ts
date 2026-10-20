import { test, expect, type Page } from "@playwright/test";

const projectPath = "/test/topology";
const topologyUrl = `/env?path=${encodeURIComponent(projectPath)}&tab=topology`;
const routingWarnings = new WeakMap<Page, string[]>();

async function seedProject(page: Page) {
  // Explicit demo mode is isolated from the installed supervisor. Never depend
  // on a failed real API call silently creating a memory-only project.
  await page.goto("/projects?demo=1");
  await page.evaluate(async (path) => {
    const { useSettingsStore } = await import("/src/store/settingsStore.ts");
    useSettingsStore.getState().setLocale("enUS");
    const { createProjectFlexible, saveProjectFlexible } = await import("/src/features/persistence/projectIo.ts");
    const { env, diagram } = await createProjectFlexible(path, "topology");
    env.spec.networks.external = { ...env.spec.networks.lan, cidr: "10.90.0.0/24" };
    const vm = { from: "ubuntu-base", clone: "linked" as const, disk: "12G", roles: [], dependsOn: [], requires: [], ports: [], mounts: [] };
    env.spec.vms = {
      router: { ...vm, roles: ["router"], networks: [{ name: "lan", ip: "10.80.0.2" }, { name: "external", ip: "10.90.0.2" }] },
      home: { ...vm, networks: [{ name: "lan", ip: "10.80.0.10" }] },
      external: { ...vm, networks: [{ name: "external", ip: "10.90.0.10" }] },
    };
    await saveProjectFlexible(path, env, diagram);
  }, projectPath);
}

async function expectRenderedTopology(page: Page) {
  const canvas = page.locator(".topology-canvas");
  await expect(canvas.locator('[data-shape="vzctl-network"]')).toHaveCount(2);
  await expect(canvas.locator('[data-shape="vzctl-vm"]')).toHaveCount(3);
  const uplinks = canvas.locator('[data-cell-id^="uplink:"] > path:last-of-type');
  await expect(uplinks).toHaveCount(2);
  await expect.poll(() => uplinks.evaluateAll((paths) =>
    paths.every((path) => (path as SVGPathElement).getTotalLength() > 10),
  )).toBe(true);
  // Existence alone missed the blank-canvas regression. Require all VM bodies
  // to have a visible size and intersect the actual clipped canvas viewport.
  await expect.poll(async () => canvas.evaluate((element) => {
    const viewport = element.getBoundingClientRect();
    return [...element.querySelectorAll('[data-shape="vzctl-vm"]')].every((node) => {
      const box = node.getBoundingClientRect();
      return box.width > 20 && box.height > 10 && box.right > viewport.left &&
        box.left < viewport.right && box.bottom > viewport.top && box.top < viewport.bottom;
    });
  })).toBe(true);
}

async function expectUplinkConnections(page: Page) {
  await expect.poll(() => page.locator(".topology-canvas").evaluate((canvas) =>
    ["lan", "external"].every((name) => {
      const path = canvas.querySelector<SVGPathElement>(`[data-cell-id="uplink:${name}"] > path:last-of-type`);
      const source = canvas.querySelector<SVGCircleElement>(`[data-cell-id="igw:${name}"] circle[port="down"]`);
      const target = canvas.querySelector<SVGCircleElement>(`[data-cell-id="net:${name}"] circle[port="uplink:${name}"]`);
      const matrix = path?.getScreenCTM();
      if (!path || !source || !target || !matrix) return false;
      return [[source, 0], [target, path.getTotalLength()]].every(([port, distance]) => {
        const point = path.getPointAtLength(distance as number);
        const screen = new DOMPoint(point.x, point.y).matrixTransform(matrix);
        const box = (port as SVGCircleElement).getBoundingClientRect();
        return Math.hypot(screen.x - box.x - box.width / 2, screen.y - box.y - box.height / 2) < 2;
      });
    }),
  )).toBe(true);
}

test.beforeEach(async ({ page }) => {
  const warnings: string[] = [];
  routingWarnings.set(page, warnings);
  page.on("console", (message) => {
    if (message.type() === "warning" && message.text().includes("Unable to execute manhattan")) {
      warnings.push(message.text());
    }
  });
  await seedProject(page);
});

test.afterEach(async ({ page }) => {
  expect(routingWarnings.get(page)).toEqual([]);
});

test("renders networks and VMs on initial load, fit, resize and remount", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(topologyUrl);
  await expectRenderedTopology(page);
  await page.getByRole("button", { name: "Fit view", exact: true }).click();
  await expectRenderedTopology(page);
  await page.setViewportSize({ width: 2400, height: 1500 });
  await page.getByRole("button", { name: "Fit view", exact: true }).click();
  await expectRenderedTopology(page);
  await page.getByRole("link", { name: "Operations", exact: true }).click();
  await expect(page.locator(".topology-canvas")).toHaveCount(0);
  await page.getByRole("link", { name: "Topology", exact: true }).click();
  await expectRenderedTopology(page);
  expect(errors).toEqual([]);
});

test("loads topology after VM inventory, operations and debug navigation", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.evaluate(async (path) => {
    const { rememberProject } = await import("/src/lib/projects.ts");
    const { api } = await import("/src/lib/api.ts");
    const get = api.get;
    api.get = async (url) => {
      if (url === "/v1/stacks") return { stacks: [{ id: "topology", path, name: "topology", openedAt: new Date().toISOString() }] } as never;
      if (url.endsWith("/debug/sources")) return { status: "ok", exit_code: 0,
        sources: { dataSources: { postgres: { default: "db", via: "router", entries: ["db"] } } } } as never;
      if (url.endsWith("/debug/doctor")) return { status: "ok", exit_code: 0,
        doctor: { ok: true, checks: [{ type: "postgres", tool: "psql", vmId: "topology/router", status: "ok" }] } } as never;
      return get(url);
    };
    const post = api.post;
    api.post = async (url, body) => {
      if (url.endsWith("/debug/postgres/query")) return { status: "ok", exit_code: 0,
        result: { columns: ["?column?"], rows: [["1"]], rowCount: 1 } } as never;
      return post(url, body);
    };
    await rememberProject(path, "topology");
    await (window as any).__TSR_ROUTER__.navigate({ to: "/vms" });
    await (window as any).__TSR_ROUTER__.navigate({ to: "/env", search: { path, tab: "ops" } });
  }, projectPath);
  for (const width of [1280, 1100, 1280, 1100]) {
    await page.setViewportSize({ width, height: 740 });
    await page.getByRole("link", { name: "Debug", exact: true }).click();
    await expect(page.locator(".topology-canvas")).toHaveCount(0);
    await page.getByRole("button", { name: "Run query", exact: true }).click();
    await expect(page.locator(".debug-result")).toContainText('"rowCount": 1');
    await page.getByRole("link", { name: "Topology", exact: true }).click();
    await page.locator(".topology-canvas").scrollIntoViewIfNeeded();
    await expectRenderedTopology(page);
    await page.getByRole("button", { name: "Fit view", exact: true }).click();
    await expectRenderedTopology(page);
    await page.getByRole("link", { name: "Operations", exact: true }).click();
  }
  expect(errors).toEqual([]);
});

test("keeps palette actions and inspector reachable in a 1100 by 740 window", async ({ page, browserName }) => {
  await page.setViewportSize({ width: 1100, height: 740 });
  await page.goto(topologyUrl);
  const palette = page.locator(".topology-palette");
  await expect(palette.locator(".topology-palette-item")).toHaveCount(4);
  // Stacked panels must retain their content height, not shrink to a title/filter strip.
  await expect.poll(() => palette.evaluate((panel) => {
    const box = panel.getBoundingClientRect();
    return [...panel.querySelectorAll(".topology-palette-item")].every((button) => {
      const item = button.getBoundingClientRect();
      return item.top >= box.top && item.bottom <= box.bottom;
    });
  })).toBe(true);
  const canvas = page.locator(".topology-canvas");
  await canvas.scrollIntoViewIfNeeded();
  await page.getByRole("button", { name: "Fit view", exact: true }).click();
  const vm = canvas.locator('[data-cell-id="vm:router"] rect').first();
  await vm.click();
  const name = page.locator(".topology-inspector input[data-topology-name]");
  await name.scrollIntoViewIfNeeded();
  await expect(name).toBeInViewport();
  await expect(name).toHaveValue("router");
  await palette.scrollIntoViewIfNeeded();
  await expect(palette.locator(".topology-palette-item").last()).toBeInViewport();
  await page.screenshot({ path: `/tmp/vzctl-topology-small-${browserName}.png` });
});

test("palette edits update the rendered graph and undo restores it", async ({ page }) => {
  await page.goto(topologyUrl);
  await expectRenderedTopology(page);
  await page.locator(".topology-palette-item", { hasText: "Compute" }).click();
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(4);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expectRenderedTopology(page);
  await expect(page.getByRole("button", { name: "Undo", exact: true })).toBeDisabled();
});

test("a graph edit survives an explicit demo save and reload", async ({ page }) => {
  await page.goto(topologyUrl);
  await expectRenderedTopology(page);
  await page.locator(".topology-palette-item", { hasText: "Compute" }).click();
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(4);
  await page.getByRole("button", { name: /Save/, exact: false }).click();
  await expect(page.getByRole("button", { name: "Save", exact: true })).toBeEnabled();
  await page.reload();
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(4);
});

test("dragging from the palette materializes exactly one VM", async ({ page }) => {
  await page.goto(topologyUrl);
  await expectRenderedTopology(page);
  const source = await page.locator(".topology-palette-item", { hasText: "Compute" }).boundingBox();
  const target = await page.locator(".topology-canvas").boundingBox();
  expect(source).not.toBeNull();
  expect(target).not.toBeNull();
  await page.mouse.move(source!.x + source!.width / 2, source!.y + source!.height / 2);
  await page.mouse.down();
  await page.mouse.move(target!.x + target!.width / 2, target!.y + target!.height / 2, { steps: 12 });
  await page.mouse.up();
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(4);
});

test("uplinks remain attached when gateway and network move and after save/reload", async ({ page }) => {
  await page.setViewportSize({ width: 1800, height: 1200 });
  await page.goto(topologyUrl);
  await expectRenderedTopology(page);
  await expectUplinkConnections(page);
  const path = page.locator('.topology-canvas [data-cell-id="uplink:lan"] > path:last-of-type');
  for (const move of [
    { id: "igw:lan", x: 40, y: 30, dx: -70, dy: -70 },
    // Move into free space, not over the multi-homed router below the network.
    { id: "net:lan", x: 40, y: 15, dx: 40, dy: -35 },
  ]) {
    const previous = await path.getAttribute("d");
    const box = await page.locator(`.topology-canvas [data-cell-id="${move.id}"]`).boundingBox();
    expect(box).not.toBeNull();
    await page.mouse.move(box!.x + move.x, box!.y + move.y);
    await page.mouse.down();
    await page.mouse.move(box!.x + move.x + move.dx, box!.y + move.y + move.dy, { steps: 12 });
    await page.mouse.up();
    await expect.poll(() => path.getAttribute("d")).not.toBe(previous);
    await expectUplinkConnections(page);
  }
  await page.getByRole("button", { name: /Save/, exact: false }).click();
  await expect(page.getByRole("button", { name: "Save", exact: true })).toBeEnabled();
  await page.reload();
  await expectRenderedTopology(page);
  await expectUplinkConnections(page);
});
