import type { Page } from "@playwright/test";
import { DAY, expect, SEED_NOW, test } from "./dashboard-test.js";

const content = (page: Page) => page.locator("#content");
const tileValue = (page: Page, key: string) => content(page).locator(`[data-tile="${key}"] .v`);
const rangeButton = (page: Page, days: number) => content(page).locator(`.rollhead [data-act="range"][data-i="${days}"]`);
const topRows = (page: Page) => content(page).locator(".ovtop .row");
const recentRows = (page: Page) => content(page).locator(".ovrecent .sesrow");

/** The mature scenario's figures per range, as GET /api/rules/activity measures them at SEED_NOW (e2e harness). */
const MATURE = [
  { days: 0, steered: 14, total: 21, sessions: 4 },
  { days: 7, steered: 31, total: 53, sessions: 12 },
  { days: 30, steered: 42, total: 97, sessions: 27 },
  { days: 9999, steered: 45, total: 106, sessions: 30 },
];

const sinceOf = (days: number) => (days === 0 ? Date.UTC(2026, 9, 22) : days === 9999 ? 0 : SEED_NOW - days * DAY);

async function api<T>(page: Page, url: string, path: string): Promise<T> {
  const response = await page.request.get(new URL(path, url).href);
  expect(response.ok()).toBe(true);
  return await response.json() as T;
}

test.describe("mature", () => {
  test("tiles follow the range: steered of all violations, sessions with violations", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature"));

    for (const { days, steered, total, sessions } of MATURE) {
      await rangeButton(page, days).click();
      await expect(rangeButton(page, days)).toHaveAttribute("aria-pressed", "true");
      await expect(tileValue(page, "steered")).toHaveText(`${steered} of ${total}`);
      // Every mature session logs a violation.
      await expect(tileValue(page, "sessions")).toHaveText(`${sessions} of ${sessions}`);
    }
    await expect(content(page).locator(".welcome")).toHaveCount(0);
  });

  test("tiles open History, Sessions and the rule packs", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature"));
    await expect(tileValue(page, "steered")).toHaveText("31 of 53");

    await content(page).locator('[data-tile="steered"]').click();
    await expect(page).toHaveURL(/#history$/u);
    await page.goto(await rerail.dashboard("mature"));
    await content(page).locator('[data-tile="sessions"]').click();
    await expect(page).toHaveURL(/#sessions$/u);
    await page.goto(await rerail.dashboard("mature"));
    // Two rules created by you: Rules on opens them, the first rule source in the sidebar.
    await content(page).locator('[data-tile="on"]').click();
    await expect(page).toHaveURL(/#created$/u);
  });

  test("Most violations logged ranks the range's rules and opens a rule's sheet", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature");
    const { manifest } = await rerail.scenario("mature");
    const activity = await api<{ rules: Record<string, { distinctViolations: number }> }>(page, url,
      `/api/rules/activity?since=${sinceOf(7)}&repository=${encodeURIComponent(manifest.primary.repositoryKey)}`);
    const counts = Object.values(activity.rules).map(a => a.distinctViolations).filter(n => n > 0).sort((a, b) => b - a);

    await page.goto(url);
    await expect(topRows(page)).toHaveCount(Math.min(10, counts.length));
    const shown = (await topRows(page).locator(".count b").allTextContents()).map(Number);
    expect(shown).toEqual(counts.slice(0, 10));

    const first = topRows(page).first();
    const id = await first.getAttribute("data-i");
    await first.locator(".rmeta").click();
    await expect(page).toHaveURL(new RegExp(`#rule/${id}$`, "u"));
    await expect(page.locator("#panel")).toBeVisible();
  });

  test("a rule's switch on Overview saves", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature"));
    const row = topRows(page).first();
    const id = await row.getAttribute("data-i");
    const sw = content(page).locator(`.ovtop .row[data-i="${id}"] .sw`);
    const before = await sw.getAttribute("aria-checked");
    const after = before === "true" ? "false" : "true";

    await sw.click();
    await expect(sw).toHaveAttribute("aria-checked", after);
    await expect(page).not.toHaveURL(/#rule\//u);
    await expect(page.locator("#toast")).toHaveClass(/show/u);
    await page.reload();
    await expect(sw).toHaveAttribute("aria-checked", after);

    // Put it back for the other tests in this worker.
    await sw.click();
    await expect(sw).toHaveAttribute("aria-checked", before!);
    await page.reload();
    await expect(sw).toHaveAttribute("aria-checked", before!);
  });

  test("Recent sessions: the range's five newest with their counts, and View all", async ({ page, rerail }) => {
    const url = await rerail.dashboard("mature");
    const { manifest } = await rerail.scenario("mature");
    const sessions = await api<{ total: number; sessions: Array<{ sessionId: string; violationCount: number; steeredViolations: number }> }>(page, url,
      `/api/sessions?since=${sinceOf(7)}&limit=5&repository=${encodeURIComponent(manifest.primary.repositoryKey)}`);

    await page.goto(url);
    await expect(recentRows(page)).toHaveCount(5);
    expect(await recentRows(page).evaluateAll(rows => rows.map(r => r.getAttribute("data-i")))).toEqual(sessions.sessions.map(s => s.sessionId));
    expect((await recentRows(page).locator(".n.logged b").allTextContents()).map(Number)).toEqual(sessions.sessions.map(s => s.violationCount));
    expect((await recentRows(page).locator(".n.steered b").allTextContents()).map(Number)).toEqual(sessions.sessions.map(s => s.steeredViolations));
    // The open session's last event is two minutes old, so it reads live.
    await expect(recentRows(page).first().locator(".when")).toHaveClass(/live/u);
    await expect(content(page).locator(".ovrecent .ghead .act button")).toContainText(String(sessions.total));

    // Today holds four sessions.
    await rangeButton(page, 0).click();
    await expect(recentRows(page)).toHaveCount(4);
    await expect(content(page).locator(".ovrecent .ghead .act button")).toContainText("4");

    const newest = manifest.primary.openSessionIds[0]!;
    await content(page).locator(`.ovrecent .sesrow[data-i="${newest}"]`).click();
    await expect(page).toHaveURL(new RegExp(`sessionId=${newest}.*#sessions$`, "u"));
  });
});

