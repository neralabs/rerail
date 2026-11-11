import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/*
 * The dashboard opened without `?repository=` or `?checkout=` (a plain browser, a bookmark): a coding agent always
 * passes one, so the page cannot know where the person should look. With one repository seen it shows that one;
 * with several it lists them to pick from, under the pill that switches between them.
 */

const pill = (page: Page) => page.locator("#repopick .repopill");
const listed = (page: Page) => page.locator("#content .norepo .repolist [role=\"option\"]");
const menuOptions = (page: Page) => page.locator("#repolayer .repomenu [role=\"option\"]");

test("one repository seen: the page shows it, without asking, keeping the view", async ({ page, rerail }) => {
  const { origin, manifest } = await rerail.scenario("packs-no-session");
  const only = manifest.primary;

  await page.goto(`${origin}/#history`);

  await expect(page).toHaveURL(url => url.searchParams.get("repository") === only.repositoryKey && url.hash === "#history");
  await expect(pill(page)).toHaveText(only.displayName);
  await expect(page.locator("#content .norepo")).toHaveCount(0);
  await expect(page.locator("#content .history")).toBeVisible();
});

test("several repositories seen: the page lists them, and the pill switches between them", async ({ page, rerail }) => {
  const { origin, manifest } = await rerail.scenario("multi-repo");
  // The list and the menu are by name.
  const names = manifest.repositories.map(repository => repository.displayName).sort();

  await page.goto(origin);

  await expect(page).toHaveURL(url => url.searchParams.get("repository") === null);
  await expect(pill(page)).toHaveText("Pick a repo");
  await expect(page.locator("#content .norepo .welcome")).toContainText("No repo selected");
  await expect(listed(page).locator("b")).toHaveText(names);
  await expect(listed(page).locator(".rline")).toHaveText(names.map(() => /^(Set up|Not set up) · /u));

  await pill(page).click();
  await expect(menuOptions(page).locator("b")).toHaveText(names);
  await page.keyboard.press("Escape");

  const picked = manifest.repositories[1];
  if (picked === undefined) throw new Error("multi-repo seeds several repositories");
  await listed(page).filter({ hasText: picked.displayName }).click();
  await expect(page).toHaveURL(url => url.searchParams.get("repository") === picked.repositoryKey);
  await expect(pill(page)).toHaveText(picked.displayName);
});
