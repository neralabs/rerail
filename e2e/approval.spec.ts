import type { Page } from "@playwright/test";
import { expect, test, type RerailScenarios } from "./dashboard-test.js";

const LIBRARY_RULE = "storefront-no-raw-fetch";
const WORKTREE_RULE = "storefront-no-legacy-client";

const banner = (page: Page) => page.locator("#content .ov .welcome.warn", { hasText: "for approval" });
const sheet = (page: Page) => page.locator("#panel");

/** The mode the API resolves for rule `id` in the scenario's checkout, what decided it, and whether it is approved. */
async function modeOf(rerail: RerailScenarios, id: string): Promise<unknown> {
  const { manifest, origin } = await rerail.scenario("awaiting-approval");
  const response = await rerail.get(`${origin}/api/repository/rules?${new URLSearchParams({ checkout: manifest.primary.checkoutPath })}`);
  const body = await response.json() as {
    rules: Array<{ rule: { id: string }; mode: string; modeSource: { by: string }; approval?: { approved: boolean } }>;
  };
  const rule = body.rules.find(entry => entry.rule.id === id)!;
  return { mode: rule.mode, by: rule.modeSource.by, approved: rule.approval?.approved };
}

test("a rule Rerail did not write runs in Log until it is approved from its page, a worktree's own rule too", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval"));

  // Overview names both, each opening the page where it is approved.
  await expect(banner(page)).toContainText("2 rules wait for approval");
  expect(await modeOf(rerail, LIBRARY_RULE)).toEqual({ mode: "log", by: "awaiting-approval", approved: false });
  expect(await modeOf(rerail, WORKTREE_RULE)).toEqual({ mode: "log", by: "awaiting-approval", approved: false });

  await banner(page).locator(`[data-act="detail"][data-i="${LIBRARY_RULE}"]`).click();
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("This rule waits for approval.");
  await notice.locator('[data-act="approve"]').click();

  await expect(page.locator("#toast")).toContainText(`${LIBRARY_RULE} is approved`);
  await expect(notice).toHaveCount(0);
  expect(await modeOf(rerail, LIBRARY_RULE)).toEqual({ mode: "steer", by: "rule-default", approved: true });

  await sheet(page).locator('[data-act="close"]').click();
  await expect(banner(page)).toContainText("1 rule waits for approval");
  await banner(page).locator(`[data-act="detail"][data-i="${WORKTREE_RULE}"]`).click();
  await sheet(page).locator('.welcome.warn [data-act="approve"]').click();

  await expect(page.locator("#toast")).toContainText(`${WORKTREE_RULE} is approved`);
  expect(await modeOf(rerail, WORKTREE_RULE)).toEqual({ mode: "steer", by: "pack-start", approved: true });
  await sheet(page).locator('[data-act="close"]').click();
  await expect(banner(page)).toHaveCount(0);
});
