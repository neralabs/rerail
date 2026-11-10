import { expect, SCENARIO_NAMES, test } from "./dashboard-test.js";

for (const scenario of SCENARIO_NAMES) {
  test(`${scenario}: the dashboard loads and the daemon answers /healthz`, async ({ page, rerail }) => {
    const url = await rerail.dashboard(scenario);

    await page.goto(url);

    await expect(page).toHaveTitle("Rerail");
    await expect(page.locator("#status")).toBeVisible();
    const health = await rerail.get(new URL("/healthz", url).href);
    expect(health.ok()).toBe(true);
    expect(await health.json()).toMatchObject({ status: "ok", ui: "dev" });
  });
}
