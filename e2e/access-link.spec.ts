import { expect, test } from "./dashboard-test.js";

// A browser that never opened the dashboard link: no capability in its storage, as after a bookmark or in a new
// profile. It gets the page and is told what to do; the link then admits it, and leaves the address clean.
test("a fresh browser needs the dashboard link once, which leaves no capability in the address", async ({ browser, rerail }) => {
  const { origin, accessToken } = await rerail.scenario("mature");
  const dashboard = await rerail.dashboard("mature", { hash: "settings" });
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.goto(dashboard);
    await expect(page.locator("#status")).toContainText("Needs the dashboard link");
    // The page's own files load, but no data: no repository is listed, and the page says what to run.
    await expect(page.locator("#content .access")).toContainText("rerail manage");
    await expect(page.locator("#content [data-repo]")).toHaveCount(0);

    const link = new URL(dashboard);
    link.searchParams.set("rerail-access", accessToken);
    await page.goto(link.href);
    await expect(page.locator("#content [data-repo]").first()).toBeVisible();
    expect(new URL(page.url()).searchParams.has("rerail-access")).toBe(false);
    expect(new URL(page.url()).hash).toBe("#settings");
    // Kept for this origin alone: a reload, or a bookmark of this port, needs no link again.
    await page.goto(`${origin}/#settings`);
    await expect(page.locator("#content [data-repo]").first()).toBeVisible();
    expect(await context.cookies()).toEqual([]);
  } finally {
    await context.close();
  }
});

// A link without the capability, as copied from a coding agent's session or from another browser's address bar: no
// view may read as a repository with nothing in it, or load forever.
test("a link without the capability says what to run on every view, not that the repository is empty", async ({ browser, rerail }) => {
  const context = await browser.newContext();
  try {
    for (const hash of ["overview", "pack/typescript", "history", "sessions"]) {
      const page = await context.newPage();
      await page.goto(await rerail.dashboard("mature", { hash }));
      const content = page.locator("#content");
      await expect(content.locator(".access"), hash).toContainText("This browser doesn’t have access to Rerail’s data yet.");
      await expect(content.locator(".access .mono"), hash).toHaveText("rerail manage");
      await expect(content, hash).not.toContainText("Loading");
      await expect(content, hash).not.toContainText("Rules on");
      await page.close();
    }
  } finally {
    await context.close();
  }
});
