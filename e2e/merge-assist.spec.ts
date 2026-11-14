import type { Page, Request } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";
import { HOSTILE_TEXT, OPEN_CONFLICT_ID, OPEN_PR_URL } from "./support/scenario-manifest.js";

const conflicts = (page: Page) => page.locator('#content section[data-section="conflicts"]');
const cards = (page: Page) => conflicts(page).locator("article.mcard");
const dismissedFold = (page: Page) => conflicts(page).locator("details.mfold");
/** The sidebar's count on Merge Assist: the open conflicts that are not quiet. */
const sidebarCount = (page: Page) => page.locator('#sidenav [data-act="nav"][data-i="merge"] .n');
const isWrite = (path: string, method: string) => (request: Request) => request.url().endsWith(path) && request.method() === method;

test.describe("Merge Assist", () => {
  test("with the experiment off, the page says so and the sidebar has no Merge Assist", async ({ page, rerail }) => {
    // Only the merge-assist scenario's daemon runs with RERAIL_EXPERIMENTAL_MERGE_ASSIST=1.
    const offRead = page.waitForResponse(response => new URL(response.url()).pathname === "/api/merge-assist");
    await page.goto(await rerail.dashboard("mature", { hash: "merge" }));
    const answer = await offRead;
    expect([answer.status(), (await answer.json() as { error: string }).error]).toEqual([404, "experimental-off"]);
    await expect(page.locator("#content")).toContainText("RERAIL_EXPERIMENTAL_MERGE_ASSIST=1");
    await expect(page.locator('#content [data-act="maon"]')).toHaveCount(0);
    await expect(page.locator("#sidenav")).toContainText("By session");
    await expect(page.locator('#sidenav [data-i="merge"]')).toHaveCount(0);
  });

  test("a person turns it on from the page, and off again", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("merge-assist");
    const billing = manifest.repositories.find(repository => repository.repositoryKey !== manifest.primary.repositoryKey)!;
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge", repository: billing.repositoryKey }));
    const consent = page.locator("#content .mconsent");
    await expect(consent).toBeVisible();
    // While off, Rerail has run no gh: it says it checks gh's sign-in once turned on.
    await expect(consent.locator('[data-check="github"]')).toContainText("turn Merge Assist on");
    await expect(conflicts(page)).toBeHidden();

    const turnOn = page.waitForRequest(isWrite("/api/merge-assist/settings", "PUT"));
    await page.locator('#content [data-act="maon"]').click();
    expect((await turnOn).postDataJSON()).toEqual({ repository: billing.repositoryKey, checkout: billing.checkoutPath, enabled: true });
    await expect(page.locator("#toast")).toContainText("on in billing");
    await expect(consent).toHaveCount(0);
    // Once on, Rerail reads gh's sign-in: the scenario's gh is signed out.
    await expect(page.locator("#content .mstrip")).toContainText("gh signed out");
    await expect(conflicts(page)).toContainText("No open conflicts");
    await page.reload();
    await expect(page.locator("#content .mstrip")).toBeVisible();

    // Leave the shared daemon as the scenario seeded it.
    await page.locator('#content [data-act="maoff"]').click();
    await expect(page.locator("#toast")).toContainText("off in billing");
    await expect(consent).toBeVisible();
  });

  test("an open conflict shows both sides, why, and who to talk it over with; a quiet one is muted and not counted", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    await expect(cards(page)).toHaveCount(2);
    await expect(conflicts(page).locator("h2 .n2")).toHaveText("2");

    const open = cards(page).nth(0);
    await expect(open).toHaveClass("mcard");
    await expect(open.locator("h3")).toHaveText("cart.ts · function total");
    await expect(open.locator(".mcage")).toHaveText(/^Open for 3h · Told/u);
    const [mine, theirs] = [open.locator(".mside").nth(0), open.locator(".mside").nth(1)];
    await expect(mine.locator(".k")).toHaveText("Mine: This branch · tbelanger");
    await expect(mine.locator("pre.mhunk span")).toHaveClass(["meta", "ctx", "del", "add"]);
    await expect(theirs.locator(".k")).toHaveText("Theirs: PR #398 · Tim");
    await expect(theirs.locator(".k a")).toHaveAttribute("href", OPEN_PR_URL);
    await expect(open.locator(".mwhy li")).toHaveText(["Both change src/cart/cart.ts lines 1–3."]);
    await expect(open.locator(".mtalk b.mauthor")).toHaveText("Tim");
    await expect(open.locator(".mtalk a")).toHaveAttribute("href", OPEN_PR_URL);

    const quiet = cards(page).nth(1);
    await expect(quiet).toHaveClass("mcard quiet");
    await expect(quiet.locator("h3")).toHaveText("package-lock.json");
    await expect(quiet.locator(".mquiet")).toBeVisible();
    await expect(quiet.locator(".mcage")).toHaveText(/^Open for 1h$/u);
    // Two open, one counted: the agent is never told of the quiet one.
    await expect(sidebarCount(page)).toHaveText("1");
  });

  test("other people's text shows as text, never as markup, and their links only open this repository's pull requests", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    const theirs = cards(page).nth(0).locator(".mside").nth(1);
    await expect(theirs.locator(".msum")).toHaveText(HOSTILE_TEXT);
    await expect(theirs.locator("pre.mhunk")).toContainText(HOSTILE_TEXT);

    // The quiet card's branch names another repository's pull request: its label shows, as text.
    const quiet = cards(page).nth(1);
    await expect(quiet.locator(".mtalk .mono")).toHaveText("jbarnes/bump-react");
    await expect(quiet.locator("a")).toHaveCount(0);

    await dismissedFold(page).locator("summary").click();
    const dismissed = dismissedFold(page).locator("article.mclosed");
    await expect(dismissed.locator("h3")).toHaveText(HOSTILE_TEXT);
    await expect(dismissed.locator(".mmeta")).toContainText(`with ${HOSTILE_TEXT} · ${HOSTILE_TEXT}`);
    await expect(conflicts(page).locator("img")).toHaveCount(0);
    await expect(conflicts(page).locator("b:not(.mauthor)")).toHaveCount(0);
    // The one link the page makes is this repository's pull request; a script or another repository never.
    await expect(conflicts(page).locator("a[href]")).toHaveCount(2);
    for (const href of await conflicts(page).locator("a[href]").evaluateAll(links => links.map(link => link.getAttribute("href")))) {
      expect(href).toBe(OPEN_PR_URL);
    }
    expect(await page.evaluate(() => "__rerailInjected" in window)).toBe(false);
  });

  test("saves the conflict hook command, and refuses a command or timeout it cannot run without sending it", async ({ page, rerail }) => {
    const { manifest, origin } = await rerail.scenario("merge-assist");
    const { repositoryKey, checkoutPath } = manifest.primary;
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    const settings = page.locator("#content details.msettings");
    const hook = page.locator("#ma-hook"), timeout = page.locator("#ma-timeout"), problem = page.locator("#ma-settings-problem");
    const save = settings.locator('[data-act="masettings"]');
    const saves: Request[] = [];
    page.on("request", request => { if (isWrite("/api/merge-assist/settings", "PUT")(request)) saves.push(request); });
    await expect(settings.locator("summary")).toContainText("no conflict hook");
    await settings.locator("summary").click();

    await hook.fill("./scripts/on-conflict.sh --quiet");
    await save.click();
    await expect(problem).toContainText("JSON array");
    await expect(hook).toHaveAttribute("aria-invalid", "true");

    await hook.fill("[\"./scripts/on-conflict.sh\", \"--quiet\"]");
    await expect(problem).toBeHidden();
    // Text that is not a number is refused as it reads, never saved as the default.
    await timeout.fill("3e");
    await save.click();
    await expect(problem).toContainText("timeout");
    await expect(timeout).toHaveAttribute("aria-invalid", "true");
    await expect(timeout).toHaveValue("3e");
    expect(saves).toHaveLength(0);

    await timeout.fill("20");
    const saving = page.waitForRequest(isWrite("/api/merge-assist/settings", "PUT"));
    await save.click();
    const hookCommand = { command: ["./scripts/on-conflict.sh", "--quiet"], timeoutSeconds: 20 };
    expect((await saving).postDataJSON()).toEqual({ repository: repositoryKey, checkout: checkoutPath, hook: hookCommand });
    await expect(page.locator("#toast")).toContainText("storefront");
    await expect(settings.locator("summary")).toContainText("./scripts/on-conflict.sh");
    const status = await rerail.get(`${origin}/api/merge-assist?${new URLSearchParams({ repository: repositoryKey, checkout: checkoutPath })}`);
    expect(await status.json()).toMatchObject({ settings: { hook: hookCommand } });

    // An empty command removes it, leaving the shared daemon as the scenario seeded it.
    await hook.fill("");
    await save.click();
    await expect(settings.locator("summary")).toContainText("no conflict hook");
  });

  test("Dismiss moves an open conflict to the dismissed fold, and the sidebar stops counting it", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("merge-assist");
    await page.goto(await rerail.dashboard("merge-assist", { hash: "merge" }));
    await expect(sidebarCount(page)).toHaveText("1");
    await expect(dismissedFold(page).locator("summary")).toHaveText("Dismissed (1)");

    const dismissing = page.waitForRequest(isWrite("/api/merge-assist/conflicts/dismiss", "POST"));
    await page.locator(`#content [data-act="madismiss"][data-i="${OPEN_CONFLICT_ID}"]`).click();
    expect((await dismissing).postDataJSON()).toEqual({ repository: manifest.primary.repositoryKey, checkout: manifest.primary.checkoutPath, conflictId: OPEN_CONFLICT_ID });
    await expect(page.locator("#toast")).toContainText("cart.ts · function total");
    await expect(cards(page)).toHaveCount(1);
    await expect(cards(page).nth(0)).toHaveClass("mcard quiet");
    // Only the quiet one is open now, and the sidebar does not count it.
    await expect(sidebarCount(page)).toHaveCount(0);

    await page.reload();
    await expect(dismissedFold(page).locator("summary")).toHaveText("Dismissed (2)");
    await dismissedFold(page).locator("summary").click();
    const row = dismissedFold(page).locator("article.mclosed").filter({ hasText: "cart.ts · function total" });
    await expect(row.locator(".mmeta b.mauthor")).toBeVisible();
    await expect(row.locator(".mmeta")).toContainText("PR #398 · Tim");
  });
});
