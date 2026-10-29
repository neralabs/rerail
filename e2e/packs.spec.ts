import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** One rule as `GET /api/repository/rules` reports it. */
interface EffectiveRule { rule: { id: string }; folder: string; on: boolean; source: { kind: string; id: string } }
interface PackRule { id: string; on?: boolean }
interface Pack { id: string; name: string; category: string; rules: PackRule[] }

/** The rules pack `packId` adds in the repository `url` is scoped to, as the daemon resolves them for that scope. */
async function packRulesAt(page: Page, url: string, packId: string): Promise<EffectiveRule[]> {
  const response = await page.request.get(new URL(`/api/repository/rules${new URL(url).search}`, url).href);
  expect(response.ok()).toBe(true);
  const body = await response.json() as { rules: EffectiveRule[] };
  return body.rules.filter(entry => entry.source.kind === "pack" && entry.source.id === packId);
}

async function installedPacks(page: Page, url: string): Promise<Pack[]> {
  const response = await page.request.get(new URL("/api/rule-packs", url).href);
  return (await response.json() as { packs: Pack[] }).packs;
}

const onCount = (rules: readonly EffectiveRule[]): number => rules.filter(entry => entry.on).length;
/** The page's whole-pack switch for `name`. */
const packSwitch = (page: Page, name: string) => page.getByRole("switch", { name: new RegExp(`^Turn all \\d+ ${name.replace(/[.()]/gu, "\\$&")} rules`, "u") });
const ruleSwitches = (page: Page) => page.locator("#content [role=switch][data-act=toggle]");

