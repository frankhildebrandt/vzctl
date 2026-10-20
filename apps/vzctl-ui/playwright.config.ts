import { defineConfig, devices } from "@playwright/test";

const bundle = process.env.VZCTL_UI_TEST_BUNDLE === "1";
const port = bundle ? 1422 : 1421;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "on-first-retry",
  },
  webServer: {
    command: `npx vite ${bundle ? "preview" : ""} --port ${port} --strictPort --host 127.0.0.1`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: true,
    timeout: 120_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    // Tauri uses WKWebView on macOS; keep its rendering in the smoke suite.
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
});
