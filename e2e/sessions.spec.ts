import type { Page } from "@playwright/test";
import { expect, test } from "./dashboard-test.js";

/** The list row for one session. */
const rowOf = (page: Page, id: string) => page.locator(`#content .srow[data-i="${id}"]`);
/** The selected session's header, which carries its full id as `data-session`. */
const headerId = (page: Page) => page.locator("#content .shead");

test.describe("History by session", () => {
  test("mature: the open session is selected first and reads live, with its counts on the row", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const open = manifest.sessionIds.at(-1)!;

    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));

    const row = rowOf(page, open);
    await expect(row).toHaveAttribute("aria-current", "true");
    await expect(row.locator(".state")).toHaveClass(/\blive\b/);
    await expect(page.locator('#content .shead .state[data-state="live"]')).toBeVisible();
    // The lead counts the sessions the daemon reports.
    await expect(page.locator("#content .hhead p")).toContainText(`${manifest.sessionIds.length} sessions, newest first.`);
    // Every row with violations logged says on its meta line how many of them were steered; the row shows no event
    // count or session id.
    const withViolations = page.locator("#content .srow").filter({ has: page.locator(".c:not(.zero)") });
    await expect(withViolations.first()).toBeVisible();
    await expect(withViolations.first().locator(".m .st")).toHaveText(/^\d+ steered$/);
    await expect(row.locator(".m")).not.toContainText("events");
    await expect(row.locator(".sid, .ev")).toHaveCount(0);
  });

  test("mature: a steer the agent declined reads Declined with its reason, is counted on the row and header, and has its own chip", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const open = manifest.sessionIds.at(-1)!;

    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));

    await expect(rowOf(page, open).locator(".dc")).toHaveText("1 declined");
    await expect(page.locator("#content .shead .stats")).toContainText("1 declined");
    const declined = page.locator("#content .corr").filter({ has: page.locator(".badge.declined:visible") });
    await expect(declined).toHaveCount(1);
    await expect(declined.locator(".ch .mono")).toHaveText("typescript-eslint-no-non-null-assertion");
    await expect(declined.locator(".badge.outcome")).toHaveText("Steered");
    await expect(declined.locator("dd.reason")).toHaveText(/^The cart is checked for null two lines above/);
    // Every other card, steered or logged, has no decline on record and says nothing about one.
    await expect(page.locator("#content .corr dd.reason:visible")).toHaveCount(1);

    // The title is the way to its rule: no Open rule button, and a click or Enter on the title opens the rule's sheet.
    await expect(page.locator("#content .corr").getByText("Open rule")).toHaveCount(0);
    const title = declined.locator(".ctitle");
    await title.click();
    await expect(page.locator("#panel")).toContainText("typescript-eslint-no-non-null-assertion");
    await page.keyboard.press("Escape");
    await expect(page.locator("#panel")).toBeHidden();
    await title.focus();
    await page.keyboard.press("Enter");
    await expect(page.locator("#panel")).toContainText("typescript-eslint-no-non-null-assertion");
    await page.keyboard.press("Escape");
    await expect(page.locator("#panel")).toBeHidden();

    await page.locator('#content [data-act="sfilter"][data-i="declined"]').click();
    await expect(page).toHaveURL(/#sessions\?filter=declined$/);
    await expect(page.locator("#content .srow")).toHaveCount(1);
    await expect(rowOf(page, open)).toBeVisible();
  });

  test("mature: the list's chips are All, With violations and Declined steers, and a chip opens every day it matches", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const chips = page.locator('#content [data-act="sfilter"]');
    await expect(chips).toHaveText(["All", "With violations", "Declined steers"]);
    // On one line: every chip sits at the first one's height.
    const tops = await chips.evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().top));
    expect(new Set(tops).size).toBe(1);

    // All keeps earlier days folded; every session in the scenario has violations, so the chip opens all of them.
    const rows = page.locator("#content .srow");
    await expect(rows).not.toHaveCount(manifest.sessionIds.length);
    await chips.nth(1).click();
    await expect(page).toHaveURL(/#sessions\?filter=violations$/);
    await expect(rows).toHaveCount(manifest.sessionIds.length);
    await chips.nth(0).click();
    await expect(rows).not.toHaveCount(manifest.sessionIds.length);
  });

  test("mature: the transcript names who each run is from, and every day's header stays on one line", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    // You, then the agent's first call, then Rerail's first card: a speaker is named once per run.
    await expect(page.locator("#content .tl .msgrow.u .who").first()).toHaveText("You");
    await expect(page.locator("#content .tl .evt > .spk:visible").first()).toHaveText("Agent");
    const cards = page.locator("#content .tl .tbody .corr");
    await expect(cards.first().locator("> .spk")).toHaveText("Rerail");
    await expect(cards.nth(1).locator("> .spk")).toBeHidden();
    // Each rail dot takes its card's outcome: grey for a logged card, yellow for a steered one.
    const dots = await page.locator("#content .rail .tick").evaluateAll(ticks => ticks.map(tick => ({
      dot: (tick as HTMLElement).dataset.outcome, card: document.getElementById(`c-${(tick as HTMLElement).dataset.i}`)?.dataset.outcome,
    })));
    expect(new Set(dots.map(d => d.dot))).toEqual(new Set(["logged", "steered"]));
    expect(dots.every(d => d.dot === d.card)).toBe(true);

    await page.locator('#content [data-act="sfilter"][data-i="corr"]').click();
    const lines = await page.locator("#content .daygrp").evaluateAll(groups => groups.map(group => {
      const name = group.querySelector(".name")!.getBoundingClientRect(), count = group.querySelector(".cnt") as HTMLElement;
      return { sameLine: Math.abs(name.top - count.getBoundingClientRect().top) < 2, cut: count.scrollWidth > count.clientWidth };
    }));
    expect(lines.length).toBeGreaterThan(5);
    expect(lines.every(line => line.sameLine && !line.cut)).toBe(true);
  });

  test("mature: turning Show all events off in a scrolled transcript keeps every rail dot on its card and no empty space", async ({ page, rerail }) => {
    await page.setViewportSize({ width: 1280, height: 520 });
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    await page.locator("#content .tl .corr").first().waitFor();
    const showAll = page.locator("#showAll");
    await showAll.check();
    await page.locator("#sscroll").evaluate(scroll => { scroll.scrollTop = scroll.scrollHeight; });
    await showAll.uncheck();
    // Each dot is centred on its card's first title line, and the scroll area is the timeline, not stale dots below it.
    const layout = await page.evaluate(() => {
      const tl = document.getElementById("tl")!, scroll = document.getElementById("sscroll")!, rail = document.getElementById("rail")!;
      const top = tl.getBoundingClientRect().top;
      const lines = [...tl.querySelectorAll(".corr .ctitle")].map(title => title.getBoundingClientRect().top - top + 10);
      const dots = [...rail.querySelectorAll<HTMLElement>(".tick")].map(tick => parseFloat(tick.style.top));
      return { lines, dots, overflow: scroll.scrollHeight - (tl.getBoundingClientRect().height + (tl.getBoundingClientRect().top - scroll.getBoundingClientRect().top + scroll.scrollTop)) };
    });
    expect(layout.dots).toHaveLength(layout.lines.length);
    layout.dots.forEach((dot, i) => expect(Math.abs(dot - layout.lines[i]!)).toBeLessThanOrEqual(1));
    expect(layout.overflow).toBeLessThanOrEqual(1);
  });

  test("mature: on a phone the header's button goes under a two-line title", async ({ page, rerail }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const head = page.locator("#content .shead");
    await head.locator(".stats").waitFor();
    const header = await head.evaluate(node => {
      const title = node.querySelector("h2.clamp2")!.getBoundingClientRect(), button = node.querySelector(".acts")!.getBoundingClientRect();
      const lineHeight = parseFloat(getComputedStyle(node.querySelector("h2.clamp2")!).lineHeight);
      return { buttonUnder: button.top >= title.bottom - 1, titleLines: Math.round(title.height / lineHeight) };
    });
    expect(header.buttonUnder).toBe(true);
    expect(header.titleLines).toBeLessThanOrEqual(2);
  });

  test("mature: an MCP call names its server and tool beside its MCP badge", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    await page.locator("#content .tl .corr").first().waitFor();
    await page.locator("#showAll").check();
    const call = page.locator("#content .tl .evt").filter({ has: page.locator(".vb", { hasText: "MCP" }) });
    await expect(call.locator(".vcell")).toHaveText("MCPgithub · get_pull_request");
    await expect(call.locator(".vtool")).toHaveAttribute("title", "mcp__github__get_pull_request");
    // The file change keeps its file, with no tool beside its badge.
    await expect(page.locator("#content .tl .evt").filter({ has: page.locator(".vb", { hasText: "Create" }) }).first().locator(".vtool")).toBeHidden();
  });

  test("mature: every time in the transcript ends at the same right edge, in the same type, with the sheet closed or open", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    await page.locator("#content .tl .corr").first().waitFor();
    const times = () => page.locator("#content .tl").evaluate(tl => [...tl.querySelectorAll<HTMLElement>(".msgrow .mt, .evt .tm")]
      .filter(time => time.offsetParent !== null && time.textContent !== "")
      .map(time => {
        const style = getComputedStyle(time);
        return { right: Math.round(time.getBoundingClientRect().right - parseFloat(style.paddingRight)), type: `${style.fontFamily} ${style.fontSize}` };
      }));
    for (const sheet of [false, true]) {
      if (sheet) await page.locator("#content .corr .ctitle").first().click();
      const shown = await times();
      expect(shown.length).toBeGreaterThan(1);
      expect(new Set(shown.map(time => time.right)).size).toBe(1);
      expect(new Set(shown.map(time => time.type)).size).toBe(1);
    }
  });

  test("mature: a session from Overview's recent sessions opens selected in History by session", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature"));
    const link = page.locator('#content [data-act="session"]').last();
    const id = await link.getAttribute("data-i");
    expect(id).toBeTruthy();

    await link.click();

    await expect(page).toHaveURL(new RegExp(`sessionId=${id}.*#sessions`));
    await expect(rowOf(page, id!)).toHaveAttribute("aria-current", "true");
    await expect(page.locator("#content .shead")).toHaveAttribute("data-session", id!);
  });

  test("mature: a deep link with sessionId and host selects that session, and survives a reload", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const target = manifest.sessionIds[manifest.sessionIds.length - 2]!;
    const url = new URL(await rerail.dashboard("mature", { hash: "sessions" }));
    url.searchParams.set("sessionId", target);
    url.searchParams.set("host", target.split("-").slice(2).join("-"));

    await page.goto(url.href);
    await expect(rowOf(page, target)).toHaveAttribute("aria-current", "true");
    await expect(headerId(page)).toHaveAttribute("data-session", target);

    await page.reload();
    await expect(headerId(page)).toHaveAttribute("data-session", target);
  });

  test("mature: a session whose turn end could not fully check part of the repository lists each place with why, and counts them", async ({ page, rerail }) => {
    const { manifest } = await rerail.scenario("mature");
    const url = new URL(await rerail.dashboard("mature", { hash: "sessions" }));
    const open = async (id: string): Promise<void> => {
      url.searchParams.set("sessionId", id);
      url.searchParams.set("host", id.split("-").slice(2).join("-"));
      await page.goto(url.href);
      await expect(headerId(page)).toHaveAttribute("data-session", id);
    };
    const places = page.locator("#content .tl .places");
    const end = page.locator("#content .tl > .bound").last();
    await open(manifest.sessionIds[0]!);

    await expect(places.locator("p")).toHaveText("Rules could not fully check these places:");
    await expect(places.locator("li")).toHaveText([
      "src/vendor/locked · EACCES: permission denied, opendir '/work/storefront/src/vendor/locked'",
      "data/catalog.json · Source exceeds 1000000 byte policy limit",
      "../storefront-hotfix/src/cache · EACCES: permission denied, opendir '/work/storefront-hotfix/src/cache'",
    ]);
    await expect(places.locator("li code").first()).toHaveText("src/vendor/locked");
    await expect(end).toContainText("rules could not fully check part of the repository (3 places)");

    // A session whose walks checked everything lists nothing.
    await open(manifest.sessionIds[1]!);
    await expect(end).not.toContainText("could not fully check");
    await expect(places).toBeHidden();
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
    await expect(headerId(page)).toHaveAttribute("data-session", other);

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
    await expect(page.locator("#content .shead .stats")).toContainText(`${total} violation`);
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

  test("mature: a narrow window folds the list into the picker, so the transcript keeps room to wrap between words", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("mature", { hash: "sessions" }));
    const message = page.locator("#content .msgrow.u .msg").first();
    await expect(message).toBeVisible();
    // 901 to 1180px once left the transcript a column about 20px wide beside the sidebar and the list (#301).
    for (const width of [800, 960, 1100, 1180, 1181, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(page.locator("#content .spick")).toBeVisible({ visible: width <= 1180 });
      expect((await message.boundingBox())!.width, `message width at ${width}px`).toBeGreaterThan(200);
    }
    // Long unbroken tokens (paths) still break; `anywhere` would let the box's min-content shrink to one character.
    expect(await message.evaluate(m => getComputedStyle(m).overflowWrap)).toBe("break-word");
  });

  test("no-packs: no sessions shows the empty state and no list or transcript", async ({ page, rerail }) => {
    await page.goto(await rerail.dashboard("no-packs", { hash: "sessions" }));

    await expect(page.locator("#content .hhead h2")).toBeVisible();
    await expect(page.locator("#content .empty h3")).toBeVisible();
    await expect(page.locator("#content .sess")).toBeHidden();
    await expect(page.locator('#content [data-act="slisttoggle"]')).toBeHidden();
  });
});