test("packs-no-session: the welcome banner, dismissed for good, and rules waiting for a session", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("packs-no-session"));
  const welcome = content(page).locator(".welcome");

  await expect(welcome).toHaveCount(1);
  await expect(welcome).not.toHaveClass(/warn/u);
  await expect(content(page).locator(".ovempty .empty")).toBeVisible();
  await expect(content(page).locator(".ovlists")).toBeHidden();
  await expect(tileValue(page, "steered")).toHaveText("0");

  await welcome.locator('[data-act="dismisswelcome"]').click();
  await expect(welcome).toHaveCount(0);
  await page.reload();
  await expect(content(page).locator(".ovempty .empty")).toBeVisible();
  await expect(content(page).locator(".welcome")).toHaveCount(0);
});

test("no-packs: no banner; the empty block offers Add rule packs and Create a rule", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("no-packs"));
  const empty = content(page).locator(".ovempty .empty");

  await expect(empty).toBeVisible();
  await expect(content(page).locator(".welcome")).toHaveCount(0);
  await expect(empty.locator('[data-act="nav"][data-i="create"]')).toBeVisible();
  await empty.locator('[data-act="nav"][data-i="packs"]').click();
  await expect(page).toHaveURL(/#packs$/u);
});

test("stopped: the banner starts Rerail, and it stays started", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("stopped"));
  const banner = content(page).locator(".welcome.warn");

  await expect(banner).toBeVisible();
  await banner.locator('[data-act="repostart"]').click();
  await expect(banner).toHaveCount(0);
  await page.reload();
  await expect(content(page).locator(".rollup")).toBeVisible();
  await expect(content(page).locator(".welcome.warn")).toHaveCount(0);
});

test("not-set-up: set up in the page with the recommended packs", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("not-set-up"));
  const setup = content(page).locator('[data-act="setup"]');

  await expect(content(page).locator(".notsetup")).toBeVisible();
  await expect(content(page).locator('.notsetup [data-act="nav"][data-i="packs?review=1"]')).toBeVisible();
  await setup.click();
  await expect(content(page).locator(".notsetup")).toHaveCount(0);
  await page.reload();
  await expect(content(page).locator(".rollup")).toBeVisible();
  await expect(content(page).locator(".notsetup")).toHaveCount(0);
});

test("multi-repo: without a repo the page lists every repo Rerail has seen, and picking one opens its Overview", async ({ page, rerail }) => {
  const { manifest } = await rerail.scenario("multi-repo");
  const scoped = new URL(await rerail.dashboard("multi-repo"));
  scoped.search = "";
  scoped.hash = "";
  await page.goto(scoped.href);

  const options = content(page).locator('.repolist [role="option"]');
  await expect(options).toHaveCount(manifest.repositories.length);
  const billing = manifest.repositories.find(r => r.repositoryKey.endsWith("acme/billing"))!;
  await content(page).locator(`.repolist [data-i="${billing.repositoryKey}"]`).click();

  await expect(page).toHaveURL(new RegExp(`repository=${encodeURIComponent(billing.repositoryKey)}`, "u"));
  await expect(content(page).locator(".rollup")).toBeVisible();
  // Set up with packs, no session yet: the welcome banner.
  await expect(content(page).locator(".welcome")).toHaveCount(1);
});
