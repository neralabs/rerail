import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** The list row for one session. */
const rowOf = (page: Page, id: string) => page.locator(`#content .srow[data-i="${id}"]`);
/** The selected session's short id in the header's meta line; its tooltip is the full id. */
const headerId = (page: Page) => page.locator("#content .shead .l3 .mono");

test.describe("History by session", () => {
  test("mature: the open session is selected first and reads live, with its counts on the row", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const open = manifest.sessionIds.at(-1)!;

    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));

    const row = rowOf(page, open);
    await expect(row).toHaveAttribute("aria-current", "true");
    await expect(row.locator(".state")).toHaveClass(/\blive\b/);
    await expect(page.locator("#content .shead .live")).toBeVisible();
    // The lead counts the loaded sessions against the total the daemon reports.
    await expect(page.locator("#content .hhead p")).toContainText(`of ${manifest.sessionIds.length}`);
    // Every row with violations logged says how many of them were steered.
    const withViolations = page.locator("#content .srow .c:not(.zero)");
    await expect(withViolations.first()).toBeVisible();
    await expect(withViolations.first().locator(".st")).toHaveText(/^\d+ steered$/);
  });

  test("mature: a session from Overview's recent sessions opens selected in History by session", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature"));
    const link = page.locator('#content [data-act="session"]').last();
    const id = await link.getAttribute("data-i");
    expect(id).toBeTruthy();

    await link.click();

    await expect(page).toHaveURL(new RegExp(`sessionId=${id}.*#sessions`));
    await expect(rowOf(page, id!)).toHaveAttribute("aria-current", "true");
    await expect(page.locator("#content .shead .mono")).not.toBeEmpty();
  });

  test("mature: a deep link with sessionId and host selects that session, and survives a reload", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const target = manifest.sessionIds[manifest.sessionIds.length - 2]!;
    const url = new URL(await rerail.dashboard("mature", { hash: "sessions" }));
    url.searchParams.set("sessionId", target);
    url.searchParams.set("host", target.split("-").slice(2).join("-"));

    await page.goto(url.href);
    await expect(rowOf(page, target)).toHaveAttribute("aria-current", "true");
    await expect(headerId(page)).toHaveAttribute("title", target);

    await page.reload();
    await expect(headerId(page)).toHaveAttribute("title", target);
  });

  test("mature: Hide list folds the list into a heading picker that still switches sessions; Show list brings it back", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const other = manifest.sessionIds[0]!;
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const aside = page.locator("#content .sess > aside");
    await expect(aside).toBeVisible();

    await page.locator('#content [data-act="slisttoggle"]').click();
    await expect(aside).toBeHidden();
    const picker = page.locator("#hpick");
    await expect(picker).toBeVisible();
    await picker.selectOption(other);
    await expect(headerId(page)).toHaveAttribute("title", other);

    await page.locator('#content [data-act="slisttoggle"]').click();
    await expect(aside).toBeVisible();
    await expect(picker).toBeHidden();
    await expect(rowOf(page, other)).toHaveAttribute("aria-current", "true");
  });

  test("mature: the resume box opens from the header, which shows it open, and closes from its close button", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const box = page.locator("#content .pask.sresume");
    const resume = page.locator('#content .shead .acts [data-act="resumetoggle"]');
    await resume.click();
    await expect(box).toBeVisible();
    await expect(resume).toHaveClass(/\bopen\b/u);
    await box.locator('button.x[data-act="resumetoggle"]').click();
    await expect(box).toBeHidden();
    await expect(resume).toHaveClass(/\bsolid\b/u);
  });

  test("mature: the header counts the violations, and the rule dropdown filters the timeline in both Show all events states", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const cards = page.locator("#content .tl .corr");
    const rule = page.locator("#srule");
    await expect(cards.first()).toBeVisible();
    const total = await cards.count();
    await expect(page.locator("#content .shead .l3")).toContainText(`${total} violation`);
    await expect(rule.locator("option").first()).toHaveText(`All rules · ${total}`);

    // The first rule listed is the one that logged most here; filtering keeps only its cards.
    const [id, count] = await rule.locator("option").nth(1).evaluate(o => [(o as HTMLOptionElement).value, Number(o.textContent!.split(" · ").at(-1))] as const);
    await rule.selectOption(id);
    await expect(cards).toHaveCount(count);
    for (const mono of await cards.locator(".ch .mono").allTextContents()) expect(mono).toBe(id);
    await page.locator("#showAll").check();
    await expect(rule).toHaveValue(id);
    await expect(cards).toHaveCount(count);
    await rule.selectOption("");
    await expect(cards).toHaveCount(total);
  });

  test("no-packs: no sessions shows the empty state and no list or transcript", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("no-packs", { hash: "sessions" }));

    await expect(page.locator("#content .hhead h2")).toBeVisible();
    await expect(page.locator("#content .empty h3")).toBeVisible();
    await expect(page.locator("#content .sess")).toBeHidden();
    await expect(page.locator('#content [data-act="slisttoggle"]')).toBeHidden();
  });
});
