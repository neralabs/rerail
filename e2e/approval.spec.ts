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
 * Open rule `id` in Created by you and approve it from its sheet, after it is read. Its notice and its button say what
 * approving it does: a rule set to steer steers ("Approve and steer"), one set to Log stays on Log only ("Approve").
 * Beside Approve is the quiet way to say no, the sheet's own delete. Once approved, the sheet stays on the rule and
 * says so, with what approving did.
 */
async function approveInCreated(page: Page, id: string): Promise<void> {
  await page.locator(`#content .row[data-i="${id}"] button.t`).click();
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("Added by your coding agent, on Log only.");
  await expect(notice).toContainText(id === LOG_LIBRARY_RULE
    ? "It’s set to Log only, so Rerail keeps recording what it catches without telling your coding agent."
    : "approve it, and Rerail tells your coding agent as it works, so it fixes what this rule catches.");
  await expect(notice.locator('[data-act="approve"]')).toHaveText(id === LOG_LIBRARY_RULE ? "Approve" : "Approve and steer");
  await expect(notice.locator('[data-act="delgo"]')).toHaveText("Delete rule");
  await notice.locator('[data-act="approve"]').click();
  await expect(page.locator("#toast")).toContainText(`${id} is approved`);
  const approved = sheet(page).locator(".welcome.approved");
  await expect(approved).toContainText("Approved.");
  await expect(approved).toContainText(id === LOG_LIBRARY_RULE ? "It stays on Log only, as it’s set." : "It now steers your coding agent from its next turn.");
}

/**
 * Approve the edited copy `copy` from its open sheet. Its notice says which rule it edits and that the original keeps
 * running until then, with Discard edit beside Approve; once approved, that the edit replaced the original.
 */
async function approveEdit(page: Page, copy: string, of: string): Promise<void> {
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("Edited by your coding agent, on Log only.");
  await expect(notice).toContainText(of);
  await expect(notice).toContainText("until you approve this edit; approving it turns the original off there.");
  await expect(notice.locator('[data-act="delgo"]')).toHaveText("Discard edit");
  await notice.locator('[data-act="approve"]').click();
  await expect(page.locator("#toast")).toContainText(`${copy} is approved`);
  await expect(sheet(page).locator(".welcome.approved")).toContainText("This edit replaced the original in");
}

