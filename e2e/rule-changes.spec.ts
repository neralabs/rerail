import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** A By Rerail rule, and a coding agent's edited copy of it among the rules created by you. */
const RERAIL_RULE = "typescript-eslint-no-explicit-any";
const RERAIL_RULE_COPY = `${RERAIL_RULE}-copy`;

const notice = (page: Page) => page.locator("#content .ov .welcome.changed");
const source = (page: Page) => page.locator("#panel dl.kv");

test("a rule added and approved outside the dashboard: Overview counts it, asks for nothing, and stays dismissed", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval"));

  // Of the rules created by you in the last hour one is approved, so it runs as set: the notice counts it. The ones
  // that wait are the approval banners', which stay.
  await expect(notice(page)).toContainText("Your rules changed: 1 added.");
  await expect(notice(page)).toContainText("You can delete it from its details.");
  await expect(notice(page)).not.toHaveClass(/warn/u);
  await expect(page.locator("#content .ov .welcome.yours")).toBeVisible();

  // Dismissed, it stays away over a reload; the approval banner does not go with it.
  await notice(page).locator('[data-act="dismissrulechanges"]').click();
  await expect(notice(page)).toHaveCount(0);
  await page.reload();
  await expect(page.locator("#content .ov .welcome.yours")).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
});

test("an edited copy's details say which rule it was edited from, with a link to it", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("awaiting-approval", { hash: `rule/${RERAIL_RULE_COPY}` }));

  await expect(source(page)).toContainText("Last edited");
  await expect(source(page)).toContainText("Edited from");
  await source(page).locator(`[data-act="detail"][data-i="${RERAIL_RULE}"]`).click();
  await expect(source(page)).toContainText(RERAIL_RULE);
  await expect(source(page)).not.toContainText("Edited from");
});
