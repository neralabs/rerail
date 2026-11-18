import type { Page } from "@playwright/test";
import { expect, test, type RerailScenarios } from "./dashboard-test.js";

const LIBRARY_RULE = "storefront-no-raw-fetch";
const MORE_LIBRARY_RULES = ["storefront-no-inner-html", "storefront-no-default-export"];
const WORKTREE_RULE = "storefront-no-legacy-client";

const banner = (page: Page) => page.locator("#content .ov .welcome.warn", { hasText: "for approval" });
const yours = (page: Page) => page.locator("#content .ov .welcome.yours");
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

test("rules created by you wait for approval on the Overview, approved one by one or all at once", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval"));

  // Overview lists the three rules a coding agent added, each with what it tells the agent; the worktree's own rule
  // stays in the banner that sends you to its page.
  await expect(yours(page)).toContainText("3 rules you created wait for your approval");
  await expect(yours(page).locator(`li[data-rule="${LIBRARY_RULE}"]`)).toContainText("Call the API through the generated client in src/api/generated.");
  await expect(banner(page)).toContainText("1 rule waits for approval");
  for (const id of [LIBRARY_RULE, ...MORE_LIBRARY_RULES]) {
    expect(await modeOf(rerail, id)).toEqual({ mode: "log", by: "awaiting-approval", approved: false });
  }

  // Approve, for one.
  await yours(page).locator(`li[data-rule="${LIBRARY_RULE}"] [data-act="approve"]`).click();
  await expect(page.locator("#toast")).toContainText(`${LIBRARY_RULE} is approved`);
  await expect(yours(page)).toContainText("2 rules you created wait for your approval");
  expect(await modeOf(rerail, LIBRARY_RULE)).toEqual({ mode: "steer", by: "rule-default", approved: true });

  // Approve all, for the rest: each runs in the mode it is set to.
  await yours(page).locator('[data-act="approveall"]').click();
  await expect(page.locator("#toast")).toContainText("2 rules are approved");
  await expect(yours(page)).toHaveCount(0);
  expect(await modeOf(rerail, "storefront-no-inner-html")).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await modeOf(rerail, "storefront-no-default-export")).toEqual({ mode: "log", by: "rule-default", approved: true });

  // The worktree's own rule is approved from its page, after it is read.
  await banner(page).locator(`[data-act="detail"][data-i="${WORKTREE_RULE}"]`).click();
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("This rule waits for approval.");
  await notice.locator('[data-act="approve"]').click();

  await expect(page.locator("#toast")).toContainText(`${WORKTREE_RULE} is approved`);
  expect(await modeOf(rerail, WORKTREE_RULE)).toEqual({ mode: "steer", by: "pack-start", approved: true });
  await sheet(page).locator('[data-act="close"]').click();
  await expect(banner(page)).toHaveCount(0);
});
