import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

const content = (page: Page) => page.locator("#content");
const ruleRows = (page: Page) => content(page).locator(".hist aside .hrow");
const violationRows = (page: Page) => content(page).locator(".hist .ev");
const rangeButton = (page: Page, days: number) => content(page).locator(`.rangectl button[data-i="${days}"]`);
const listedIds = async (page: Page) => ruleRows(page).evaluateAll(rows => rows.map(row => row.getAttribute("data-i")));

test.describe("History by rule over a mature repository", () => {
  test("the range changes which rules are listed, ranked by violations", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "history" }));
    await expect(ruleRows(page).first()).toBeVisible();

    await rangeButton(page, 0).click();
    await expect(rangeButton(page, 0)).toHaveAttribute("aria-pressed", "true");
    // eslint-no-debugger logs only today; no-empty-object-type only more than 30 days ago.
    await expect(ruleRows(page).and(page.locator('[data-i="eslint-no-debugger"]'))).toBeVisible();
    const today = await listedIds(page);
    expect(today).toContain("eslint-no-debugger");
    expect(today).not.toContain("typescript-eslint-no-empty-object-type");

    await rangeButton(page, 9999).click();
    await expect(ruleRows(page).and(page.locator('[data-i="typescript-eslint-no-empty-object-type"]'))).toBeVisible();
    const all = await listedIds(page);
    expect(all.length).toBeGreaterThan(today.length);
    const counts = await ruleRows(page).locator(".c b").allTextContents();
    const numbers = counts.map(text => Number(text.replace(/,/g, "")));
    expect(numbers).toEqual([...numbers].sort((a, b) => b - a));
  });

  test("selecting a rule shows its violations and survives a reload", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "history" }));
    const row = ruleRows(page).and(page.locator('[data-i="eslint-eqeqeq"]'));
    await row.click();

    await expect(row).toHaveAttribute("aria-current", "true");
    await expect(page).toHaveURL(/#history\?rule=eslint-eqeqeq$/);
    await expect(content(page).locator(".hist .hdetail")).toContainText("eslint-eqeqeq");
    await expect(violationRows(page).first()).toBeVisible();

    // The header carries the mode and what the agent is told; rows carry time (24-hour), path and session, and the
    // message only where it differs from the header's.
    const { origin, manifest } = await rerail.scenario("mature");
    const api = new URL("/api/violations", origin);
    api.search = new URLSearchParams({ ruleId: "eslint-eqeqeq", since: "0", limit: "1", repository: manifest.primary.repositoryKey }).toString();
    const { violations } = await (await page.request.get(api.href)).json() as { violations: { message: string }[] };
    await expect(content(page).locator(".hist .hkv .oc-blocked")).toBeVisible();
    await expect(content(page).locator(".hist .hkv .said2")).toHaveText(violations[0]!.message);
    await expect(violationRows(page).first().locator(".when")).toHaveText(/^\d{2}:\d{2}$/);
    await expect(violationRows(page).first().locator(".loc")).not.toBeEmpty();
    await expect(violationRows(page).first().locator(".diff")).toBeHidden();

    await page.reload();
    await expect(ruleRows(page).and(page.locator('[data-i="eslint-eqeqeq"]'))).toHaveAttribute("aria-current", "true");
  });

  test("Load more pages through every violation of the rule in the range", async ({ page, rerail }) => {
    const { origin, manifest } = await rerail.scenario("mature");
    const api = new URL("/api/violations", origin);
    api.search = new URLSearchParams({ ruleId: "eslint-no-console", since: "0", limit: "1", repository: manifest.primary.repositoryKey }).toString();
    const { total } = await (await page.request.get(api.href)).json() as { total: number };
    expect(total).toBeGreaterThan(25);

    await page.goto(await rerail.dashboard("mature", { hash: "history?rule=eslint-no-console" }));
    await rangeButton(page, 9999).click();
    await expect(violationRows(page)).toHaveCount(25);

    const more = content(page).locator(".hist .more button");
    while (await more.isVisible()) {
      const before = await violationRows(page).count();
      await more.click();
      await expect.poll(() => violationRows(page).count()).toBeGreaterThan(before);
    }
    await expect(violationRows(page)).toHaveCount(total);
  });

  test("a legacy #activity?rule= link shows that rule", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "activity?rule=eslint-eqeqeq" }));

    await expect(ruleRows(page).and(page.locator('[data-i="eslint-eqeqeq"]'))).toHaveAttribute("aria-current", "true");
    await expect(page).toHaveURL(/#history\?rule=eslint-eqeqeq$/);
  });

  test("a violation's session link opens that session", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "history" }));
    const open = violationRows(page).first().locator(".open");
    const sessionId = await open.getAttribute("data-i");
    expect(sessionId).toBeTruthy();

    await open.click();

    await expect(page).toHaveURL(/#sessions$/);
    expect(new URL(page.url()).searchParams.get("sessionId")).toBe(sessionId);
  });

  test("the search box narrows the rule list by title, id or pack", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "history" }));
    await rangeButton(page, 9999).click();
    await expect(ruleRows(page).and(page.locator('[data-i="typescript-eslint-no-empty-object-type"]'))).toBeVisible();
    const all = await listedIds(page);

    await content(page).locator("#hq").fill("ESLINT-NO-");
    await expect.poll(() => listedIds(page)).toEqual(all.filter(id => id?.toLowerCase().includes("eslint-no-")));

    await content(page).locator("#hq").fill("no rule is called this");
    await expect(ruleRows(page)).toHaveCount(0);
    await expect(content(page).locator(".hnomatch")).toBeVisible();

    await content(page).locator("#hq").fill("");
    await expect.poll(() => listedIds(page)).toEqual(all);
  });

  test("with the list hidden, the heading picks the rule", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "history?rule=eslint-no-console" }));
    await expect(ruleRows(page).first()).toBeVisible();

    await content(page).locator(".hltoggle").click();
    await expect(content(page).locator(".hist aside")).toBeHidden();
    const pick = content(page).locator("#hpick");
    await expect(pick).toHaveValue("eslint-no-console");

    await pick.selectOption("eslint-eqeqeq");
    await expect(page).toHaveURL(/#history\?rule=eslint-eqeqeq$/);
    await expect(content(page).locator(".hist .hdetail .l3")).toContainText("eslint-eqeqeq");
    await expect(violationRows(page).first()).toBeVisible();

    await content(page).locator(".hltoggle").click();
    await expect(ruleRows(page).and(page.locator('[data-i="eslint-eqeqeq"]'))).toHaveAttribute("aria-current", "true");
  });
});

test.describe("History by rule before anything was logged", () => {
  test("no rules on: says to add a rule pack or create a rule", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("no-packs", { hash: "history" }));

    await expect(content(page).locator(".hempty")).toBeVisible();
    await expect(content(page).locator(".hempty")).toContainText("Add a rule pack");
    await expect(content(page).locator(".hist")).toBeHidden();
  });

  test("rules on, no session yet: says violations show once a session logs one", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("packs-no-session", { hash: "history" }));

    await expect(content(page).locator(".hempty")).toBeVisible();
    await expect(content(page).locator(".hempty")).not.toContainText("Add a rule pack");
    await expect(content(page).locator(".hist")).toBeHidden();
  });
});
