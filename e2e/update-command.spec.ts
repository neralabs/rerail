import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** A version newer than any the daemon under test runs, reported as `/healthz.update` while the page polls. */
const LATEST = "99.0.0";

/** Open `hash` with `/healthz` reporting `LATEST` as out, and the clipboard granted. */
async function openWithUpdate(page: Page, url: string): Promise<void> {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(url).origin });
  await page.route("**/healthz", async route => {
    const response = await route.fetch();
    await route.fulfill({ response, json: { ...await response.json(), update: { latest: LATEST, checkedAt: 0 } } });
  });
  await page.goto(url);
}

const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

test("Update available: Copy copies /rerail-update, reads Copied, and leaves the popover open", async ({ page, rerail }) => {
  await openWithUpdate(page, await rerail.dashboard("mature", { hash: "overview" }));

  await page.locator("#status .vnew").click();
  const copy = page.locator('.updpop [data-act="cmdcopy"]');
  await copy.click();

  await expect(copy).toHaveText("Copied");
  expect(await clipboard(page)).toBe("/rerail-update");
  await expect(copy).toHaveText("Copy");
  await expect(page.locator(".updpop")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".updpop")).toHaveCount(0);
  await expect(page.locator("#status .vnew")).toBeFocused();
});

test("Overview's update banner: Copy update command reads Copied", async ({ page, rerail }) => {
  await openWithUpdate(page, await rerail.dashboard("mature", { hash: "overview" }));

  const copy = page.locator('#content .welcome [data-act="cmdcopy"]');
  await copy.click();

  await expect(copy).toHaveText("Copied");
  expect(await clipboard(page)).toBe("/rerail-update");
  await expect(copy).toHaveText("Copy update command");
});

test("Settings: Copy update command copies rerail update and reads Copied", async ({ page, rerail }) => {
  await openWithUpdate(page, await rerail.dashboard("mature", { hash: "settings" }));

  const copy = page.locator('#content #about [data-act="cmdcopy"]');
  await copy.click();

  await expect(copy).toHaveText("Copied");
  expect(await clipboard(page)).toBe("rerail update");
  await expect(copy).toHaveText("Copy update command");
});