test.describe("rule pack pages and the catalog over mature", () => {
  // One seeded daemon per worker: the writes of one test are the next one's starting point.
  test.describe.configure({ mode: "serial" });

  test("a pack page counts its rules on, and its whole-pack switch reads off while some are off", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature", { hash: "pack/typescript" });
    const rules = await packRulesAt(page, url, "typescript");
    const on = onCount(rules);
    expect(on).toBeGreaterThan(0);
    expect(on).toBeLessThan(rules.length);

    await page.goto(url);

    await expect(page.getByRole("heading", { name: "TypeScript", exact: true })).toBeVisible();
    await expect(page.locator("#content")).toContainText(`${on} of ${rules.length} on`);
    await expect(ruleSwitches(page)).toHaveCount(rules.length);
    await expect(packSwitch(page, "TypeScript")).toHaveAttribute("aria-checked", "false");
    await expect(page.locator("#content .packall")).toHaveText("All");

    await page.getByLabel("Filter rules").selectOption("off");
    await expect(ruleSwitches(page)).toHaveCount(rules.length - on);
    await expect(page.locator("#content")).toContainText(`${rules.length - on} of ${rules.length} shown`);
    await page.getByLabel("Filter rules").selectOption("all");
  });

  test("the whole-pack switch turns the rest of a partly-on pack on, then all off, and both survive a reload", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature", { hash: "pack/typescript" });
    await page.goto(url);
    await expect(packSwitch(page, "TypeScript")).toHaveAttribute("aria-checked", "false");

    await packSwitch(page, "TypeScript").click();
    await expect.poll(async () => { const rules = await packRulesAt(page, url, "typescript"); return onCount(rules) === rules.length; }).toBe(true);
    await page.reload();
    await expect(packSwitch(page, "TypeScript")).toHaveAttribute("aria-checked", "true");
    await expect(page.locator('#content [data-act=toggle][aria-checked="false"]')).toHaveCount(0);

    await packSwitch(page, "TypeScript").click();
    // Off includes the rule the scenario switched on with a setting of its own (no-non-null-assertion).
    await expect.poll(async () => onCount(await packRulesAt(page, url, "typescript"))).toBe(0);
    await page.reload();
    await expect(packSwitch(page, "TypeScript")).toHaveAttribute("aria-checked", "false");
    await expect(page.locator('#content [data-act=toggle][aria-checked="true"]')).toHaveCount(0);
  });

  test("one rule's switch turns that rule alone on; the pack switch is on only once every rule is", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature", { hash: "pack/typescript" });
    await page.goto(url);
    const off = page.locator('#content [data-act=toggle][aria-checked="false"]').first();
    const id = await off.getAttribute("data-i");
    expect(id).not.toBeNull();
    const before = onCount(await packRulesAt(page, url, "typescript"));

    await off.click();

    await expect.poll(async () => (await packRulesAt(page, url, "typescript")).find(entry => entry.rule.id === id)?.on).toBe(true);
    expect(onCount(await packRulesAt(page, url, "typescript"))).toBe(before + 1);
    await page.reload();
    await expect(page.locator(`#content [data-act=toggle][data-i="${id}"]`)).toHaveAttribute("aria-checked", "true");
    const total = (await packRulesAt(page, url, "typescript")).length;
    await expect(packSwitch(page, "TypeScript")).toHaveAttribute("aria-checked", before + 1 === total ? "true" : "false");
  });

  test("Remove pack asks first, then removes the whole pack", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature", { hash: "pack/complexity" });
    expect((await packRulesAt(page, url, "complexity")).length).toBeGreaterThan(0);
    await page.goto(url);
    const name = (await installedPacks(page, url)).find(pack => pack.id === "complexity")!.name;
    await expect(page.locator("#sidenav")).toContainText(name);

    await page.locator("#content").getByRole("button", { name: "Remove pack" }).click();
    const question = page.getByRole("alertdialog");
    await expect(question).toBeVisible();
    expect((await packRulesAt(page, url, "complexity")).length).toBeGreaterThan(0);
    await question.getByRole("button", { name: "Remove", exact: true }).click();

    await expect.poll(async () => (await packRulesAt(page, url, "complexity")).length).toBe(0);
    await page.reload();
    await expect(page.locator("#sidenav")).not.toContainText(name);
    await expect(page.locator("#content").getByRole("button", { name: "Add pack" })).toBeVisible();
  });

  test("the catalog filters by kind, alphabetical; adds Python whole with its opt-in rules off; its sheet removes it", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature", { hash: "packs" });
    const packs = (await installedPacks(page, url)).filter(pack => pack.rules.length > 0);
    const python = packs.find(pack => pack.id === "python")!;
    expect(await packRulesAt(page, url, "python")).toHaveLength(0);
    await page.goto(url);
    const rowNames = () => page.locator("#content [data-act=viewpack]").evaluateAll(rows => rows.map(row => row.querySelector("b")?.textContent ?? ""));
    const alphabetical = (names: string[]) => [...names].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));

    await expect.poll(rowNames).toEqual(alphabetical(packs.map(pack => pack.name)));
    await page.getByRole("button", { name: "Framework", exact: true }).click();
    await expect.poll(rowNames).toEqual(alphabetical(packs.filter(pack => pack.category === "framework").map(pack => pack.name)));
    await page.getByRole("button", { name: "All", exact: true }).click();

    const row = page.locator("#content [data-act=viewpack]").filter({ has: page.locator("b", { hasText: /^Python$/u }) });
    await row.getByRole("button", { name: "Add pack" }).click();

    // Whole: every rule of the pack, each on or off as the pack starts it.
    const startsOn = python.rules.filter(rule => rule.on !== false).map(rule => rule.id).sort();
    await expect.poll(async () => (await packRulesAt(page, url, "python")).length).toBe(python.rules.length);
    expect((await packRulesAt(page, url, "python")).filter(entry => entry.on).map(entry => entry.rule.id).sort()).toEqual(startsOn);
    await page.reload();
    await expect(page.locator("#sidenav")).toContainText("Python");
    await expect(row.getByRole("button", { name: "Remove pack" })).toBeVisible();

    await page.goto(await rerail.dashboard("mature", { hash: "pack/python" }));
    await expect(page.locator("#content")).toContainText(`${startsOn.length} of ${python.rules.length} on`);
    await expect(packSwitch(page, "Python")).toHaveAttribute("aria-checked", startsOn.length === python.rules.length ? "true" : "false");

    await page.goto(url);
    await row.locator("b", { hasText: /^Python$/u }).click();
    const sheet = page.locator("#panel");
    await expect(sheet.getByRole("heading", { name: "Python" })).toBeVisible();
    await sheet.getByRole("button", { name: "Remove pack" }).click();
    await sheet.getByRole("alertdialog").getByRole("button", { name: "Remove", exact: true }).click();
    await expect.poll(async () => (await packRulesAt(page, url, "python")).length).toBe(0);
    await page.reload();
    await expect(row.getByRole("button", { name: "Add pack" })).toBeVisible();
  });

  test("a catalog row asks on a line of its own, then reads Removed with Undo until the page is left", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature", { hash: "packs" });
    await page.goto(url);
    const row = page.locator("#content .prow2").filter({ has: page.locator("b", { hasText: /^Python$/u }) });
    await row.getByRole("button", { name: "Add pack" }).click();
    await expect.poll(async () => (await packRulesAt(page, url, "python")).length).toBeGreaterThan(0);

    await row.getByRole("button", { name: "Remove pack" }).click();
    // The question replaces the row's state and action, under the pack's text.
    await expect(row.locator(".pend")).toHaveCount(0);
    await row.locator(".rmline").getByRole("button", { name: "Remove", exact: true }).click();
    await expect.poll(async () => (await packRulesAt(page, url, "python")).length).toBe(0);
    await expect(row.locator(".pend")).toHaveText(/Removed\s*Undo/u);

    await row.getByRole("button", { name: "Undo" }).click();
    await expect.poll(async () => (await packRulesAt(page, url, "python")).length).toBeGreaterThan(0);
    await expect(row.getByRole("button", { name: "Remove pack" })).toBeVisible();

    await row.getByRole("button", { name: "Remove pack" }).click();
    await row.locator(".rmline").getByRole("button", { name: "Remove", exact: true }).click();
    await expect(row.getByRole("button", { name: "Undo" })).toBeVisible();
    await page.locator('#sidenav [data-i="overview"]').click();
    await page.locator('#sidenav [data-i="packs"]').click();
    await expect(row.getByRole("button", { name: "Add pack" })).toBeVisible();
  });
});

