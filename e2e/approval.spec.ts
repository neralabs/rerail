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

/** Open rule `id` in Created by you and approve it from its sheet, after it is read. */
async function approveInCreated(page: Page, id: string): Promise<void> {
  await page.locator(`#content .row[data-i="${id}"] button.t`).click();
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("Added by your coding agent, on Log only.");
  await notice.locator('[data-act="approve"]').click();
  await expect(page.locator("#toast")).toContainText(`${id} is approved`);
}

test("rules your coding agent added wait for approval: Overview sends you to them, each approved on its page", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval"));

  // Overview says how many rules the coding agent added; the worktree's own rule stays in the banner that sends you to
  // its page.
  await expect(yours(page)).toContainText("Your coding agent added 3 rules. They’re on Log only until you approve them.");
  await expect(banner(page)).toContainText("1 rule waits for approval");
  // A rule that would steer is held in Log by the approval; one written as Log runs in Log by its own default.
  for (const id of [LIBRARY_RULE, "storefront-no-inner-html"]) {
    expect(await modeOf(rerail, id)).toEqual({ mode: "log", by: "awaiting-approval", approved: false });
  }
  expect(await modeOf(rerail, "storefront-no-default-export")).toEqual({ mode: "log", by: "rule-default", approved: false });

  // Its one button goes to Created by you, where each rule is read and approved on its sheet.
  await yours(page).locator('[data-act="nav"][data-i="created"]').click();
  await approveInCreated(page, LIBRARY_RULE);
  expect(await modeOf(rerail, LIBRARY_RULE)).toEqual({ mode: "steer", by: "rule-default", approved: true });

  // The rest: each runs in the mode it is set to once approved.
  for (const id of MORE_LIBRARY_RULES) await approveInCreated(page, id);
  expect(await modeOf(rerail, "storefront-no-inner-html")).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await modeOf(rerail, "storefront-no-default-export")).toEqual({ mode: "log", by: "rule-default", approved: true });
  await sheet(page).locator('[data-act="close"]').click();

  await page.locator('[data-act="nav"][data-i="overview"]').first().click();
  await expect(yours(page)).toHaveCount(0);

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
