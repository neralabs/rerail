import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

// Dates on the sheet are local; the seeded instants are UTC.
test.use({ timezoneId: "UTC" });

const PACK_RULE = "eslint-no-console";
const STEER_LIBRARY_RULE = "storefront-no-raw-fetch";
const BLOCK_LIBRARY_RULE = "storefront-money-as-cents";

const sheet = (page: Page) => page.locator("#panel");
const ruleRow = (page: Page, id: string) => page.locator(`#content [data-act="detail"][data-i="${id}"]`);
const sheetSwitch = (page: Page) => sheet(page).locator('[role="switch"]');

test("a rule opens from Overview without moving the top bar's status", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("mature"));
  const status = page.locator("#status");
  await expect(ruleRow(page, PACK_RULE).first()).toBeVisible();
  const before = await status.boundingBox();

  await ruleRow(page, PACK_RULE).first().click();

  await expect(sheet(page)).toContainText(PACK_RULE);
  await expect(page).toHaveURL(new RegExp(`#rule/${PACK_RULE}$`));
  expect(await status.boundingBox()).toEqual(before);
});

test("a rule opens from its pack's page, and a pack rule has no Delete", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("mature", { hash: "pack/javascript" }));

  await ruleRow(page, PACK_RULE).first().click();

  await expect(sheet(page)).toContainText(PACK_RULE);
  await expect(sheet(page).locator('[data-act="delask"]')).toHaveCount(0);
  await expect(sheet(page).locator(".pfoot")).toBeHidden();
  // On Log the agent is told nothing yet; the sheet says so under what it would see.
  await expect(sheet(page).locator(".sees .note2")).toBeVisible();
});

test("recent violations show where and when, and open their session", async ({ page, rerail }) => {
  const { manifest } = await rerail.scenario("mature");
  await page.goto(await rerail.dashboard("mature", { hash: `rule/${STEER_LIBRARY_RULE}` }));
  const rows = sheet(page).locator(".ract .rrow");
  await expect(rows.first()).toBeVisible();
  expect(await rows.count()).toBeLessThanOrEqual(3);
  await expect(rows.first().locator(".file")).not.toBeEmpty();

  const session = rows.first().locator('[data-act="session"]');
  const sessionId = await session.getAttribute("data-i");
  expect(manifest.primary.sessionIds).toContain(sessionId);
  await session.click();
  await expect(sheet(page)).toBeHidden();
  await expect(page).toHaveURL(/#sessions/);
});

test("the switch in the sheet saves the rule's on, across a reload", async ({ page, rerail }) => {
  const id = "typescript-eslint-no-empty-object-type";
  await page.goto(await rerail.dashboard("mature", { hash: `rule/${id}` }));
  const toggle = sheetSwitch(page);
  await expect(toggle).toHaveAttribute("aria-checked", /true|false/);
  const was = await toggle.getAttribute("aria-checked");
  const flipped = was === "true" ? "false" : "true";

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", flipped);
  await expect(page.locator("#toast")).toBeVisible();
  await page.reload();
  await expect(sheetSwitch(page)).toHaveAttribute("aria-checked", flipped);

  // Put it back: the scenario is shared with the worker's later tests.
  await sheetSwitch(page).click();
  await expect(sheetSwitch(page)).toHaveAttribute("aria-checked", was ?? "true");
});

test("Source shows when a rule created by you was added, with the year", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("mature", { hash: `rule/${BLOCK_LIBRARY_RULE}` }));
  // Added 30 days before the seed clock (Oct 22, 2026).
  await expect(sheet(page).locator("dl.kv")).toContainText("Sep 22, 2026");
});

test("Edit with coding agent and the agent-sees Edit hand off prompts, one box at a time", async ({ page, rerail }) => {
  const url = await rerail.dashboard("mature", { hash: `rule/${STEER_LIBRARY_RULE}` });
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(url).origin });
  await page.goto(url);
  const askBox = sheet(page).locator(".sask");
  const draft = page.locator("#agentDraft");
  const said = sheet(page).locator(".sees .said2");
  await expect(said).not.toBeEmpty();
  const message = await said.innerText();

  // The prompt box: each chip gives its own prompt, naming the rule.
  await sheet(page).locator('[data-act="asktoggle"]').first().click();
  await expect(askBox).toBeVisible();
  const prompts = new Set<string>();
  for (const chip of await askBox.locator('[data-act="askintent"]').all()) {
    await chip.click();
    await expect(chip).toHaveAttribute("aria-pressed", "true");
    const prompt = await page.locator("#askText").inputValue();
    expect(prompt).toContain(STEER_LIBRARY_RULE);
    prompts.add(prompt);
  }
  expect(prompts.size).toBe(4);

  // Edit on What the agent sees closes the prompt box, and opens a draft of the current text.
  await sheet(page).locator('[data-act="promptedit"]').click();
  await expect(askBox).toBeHidden();
  await expect(draft).toHaveValue(message);
  await draft.fill("Call the API through the generated client in src/api.");
  await sheet(page).locator('[data-act="promptcopy"]').click();
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toContain(`"${STEER_LIBRARY_RULE}"`);
  expect(copied).toContain("Call the API through the generated client in src/api.");

  // Opening the prompt box closes the draft.
  await sheet(page).locator(".pact [data-act='asktoggle']").click();
  await expect(askBox).toBeVisible();
  await expect(draft).toHaveCount(0);

  // The page wrote nothing: the agent still sees the rule's text after a reload.
  await page.reload();
  await expect(sheet(page).locator(".sees .said2")).toHaveText(message);
});

test("a rule created by you is deleted from the repository after a confirm, and Undo adds it back", async ({ page, rerail }) => {
  const url = await rerail.dashboard("mature");
  await page.goto(url);
  await ruleRow(page, BLOCK_LIBRARY_RULE).first().click();

  await sheet(page).locator('[data-act="delask"]').click();
  const confirm = sheet(page).locator(".pdelbox");
  await expect(confirm).toBeVisible();
  await expect(confirm.locator("li")).not.toHaveCount(0);
  await confirm.locator('[data-act="delgo"]').click();

  await expect(page).not.toHaveURL(/#rule\//);
  await expect(ruleRow(page, BLOCK_LIBRARY_RULE)).toHaveCount(0);
  // Saved: a fresh page of the same repository no longer lists it.
  const other = await page.context().newPage();
  await other.goto(url);
  await expect(ruleRow(other, STEER_LIBRARY_RULE).first()).toBeVisible();
  await expect(ruleRow(other, BLOCK_LIBRARY_RULE)).toHaveCount(0);
  await other.close();

  await page.locator('#toast [data-act="ruleundo"]').click();
  await expect(ruleRow(page, BLOCK_LIBRARY_RULE).first()).toBeVisible();
  await page.reload();
  await expect(ruleRow(page, BLOCK_LIBRARY_RULE).first()).toBeVisible();
});
