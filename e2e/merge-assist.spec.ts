import type { Page, Request } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** What the `merge-assist` scenario puts in other people's summary, hunk and the dismissed conflict's title. */
const HOSTILE = `<img src=x onerror="window.__rerailInjected=1"><b>bold</b>`;
const OPEN_ID = "conflict:src/cart/cart.ts#1-3:pr398";
const PR_URL = "https://github.com/acme/storefront/pull/398";

const conflicts = (page: Page) => page.locator('#content section[data-section="conflicts"]');
const cards = (page: Page) => conflicts(page).locator("article.mcard");
const dismissedFold = (page: Page) => conflicts(page).locator("details.mfold");
/** The sidebar's count on Merge Assist: the open conflicts that are not quiet. */
const sidebarCount = (page: Page) => page.locator('#sidenav [data-act="nav"][data-i="merge"] .n');
const isWrite = (path: string, method: string) => (request: Request) => request.url().endsWith(path) && request.method() === method;

test.describe("Merge Assist", () => {
  test("a person turns it on from the page, and off again", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("merge-assist");
    const billing = manifest.repositories.find(repository => repository.repositoryKey !== manifest.primary.repositoryKey)!;
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge", repository: billing.repositoryKey }));
    const consent = page.locator("#content .mconsent");
    await expect(consent).toContainText("What turning it on does");
    await expect(conflicts(page)).toBeHidden();

    const turnOn = page.waitForRequest(isWrite("/api/merge-assist/settings", "PUT"));
    await page.locator('#content [data-act="maon"]').click();
    expect((await turnOn).postDataJSON()).toEqual({ repository: billing.repositoryKey, checkout: billing.checkoutPath, enabled: true });
    await expect(page.locator("#toast")).toContainText("Merge Assist is on in billing. You and your coding agent are told when your work here collides with work in flight.");
    await expect(consent).toHaveCount(0);
    await expect(page.locator("#content .mstrip")).toContainText("Not scanned yet");
    await expect(conflicts(page)).toContainText("No open conflicts");
    await page.reload();
    await expect(page.locator("#content .mstrip")).toBeVisible();

    // Leave the shared daemon as the scenario seeded it.
    await page.locator('#content [data-act="maoff"]').click();
    await expect(page.locator("#toast")).toContainText("Merge Assist is off in billing. Rerail no longer looks for conflicts here.");
    await expect(consent).toBeVisible();
  });

  test("an open conflict shows both sides, why, and who to talk it over with; a quiet one is muted and not counted", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    await expect(cards(page)).toHaveCount(2);
    await expect(conflicts(page).locator("h2")).toHaveText("Open conflicts2");

    const open = cards(page).nth(0);
    await expect(open).toHaveClass("mcard");
    await expect(open.locator("h3")).toHaveText("cart.ts · function total");
    await expect(open.locator(".mcage")).toHaveText("Open for 3h · Told your agent and you in the session");
    const [mine, theirs] = [open.locator(".mside").nth(0), open.locator(".mside").nth(1)];
    await expect(mine.locator(".k")).toHaveText("Mine: This branch · tbelanger");
    await expect(mine.locator("pre.mhunk span")).toHaveClass(["meta", "ctx", "del", "add"]);
    await expect(theirs.locator(".k")).toHaveText("Theirs: PR #398 · Tim");
    await expect(theirs.locator(".k a")).toHaveAttribute("href", PR_URL);
    await expect(open.locator(".mwhy li")).toHaveText(["Both change src/cart/cart.ts lines 1–3."]);
    await expect(open.locator(".mtalk")).toHaveText("Talk it over with Tim (PR #398). Rerail never picks a side; dismiss it if it isn’t a conflict.");
    await expect(open.locator(".mtalk a")).toHaveAttribute("href", PR_URL);

    const quiet = cards(page).nth(1);
    await expect(quiet).toHaveClass("mcard quiet");
    await expect(quiet.locator("h3")).toHaveText("package-lock.json");
    await expect(quiet.locator(".mquiet")).toHaveText("Quiet: only lockfiles, changelogs, generated output or docs. Not told to your agent.");
    await expect(quiet.locator(".mcage")).not.toContainText("Told your agent");
    // Two open, one counted: the agent is never told of the quiet one.
    await expect(sidebarCount(page)).toHaveText("1");
  });

  test("other people's text shows as text, never as markup", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    const theirs = cards(page).nth(0).locator(".mside").nth(1);
    await expect(theirs.locator(".msum")).toHaveText(HOSTILE);
    await expect(theirs.locator("pre.mhunk")).toContainText(HOSTILE);

    await dismissedFold(page).locator("summary").click();
    await expect(dismissedFold(page).locator("article.mclosed h3")).toHaveText(HOSTILE);
    await expect(conflicts(page).locator("img")).toHaveCount(0);
    await expect(conflicts(page).locator("b:not(.mauthor)")).toHaveCount(0);
    expect(await page.evaluate(() => "__rerailInjected" in window)).toBe(false);
  });

  test("saves the conflict hook command, and refuses one that is not a JSON array without sending it", async ({ page, rerail }) => {
    const { manifest, origin } = await rerail.scenario("merge-assist");
    const { repositoryKey, checkoutPath } = manifest.primary;
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    const settings = page.locator("#content details.msettings");
    const hook = page.locator("#ma-hook"), timeout = page.locator("#ma-timeout"), problem = page.locator("#ma-settings-problem");
    const save = settings.locator('[data-act="masettings"]');
    const saves: Request[] = [];
    page.on("request", request => { if (isWrite("/api/merge-assist/settings", "PUT")(request)) saves.push(request); });
    await expect(settings.locator("summary")).toHaveText("Settings: no conflict hook");
    await settings.locator("summary").click();

    await hook.fill("./scripts/on-conflict.sh --quiet");
    await save.click();
    await expect(problem).toHaveText("The hook command must be a JSON array of non-empty strings, like [\"./scripts/on-conflict.sh\", \"--quiet\"].");
    await expect(hook).toHaveAttribute("aria-invalid", "true");
    expect(saves).toHaveLength(0);

    await hook.fill("[\"./scripts/on-conflict.sh\", \"--quiet\"]");
    await expect(problem).toBeHidden();
    await timeout.fill("20");
    const saving = page.waitForRequest(isWrite("/api/merge-assist/settings", "PUT"));
    await save.click();
    const hookCommand = { command: ["./scripts/on-conflict.sh", "--quiet"], timeoutSeconds: 20 };
    expect((await saving).postDataJSON()).toEqual({ repository: repositoryKey, checkout: checkoutPath, hook: hookCommand });
    await expect(page.locator("#toast")).toContainText("Saved Merge Assist’s settings for storefront.");
    await expect(settings.locator("summary")).toHaveText("Settings: conflict hook ./scripts/on-conflict.sh");
    const status = await rerail.get(`${origin}/api/merge-assist?${new URLSearchParams({ repository: repositoryKey, checkout: checkoutPath })}`);
    expect(await status.json()).toMatchObject({ settings: { hook: hookCommand } });

    // An empty command removes it, leaving the shared daemon as the scenario seeded it.
    await hook.fill("");
    await save.click();
    await expect(settings.locator("summary")).toHaveText("Settings: no conflict hook");
  });

  // Last in the file: the dismissal holds for every later test of this worker.
  test("Dismiss moves an open conflict to the dismissed fold, and the sidebar stops counting it", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("merge-assist");
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    await expect(sidebarCount(page)).toHaveText("1");
    await expect(dismissedFold(page).locator("summary")).toHaveText("Dismissed (1)");

    const dismissing = page.waitForRequest(isWrite("/api/merge-assist/conflicts/dismiss", "POST"));
    await page.locator(`#content [data-act="madismiss"][data-i="${OPEN_ID}"]`).click();
    expect((await dismissing).postDataJSON()).toEqual({ repository: manifest.primary.repositoryKey, checkout: manifest.primary.checkoutPath, conflictId: OPEN_ID });
    await expect(page.locator("#toast")).toContainText("Marked “cart.ts · function total” as not a conflict. It stays hidden for PR #398.");
    await expect(cards(page)).toHaveCount(1);
    await expect(cards(page).nth(0)).toHaveClass("mcard quiet");
    // Only the quiet one is open now, and the sidebar does not count it.
    await expect(sidebarCount(page)).toHaveCount(0);

    await page.reload();
    await expect(dismissedFold(page).locator("summary")).toHaveText("Dismissed (2)");
    await dismissedFold(page).locator("summary").click();
    const row = dismissedFold(page).locator("article.mclosed").filter({ hasText: "cart.ts · function total" });
    await expect(row.locator(".badge")).toHaveText("Not a conflict");
    await expect(row.locator(".mmeta")).toContainText("Dismissed by ");
    await expect(row.locator(".mmeta")).toContainText("with PR #398 · Tim");
  });
});
