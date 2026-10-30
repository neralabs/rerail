import { defineConfig } from "@playwright/test";

import { baseURL, isCI } from "./test/browser/config.mjs";

export default defineConfig({
  testDir: "./test/browser",
  fullyParallel: true,
  forbidOnly: isCI,
  retries: isCI ? 2 : 0,
  reporter: isCI ? "github" : "list",
  globalSetup: "./test/browser/global-setup.mjs",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
  webServer: {
    command: "node test/browser/server.mjs",
    url: baseURL,
    reuseExistingServer: !isCI,
    timeout: 60_000,
  },
});
