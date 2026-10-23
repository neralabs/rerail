import { defineConfig, devices } from "@playwright/test";

/**
 * The dashboard's browser tests (`npm run test:ui:e2e`, e2e/README.md): Chromium only, each worker running its own
 * seeded daemons (e2e/dashboard-test.ts), so there is no `webServer`. UTC and en-US keep dates and "today" stable. Results
 * go under the ignored `.rerail/e2e/`.
 */
export default defineConfig({
  testDir: "./e2e",
  outputDir: ".rerail/e2e/test-results",
  fullyParallel: true,
  forbidOnly: process.env.CI !== undefined,
  retries: process.env.CI === undefined ? 0 : 1,
  reporter: process.env.CI === undefined ? "list" : [["list"], ["html", { open: "never", outputFolder: ".rerail/e2e/report" }]],
  use: {
    trace: "retain-on-failure",
  },
  projects: [{
    name: "chromium",
    use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 }, locale: "en-US", timezoneId: "UTC" },
  }],
});
