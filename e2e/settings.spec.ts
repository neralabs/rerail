import type { Page } from "@playwright/test";
import { expect, test, type RerailScenarios } from "./dashboard-test.js";

/** One repository's rules as `GET /api/repository/rules` reads them: distinct ids, and those on in every folder that run. */
async function ruleCounts(rerail: RerailScenarios, origin: string, key: string): Promise<{ total: number; on: number }> {
  const response = await rerail.get(`${origin}/api/repository/rules?repository=${encodeURIComponent(key)}`);
  expect(response.ok()).toBe(true);
  const { rules } = await response.json() as { rules: Array<{ rule: { id: string }; on: boolean; configError: string | null }> };
  const on = new Map<string, boolean>();
  for (const entry of rules) on.set(entry.rule.id, (on.get(entry.rule.id) ?? true) && entry.on && entry.configError === null);
  return { total: on.size, on: [...on.values()].filter(Boolean).length };
}

async function started(rerail: RerailScenarios, origin: string, key: string): Promise<boolean> {
  const { repositories } = await (await rerail.get(`${origin}/api/repositories`)).json() as { repositories: Array<{ repositoryKey: string; started: boolean }> };
  return repositories.find(repository => repository.repositoryKey === key)!.started;
}

const repoRow = (page: Page, key: string) => page.locator(`#content [data-repo="${key}"]`);

test.describe("Settings over a repo with history", () => {
  test("shows the day Rerail was installed, with its year", async ({ page, rerail }) => {
    const { origin } = await rerail.scenario("mature");
    const { installedAt } = await (await rerail.get(`${origin}/healthz`)).json() as { installedAt: number };
    await page.goto(await rerail.dashboard("mature", { hash: "settings" }));

    const expected = await page.evaluate(ms => new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" }), installedAt);
    expect(expected).toMatch(/\b20\d\d$/u);
    await expect(page.locator("#content")).toContainText(expected);
  });

  test("hiding the status line is kept after a reload", async ({ page, rerail }) => {
    const { origin } = await rerail.scenario("mature");
    await page.goto(await rerail.dashboard("mature", { hash: "settings" }));
    const hide = page.locator('#content [data-act="presence"][data-i="silent"]');

    const saved = page.waitForResponse(response => response.url().endsWith("/api/presentation-preferences") && response.request().method() === "PUT");
    await hide.click();
    expect((await saved).ok()).toBe(true);
    await page.reload();

    await expect(hide).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator('#content [data-act="presence"][data-i="compact"]')).toHaveAttribute("aria-pressed", "false");
    const { preferences } = await (await rerail.get(`${origin}/api/presentation-preferences`)).json() as { preferences: Array<{ repository: string; host: string; presence: string }> };
    expect(preferences.find(p => p.repository === "" && p.host === "")?.presence).toBe("silent");

    // Leave the shared daemon as the scenario seeded it.
    await page.locator('#content [data-act="presence"][data-i="compact"]').click();
    await expect(page.locator('#content [data-act="presence"][data-i="compact"]')).toHaveAttribute("aria-pressed", "true");
  });

  // Last in the file: it removes the scenario's history for every later test of this worker.
  test("clearing history asks first, then leaves the repo's history empty", async ({ page, rerail }) => {
    const { origin, manifest } = await rerail.scenario("mature");
    const key = manifest.primary.repositoryKey;
    const violations = async () => (await (await rerail.get(`${origin}/api/violations?since=0&repository=${encodeURIComponent(key)}`)).json() as { total: number }).total;
    expect(await violations()).toBeGreaterThan(0);
    await page.goto(await rerail.dashboard("mature", { hash: "settings" }));

    await page.locator('#content [data-act="clearask"]').click();
    // The first click only asks: nothing is cleared, and Cancel puts the button back.
    await expect(page.locator('#content [data-act="cleargo"]')).toBeVisible();
    expect(await violations()).toBeGreaterThan(0);
    await page.locator('#content [data-act="clearcancel"]').click();
    await expect(page.locator('#content [data-act="cleargo"]')).toHaveCount(0);

    await page.locator('#content [data-act="clearask"]').click();
    const reset = page.waitForResponse(response => response.url().endsWith("/api/repository/reset"));
    await page.locator('#content [data-act="cleargo"]').click();
    const answer = await reset;
    expect(answer.ok()).toBe(true);
    const { sessions, openSessions } = await answer.json() as { sessions: number; openSessions: number };
    expect(sessions).toBe(manifest.primary.sessionIds.length - manifest.primary.openSessionIds.length);
    expect(openSessions).toBe(manifest.primary.openSessionIds.length);
    await expect(page.locator("#toast")).toContainText(String(sessions));

    await page.goto(await rerail.dashboard("mature", { hash: "history" }));
    await page.reload();
    expect(await violations()).toBe(0);
    const { sessions: left } = await (await rerail.get(`${origin}/api/sessions?repository=${encodeURIComponent(key)}`)).json() as { sessions: Array<{ violationCount: number }> };
    expect(left).toHaveLength(manifest.primary.openSessionIds.length);
    expect(left.every(session => session.violationCount === 0)).toBe(true);
  });
});

test.describe("Settings over several repos", () => {
  test("lists every repo with its own rule count, and a switch only where it has rules", async ({ page, rerail }) => {
    const { origin, manifest } = await rerail.scenario("multi-repo");
    // A direct load of #settings: no other view has polled the rules first, and the counts must still be right.
    await page.goto(await rerail.dashboard("multi-repo", { hash: "settings" }));

    await expect(page.locator("#content [data-repo]")).toHaveCount(manifest.repositories.length);
    expect(manifest.repositories).toHaveLength(3);
    for (const repository of manifest.repositories) {
      const counts = await ruleCounts(rerail, origin, repository.repositoryKey);
      const row = repoRow(page, repository.repositoryKey);
      await expect(row.locator(".n")).toHaveText(counts.total === 0 ? "No rules added" : `${counts.on} ${counts.on === 1 ? "rule" : "rules"} on`);
      await expect(row.locator('[role="switch"]')).toHaveCount(counts.total === 0 ? 0 : 1);
    }
    const neverSetUp = manifest.repositories.find(repository => repository.packIds.length === 0 && repository.libraryRuleIds.length === 0)!;
    await expect(repoRow(page, neverSetUp.repositoryKey).locator(".n")).toHaveText("No rules added");
  });

  test("switching another repo off and on again is kept after a reload", async ({ page, rerail }) => {
    const { origin, manifest } = await rerail.scenario("multi-repo");
    const other = manifest.repositories.find(repository => repository.repositoryKey !== manifest.primary.repositoryKey && repository.packIds.length > 0)!;
    await page.goto(await rerail.dashboard("multi-repo", { hash: "settings" }));
    const toggle = repoRow(page, other.repositoryKey).locator('[role="switch"]');
    await expect(toggle).toHaveAttribute("aria-checked", "true");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    await page.reload();
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(await started(rerail, origin, other.repositoryKey)).toBe(false);
    // Switching it off leaves its rules: the tag still counts them.
    await expect(repoRow(page, other.repositoryKey).locator(".n")).not.toHaveText("No rules added");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "true");
    await page.reload();
    await expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(await started(rerail, origin, other.repositoryKey)).toBe(true);
  });
});