test("what your coding agent added and edited waits for approval: Overview sends you to each, approved on its page", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval"));

  // Overview counts the rules the coding agent added and the edits it made in one banner. The worktree's own rule stays
  // in the banner for what someone else wrote, which sends you to each one's page.
  await expect(yours(page)).toContainText("5 rules wait for your approval: 3 added, 2 edited. They’re on Log only until you approve them.");
  await expect(yours(page)).toContainText("An edited rule’s original keeps running until then.");
  await expect(yours(page).locator('[data-act="reviewcreated"]')).toHaveText("Review next rule");
  await expect(banner(page)).toContainText("1 rule waits for approval");
  await expect(banner(page)).toContainText("Rerail did not write it. Read what it tells the coding agent, then approve it on its page.");
  await expect(banner(page)).not.toContainText("/rerail-create");
  await expect(banner(page).locator('[data-act="detail"]')).toHaveCount(1);
  // A rule that would steer is held in Log by the approval; one written as Log runs in Log by its own default.
  for (const id of [LIBRARY_RULE, "storefront-no-inner-html"]) {
    expect(await modeOf(rerail, id)).toEqual({ mode: "log", by: "awaiting-approval", approved: false });
  }
  expect(await modeOf(rerail, "storefront-no-default-export")).toEqual({ mode: "log", by: "rule-default", approved: false });

  // Its one button goes to Created by you with the sheet of the next waiting rule open, which says where it stands
  // among the five; each is read and approved there.
  await yours(page).locator('[data-act="reviewcreated"]').click();
  await expect(page).toHaveURL(/#rule\//u);
  await expect(page.locator(`#content .row[data-i="${LIBRARY_RULE}"]`)).toBeVisible();
  await expect(sheet(page).locator(".welcome.warn .queuepos")).toHaveText("1 of 5 waiting");
  await approveInCreated(page, LIBRARY_RULE);
  expect(await modeOf(rerail, LIBRARY_RULE)).toEqual({ mode: "steer", by: "rule-default", approved: true });
  // The sheet stays on the approved rule and offers the next one waiting.
  await expect(sheet(page).locator(".welcome.approved")).toContainText("4 more rules wait for your approval.");
  await expect(sheet(page).locator('.welcome.approved [data-act="detail"]')).toHaveText("Review next rule");

  // The rest of the added rules: each runs in the mode it is set to once approved.
  for (const id of MORE_LIBRARY_RULES) await approveInCreated(page, id);
  expect(await modeOf(rerail, "storefront-no-inner-html")).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await modeOf(rerail, "storefront-no-default-export")).toEqual({ mode: "log", by: "rule-default", approved: true });
  // Approved is the one notice that can be dismissed.
  await sheet(page).locator('.welcome.approved [data-act="dismissapproved"]').click();
  await expect(sheet(page).locator(".welcome.approved")).toHaveCount(0);
  await sheet(page).locator('[data-act="close"]').click();

  // The two edits still wait, and Overview still asks for them.
  await page.locator('[data-act="nav"][data-i="overview"]').first().click();
  await expect(yours(page)).toContainText("Your coding agent edited 2 rules. The edits are on Log only until you approve them.");

  // The worktree's own rule is approved from its page, after it is read.
  await banner(page).locator(`[data-act="detail"][data-i="${WORKTREE_RULE}"]`).click();
  const notice = sheet(page).locator(".welcome.warn");
  await expect(notice).toContainText("This rule waits for approval.");
  await notice.locator('[data-act="approve"]').click();

  await expect(page.locator("#toast")).toContainText(`${WORKTREE_RULE} is approved`);
  expect(await modeOf(rerail, WORKTREE_RULE)).toEqual({ mode: "steer", by: "pack-start", approved: true });
  await sheet(page).locator('[data-act="close"]').click();
  await expect(banner(page)).toHaveCount(0);

  // The By Rerail rule the coding agent edited reads Edited on its pack's page. Its sheet says the edit waits for
  // approval and that the rule runs as it was, with Review the edit, which opens the copy. Nothing dismisses it.
  await page.goto(await rerail.dashboard("awaiting-approval", { hash: "pack/typescript" }));
  const original = page.locator(`#content .row[data-i="${RERAIL_RULE}"]`);
  await expect(original.locator(".badge", { hasText: "Edited" })).toBeVisible();
  await original.locator("button.t").click();
  const edited = sheet(page).locator(".welcome.warn");
  await expect(edited).toContainText("An edit of this rule waits for approval.");
  await expect(edited).toContainText(/Edited (today|yesterday)\./u);
  await expect(edited.locator(".x")).toHaveCount(0);
  await expect(edited.locator('[data-act="approve"]')).toHaveCount(0);
  await expect(edited.locator('[data-act="detail"]')).toHaveAttribute("data-i", RERAIL_RULE_COPY);

  // Review the edit opens the copy's sheet, where it is approved; approving it turns the original off.
  expect(await entryOf(rerail, RERAIL_RULE)).toMatchObject({ on: true });
  await edited.locator('[data-act="detail"]').click();
  await approveEdit(page, RERAIL_RULE_COPY, "a rule Rerail ships.");
  expect(await modeOf(rerail, RERAIL_RULE_COPY)).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await entryOf(rerail, RERAIL_RULE)).toMatchObject({ on: false });
  await expect(original.locator(".badge", { hasText: "Edited" })).toBeHidden();
  await sheet(page).locator('[data-act="close"]').click();

  // A coding agent's edit of your approved rule: your rule steers until the copy is approved from its page.
  expect(await entryOf(rerail, OWN_RULE)).toMatchObject({ on: true, mode: "steer" });
  await page.locator('#sidenav [data-i="created"]').click();
  await page.locator(`#content .row[data-i="${OWN_RULE_COPY}"] button.t`).click();
  await approveEdit(page, OWN_RULE_COPY, "An edited copy of your rule “");
  expect(await modeOf(rerail, OWN_RULE_COPY)).toEqual({ mode: "steer", by: "rule-default", approved: true });
  expect(await entryOf(rerail, OWN_RULE)).toMatchObject({ on: false });
  await sheet(page).locator('[data-act="close"]').click();
  await page.locator('#sidenav [data-i="overview"]').click();
  await expect(banner(page)).toHaveCount(0);
  await expect(yours(page)).toHaveCount(0);
});
