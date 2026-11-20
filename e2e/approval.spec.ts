import type { Page } from "@playwright/test";
import { expect, test, type RerailScenarios } from "./dashboard-test.js";

const LIBRARY_RULE = "storefront-no-raw-fetch";
/** The one library rule written as Log: it stays on Log only once approved. */
const LOG_LIBRARY_RULE = "storefront-no-default-export";
const MORE_LIBRARY_RULES = ["storefront-no-inner-html", LOG_LIBRARY_RULE];
const WORKTREE_RULE = "storefront-no-legacy-client";
/** A By Rerail rule, and the coding agent's edited copy of it: approving the copy turns the original off here. */
const RERAIL_RULE = "typescript-eslint-no-explicit-any";
const RERAIL_RULE_COPY = `${RERAIL_RULE}-copy`;
/** Your approved rule, and a coding agent's edited copy of it: approving the copy turns your rule off here. */
const OWN_RULE = "storefront-no-alert";
const OWN_RULE_COPY = `${OWN_RULE}-copy`;

const banner = (page: Page) => page.locator("#content .ov .welcome.warn", { hasText: "for approval" });
const yours = (page: Page) => page.locator("#content .ov .welcome.yours");
const sheet = (page: Page) => page.locator("#panel");

type RuleEntry = { rule: { id: string }; on: boolean; mode: string; modeSource: { by: string }; approval?: { approved: boolean } };

/** Rule `id` as the API resolves it in the scenario's checkout. */
async function entryOf(rerail: RerailScenarios, id: string): Promise<RuleEntry> {
  const { manifest, origin } = await rerail.scenario("awaiting-approval");
  const response = await rerail.get(`${origin}/api/repository/rules?${new URLSearchParams({ checkout: manifest.primary.checkoutPath })}`);
  const body = await response.json() as { rules: RuleEntry[] };
  return body.rules.find(entry => entry.rule.id === id)!;
}

/** The mode the API resolves for rule `id` in the scenario's checkout, what decided it, and whether it is approved. */
async function modeOf(rerail: RerailScenarios, id: string): Promise<unknown> {
  const rule = await entryOf(rerail, id);
  return { mode: rule.mode, by: rule.modeSource.by, approved: rule.approval?.approved };
}

/**
 * Open rule `id` in Created by you and approve it from its sheet, after it is read. Its notice says what approving it
 * does: a rule set to steer steers, one set to Log stays on Log only.
 */
async function approveInCreated(page: Page, id: string): Promise<void> {
  await page.locator(`#content .row[data-i="${id}"] button.t`).click();
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("Added by your coding agent, on Log only.");
  await expect(notice).toContainText(id === LOG_LIBRARY_RULE
    ? "It’s set to Log only, so Rerail keeps recording what it catches without telling your coding agent."
    : "approve it, and Rerail tells your coding agent as it works, so it fixes what this rule catches.");
  await notice.locator('[data-act="approve"]').click();
  await expect(page.locator("#toast")).toContainText(`${id} is approved`);
}

test("rules your coding agent added wait for approval: Overview sends you to them, each approved on its page", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval"));

  // Overview says how many rules the coding agent added. The worktree's own rule, and the edited copy of a By Rerail
  // rule, stay in the banner that sends you to each one's page, which says what approving the copy does.
  await expect(yours(page)).toContainText("Your coding agent added 3 rules. They’re on Log only until you approve them.");
  await expect(banner(page)).toContainText("3 rules wait for approval");
  await expect(banner(page)).toContainText("Approving an edited copy turns off the rule it was copied from in");
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

  // The edited copy is approved from its page too, which says the original turns off; approving it does that.
  expect(await entryOf(rerail, RERAIL_RULE)).toMatchObject({ on: true });
  await banner(page).locator(`[data-act="detail"][data-i="${RERAIL_RULE_COPY}"]`).click();
  const copyNotice = sheet(page).locator(".welcome.warn");
  await expect(copyNotice).toContainText("This is an edited copy");
  await expect(copyNotice).toContainText("a rule Rerail ships. Approving it turns the original off in");
  await copyNotice.locator('[data-act="approve"]').click();

  await expect(page.locator("#toast")).toContainText(`${RERAIL_RULE_COPY} is approved`);
  expect(await modeOf(rerail, RERAIL_RULE_COPY)).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await entryOf(rerail, RERAIL_RULE)).toMatchObject({ on: false });
  await sheet(page).locator('[data-act="close"]').click();

  // A coding agent's edit of your approved rule: your rule steers until the copy is approved from its page.
  expect(await entryOf(rerail, OWN_RULE)).toMatchObject({ on: true, mode: "steer" });
  await banner(page).locator(`[data-act="detail"][data-i="${OWN_RULE_COPY}"]`).click();
  const ownNotice = sheet(page).locator(".welcome.warn");
  await expect(ownNotice).toContainText("This is an edited copy of your rule “");
  await expect(ownNotice).toContainText("Approving it turns the original off in");
  await ownNotice.locator('[data-act="approve"]').click();

  await expect(page.locator("#toast")).toContainText(`${OWN_RULE_COPY} is approved`);
  expect(await modeOf(rerail, OWN_RULE_COPY)).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await entryOf(rerail, OWN_RULE)).toMatchObject({ on: false });
  await sheet(page).locator('[data-act="close"]').click();
  await expect(banner(page)).toHaveCount(0);
});
