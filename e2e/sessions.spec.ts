import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** The list row for one session. */
const rowOf = (page: Page, id: string) => page.locator(`#content .srow[data-i="${id}"]`);

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
    await expect(page.locator("#content .shead [data-act=copyid]")).toHaveAttribute("data-i", target);

    await page.reload();
    await expect(page.locator("#content .shead [data-act=copyid]")).toHaveAttribute("data-i", target);
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
    await expect(page.locator("#content .shead [data-act=copyid]")).toHaveAttribute("data-i", other);

    await page.locator('#content [data-act="slisttoggle"]').click();
    await expect(aside).toBeVisible();
    await expect(picker).toBeHidden();
    await expect(rowOf(page, other)).toHaveAttribute("aria-current", "true");
  });

  test("mature: the resume box opens from the header and closes from its close button", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const box = page.locator("#content .pask.sresume");
    await page.locator('#content .shead .acts [data-act="resumetoggle"]').click();
    await expect(box).toBeVisible();
    await box.locator('button.x[data-act="resumetoggle"]').click();
    await expect(box).toBeHidden();
  });

  test("no-packs: no sessions shows the empty state and no list or transcript", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("no-packs", { hash: "sessions" }));

    await expect(page.locator("#content .hhead h2")).toBeVisible();
    await expect(page.locator("#content .empty h3")).toBeVisible();
    await expect(page.locator("#content .sess")).toBeHidden();
    await expect(page.locator('#content [data-act="slisttoggle"]')).toBeHidden();
  });
});
