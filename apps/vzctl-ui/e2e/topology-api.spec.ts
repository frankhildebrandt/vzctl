import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { parse, stringify } from "yaml";

test("renders the REST-loaded graph repeatedly after successful SQL queries", async ({ page }) => {
  const env = parse(readFileSync("../../crates/vzctl/tests/fixtures/validate/valid-data-sources-loopback.yaml", "utf8"));
  env.spec.networks.external = { cidr: "10.91.0.0/24", mode: "shared" };
  env.spec.vms.web.roles = ["router"];
  env.spec.vms.web.networks.push({ name: "external", ip: "10.91.0.2" });
  env.spec.vms.home = { from: "ubuntu-base", disk: "4G", networks: [{ name: "lan", ip: "10.90.0.11" }] };
  env.spec.vms.external = { from: "ubuntu-base", disk: "4G", networks: [{ name: "external", ip: "10.91.0.11" }] };
  await page.addInitScript(({ yaml }) => {
    localStorage.setItem("vzctl.ui.settings.v1", JSON.stringify({ theme: "hot-pink", locale: "enUS" }));
    const stack = { id: "loopback", name: "loopback", path: "/fixture/rest-topology", openedAt: new Date().toISOString() };
    // Exercise the native API transport boundary, YAML parser and missing
    // sidecar handling. Nothing reaches the installed supervisor or a VM.
    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (command: string, payload: any) => {
        if (command !== "api_request") return null;
        const path = payload.args.path as string;
        const response = (body: unknown, status = 200) => ({ status, body: JSON.stringify(body), contentType: "application/json" });
        if (path === "/v1/stacks") return response(payload.args.method === "GET" ? { stacks: [stack] } : stack);
        if (path.endsWith("/config")) return { status: 200, body: yaml, contentType: "text/yaml" };
        if (path.endsWith("/diagram")) return response({ error: { code: "not_found", message: "diagram not found" } }, 404);
        if (path.endsWith("/debug/sources")) return response({ status: "ok", exit_code: 0, sources: { dataSources: { postgres: { default: "local", via: "web", entries: ["local"] } } } });
        if (path.endsWith("/debug/doctor")) return response({ status: "ok", exit_code: 0, doctor: { ok: true, checks: [{ type: "postgres", tool: "psql", vmId: "loopback/web", status: "ok" }] } });
        if (path.endsWith("/debug/postgres/query")) return response({ status: "ok", exit_code: 0, result: { columns: ["?column?"], rows: [["1"]], rowCount: 1 } });
        if (path.endsWith("/status")) return response({ status: "ok", exit_code: 0, sections: {} });
        if (path === "/v1/vms") return response({ vms: [] });
        return response({});
      },
    };
  }, { yaml: stringify(env) });
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1848, height: 1147 });
  await page.goto("/env?path=%2Ffixture%2Frest-topology&tab=debug");
  for (let i = 0; i < 4; i++) {
    await page.getByRole("button", { name: "Run query", exact: true }).click();
    await expect(page.locator(".debug-result")).toContainText('"rowCount": 1');
    await page.getByRole("link", { name: "Topology", exact: true }).click();
    const canvas = page.locator(".topology-canvas");
    await expect(canvas.locator('[data-shape="vzctl-vm"]')).toHaveCount(3);
    await expect.poll(() => canvas.evaluate((element) => {
      const viewport = element.getBoundingClientRect();
      return [...element.querySelectorAll('[data-shape="vzctl-vm"]')].every((node) => {
        const box = node.getBoundingClientRect();
        return box.width > 20 && box.height > 10 && box.left < viewport.right && box.right > viewport.left &&
          box.top < viewport.bottom && box.bottom > viewport.top;
      });
    })).toBe(true);
    await page.getByRole("button", { name: "Fit view", exact: true }).click();
    await page.getByRole("link", { name: "Debug", exact: true }).click();
  }
  // Native accessibility navigation can run while WKWebView defers animation
  // frames in the background. Reproduce that scheduling boundary explicitly,
  // then ensure foreground resumption renders the already-loaded model.
  await page.evaluate(() => {
    const request = window.requestAnimationFrame.bind(window);
    const cancel = window.cancelAnimationFrame.bind(window);
    const pending = new Map<number, FrameRequestCallback>();
    let id = 0;
    window.requestAnimationFrame = (callback) => { pending.set(++id, callback); return id; };
    window.cancelAnimationFrame = (token) => { pending.delete(token); };
    (window as any).resumeTopologyFrames = () => {
      window.requestAnimationFrame = request;
      window.cancelAnimationFrame = cancel;
      pending.forEach((callback) => request(callback));
    };
    [...document.querySelectorAll<HTMLAnchorElement>("a")].find((link) => link.textContent === "Topology")!.click();
  });
  await expect(page.locator(".topology-canvas")).toHaveCount(1);
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(0);
  await page.evaluate(() => (window as any).resumeTopologyFrames());
  await expect(page.locator('.topology-canvas [data-shape="vzctl-vm"]')).toHaveCount(3);
  expect(errors).toEqual([]);
});
