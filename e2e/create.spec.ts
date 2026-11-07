import type { Page } from "@playwright/test";
import { expect, test, type ScenarioName } from "./dashboard-test.js";

/** The packs the daemon recommends for the scenario's primary checkout that are not added there. */
async function unaddedRecommendations(page: Page, url: string, packIds: readonly string[]): Promise<string[]> {
  const scope = new URL(url).searchParams;
  const response = await page.request.get(new URL(`/api/rule-packs/recommendations?${scope}`, url).href);
  expect(response.ok()).toBe(true);
  const { recommendations } = await response.json() as { recommendations: { packId: string }[] };
  return [...new Set(recommendations.map(r => r.packId).filter(id => !packIds.includes(id)))].sort();
}

for (const scenario of ["mature", "packs-no-session", "no-packs"] satisfies ScenarioName[]) {
  test(`${scenario}: Create a rule shows the command, and suggests only recommended packs not added`, async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario(scenario);
    const url = await rerail.dashboard(scenario, { hash: "create" });
    const expected = await unaddedRecommendations(page, url, manifest.primary.packIds);
    await page.goto(url);

    const content = page.locator("#content");
    await expect(content.locator("h1")).toBeVisible();
    await expect(content.locator(".cmd code").first()).toHaveText("/rerail-create");

    const places = content.locator(".places");
    if (expected.length === 0) {
      await expect(places).toHaveCount(0);
      return;
    }
    await expect(places.locator('[data-act="addpack"]')).toHaveCount(expected.length);
    const shown = await places.locator('[data-act="addpack"]').evaluateAll(buttons => buttons.map(b => b.getAttribute("data-i") ?? ""));
    expect(shown.sort()).toEqual(expected);
  });
}

test("the create command copies to the clipboard", async ({ page, rerail }) => {
  const url = await rerail.dashboard("mature", { hash: "create" });
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(url).origin });
  await page.goto(url);

  await page.locator('#content .cmd [data-act="cmdcopy"]').first().click();

  await expect(page.locator("#toast")).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("/rerail-create");
});
