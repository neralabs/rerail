import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** The rule `pack-rule-not-running` installs as the complexity pack published for an earlier Rerail wrote it. */
const NOT_RUNNING = "complexity-cyclomatic-python";
/** A rule of the same pack that runs. */
const RUNNING = "complexity-cyclomatic-typescript";
/** Why the rule does not run; the pack's version, which the sentence names, is left out. */
const REASON = /This Rerail no longer runs the rule as Function complexity \S+ writes it: it tests a token that parsed files now leave out\. The pack's next version runs it again\./u;

const row = (page: Page, id: string) => page.locator(`#content .row[data-i="${id}"]`);
const sheet = (page: Page) => page.locator("#panel");

test("a pack rule this Rerail no longer runs shows Not running in its row, with why in its hover", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("pack-rule-not-running", { hash: "pack/complexity" }));

  const badge = row(page, NOT_RUNNING).locator(".rmeta .badge", { hasText: "Not running" });
  await expect(badge).toBeVisible();
  await expect(badge).toHaveAttribute("title", REASON);
  // Its enforcement tag says it does nothing, and its switch still says On.
  const tag = row(page, NOT_RUNNING).locator(".enftag");
  await expect(tag).toHaveClass(/\bnone\b/u);
  await expect(tag).toHaveText("Not running");
  await expect(tag).toHaveAttribute("title", REASON);
  await expect(row(page, NOT_RUNNING).locator('[role="switch"]')).toHaveAttribute("aria-checked", "true");

  // The pack's other rules run.
  await expect(row(page, RUNNING).locator(".rmeta .badge", { hasText: "Not running" })).toBeHidden();
  await expect(row(page, RUNNING).locator(".enftag")).not.toHaveClass(/\bnone\b/u);
});

test("its sheet says it is on but not running, and why", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("pack-rule-not-running", { hash: `rule/${NOT_RUNNING}` }));

  const notice = sheet(page).locator(".welcome.warn", { hasText: "On, but not running." });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText(REASON);
  await expect(sheet(page).locator('[role="switch"]')).toHaveAttribute("aria-checked", "true");
  await expect(sheet(page)).toContainText("Nothing is logged or steered while it does not run.");
});

test("a pack rule that runs has no such notice in its sheet", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("pack-rule-not-running", { hash: `rule/${RUNNING}` }));

  await expect(sheet(page)).toContainText(RUNNING);
  await expect(sheet(page)).not.toContainText("On, but not running.");
});