test("multi-repo: the whole-pack switch writes to the repository the page shows, not the others", async ({ page, rerail }) => {
  const { manifest } = await rerail.scenario("multi-repo");
  const billing = manifest.repositories.find(repo => repo.repositoryKey !== manifest.primary.repositoryKey && repo.packIds.includes("typescript"))!;
  const billingUrl = await rerail.dashboard("multi-repo", { repository: billing.repositoryKey, hash: "pack/typescript" });
  const storefrontUrl = await rerail.dashboard("multi-repo", { hash: "pack/typescript" });
  const storefrontBefore = (await packRulesAt(page, storefrontUrl, "typescript")).map(entry => [entry.rule.id, entry.on]);
  const billingRules = await packRulesAt(page, billingUrl, "typescript");
  expect(onCount(billingRules)).toBeLessThan(billingRules.length);

  await page.goto(billingUrl);
  await expect(page.locator("#content")).toContainText(`${onCount(billingRules)} of ${billingRules.length} on`);
  await packSwitch(page, "TypeScript").click();

  await expect.poll(async () => onCount(await packRulesAt(page, billingUrl, "typescript"))).toBe(billingRules.length);
  expect((await packRulesAt(page, storefrontUrl, "typescript")).map(entry => [entry.rule.id, entry.on])).toEqual(storefrontBefore);
  await page.reload();
  await expect(packSwitch(page, "TypeScript")).toHaveAttribute("aria-checked", "true");
});
