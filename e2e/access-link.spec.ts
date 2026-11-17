import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** A rule of a pack the `mature` scenario has added. */
const RULE = "eslint-no-console";

/** The poll reads every 5 s: a page recovers, or loses access, on the next read. */
const NEXT_POLL = { timeout: 15_000 };

/** The page that asks to connect this browser, with the rest of the frame hidden. */
async function expectConnectPage(page: Page, label: string): Promise<void> {
  await expect(page.locator("#content .access"), label).toContainText("Connect this browser to Rerail");
  await expect(page.locator("#content .access .cmd code"), label).toHaveText("rerail manage");
  for (const part of ["#sidenav", "#repopick", "#status", "#navToggle"]) await expect(page.locator(part), `${label}: ${part}`).toBeHidden();
}

// A browser that never opened the dashboard link: no capability in its storage, as after a bookmark or in a new
// profile. It gets the page and is told what to do; the link then admits it, and leaves the address clean.
test("a fresh browser needs the dashboard link once, which leaves no capability in the address", async ({ browser, rerail }) => {
  const { origin, accessToken } = await rerail.scenario("mature");
  const dashboard = await rerail.dashboard("mature", { hash: "settings" });
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.goto(dashboard);
    // The page's own files load, but no data: no repository is listed, and the page says what to run.
    await expectConnectPage(page, "settings");
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
test("a link without the capability asks to connect the browser on every view, not that the repository is empty", async ({ browser, rerail }) => {
  const context = await browser.newContext();
  try {
    for (const hash of ["overview", "pack/typescript", "history", "sessions", `rule/${RULE}`]) {
      const page = await context.newPage();
      await page.goto(await rerail.dashboard("mature", { hash }));
      await expectConnectPage(page, hash);
      await expect(page.locator("#content"), hash).not.toContainText("Loading");
      await expect(page.locator("#content"), hash).not.toContainText("Rules on");
      await expect(page.locator("#panel"), hash).toBeHidden();
      await page.close();
    }
  } finally {
    await context.close();
  }
});

// What the page promises: once `rerail manage` connects this browser (here, its link opened in another tab), the tab
// that asked continues where its link pointed, a rule's sheet included, without a reload.
test("a tab that asked to connect the browser continues on its own once another tab connects it", async ({ browser, rerail }) => {
  const { accessToken } = await rerail.scenario("mature");
  const dashboard = await rerail.dashboard("mature", { hash: `rule/${RULE}` });
  const context = await browser.newContext();
  try {
    const waiting = await context.newPage();
    await waiting.goto(dashboard);
    await expectConnectPage(waiting, "before");

    const link = new URL(dashboard);
    link.searchParams.set("rerail-access", accessToken);
    await (await context.newPage()).goto(link.href);

    await expect(waiting.locator("#panel"), "after").toBeVisible(NEXT_POLL);
    await expect(waiting.locator("#panel")).toContainText(RULE);
    await expect(waiting.locator("#content .access")).toHaveCount(0);
    await expect(waiting.locator("#sidenav")).toBeVisible();
    await expect(waiting).toHaveURL(new RegExp(`#rule/${RULE}$`));
  } finally {
    await context.close();
  }
});

// A browser that loses the capability while a rule's sheet is open (site data cleared, the daemon's capability
// rotated) still holds what it read: the page that asks to connect it shows alone, and the sheet comes back with it.
test("a browser that loses the capability shows only the page that asks to connect it, and gets its sheet back", async ({ page, rerail }) => {
  const { accessToken } = await rerail.scenario("mature");
  await page.goto(await rerail.dashboard("mature", { hash: `rule/${RULE}` }));
  await expect(page.locator("#panel")).toContainText(RULE);

  await page.evaluate(() => localStorage.removeItem("rerail-access"));
  await expect(page.locator("#content .access")).toBeVisible(NEXT_POLL);
  await expectConnectPage(page, "lost");
  await expect(page.locator("#panel")).toBeHidden();

  // The hidden sheet keeps its text: what shows it again is the next read.
  await page.evaluate(token => localStorage.setItem("rerail-access", token), accessToken);
  await expect(page.locator("#panel")).toBeVisible(NEXT_POLL);
  await expect(page.locator("#panel")).toContainText(RULE);
  await expect(page.locator("#content .access")).toHaveCount(0);
});
