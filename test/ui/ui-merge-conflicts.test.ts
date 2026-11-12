import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MergeAssistStatus } from "../../src/engine/index.js";
import { data, ui } from "../../ui/state.js";
import { enter, render } from "../../ui/views/merge.js";
import { conflict, dismissed, side } from "./support/merge-conflicts.js";
import { mergePage, storefrontStatus } from "./support/merge-page.js";

const HOSTILE = `<img src=x onerror="alert(1)"><b>bold</b>`;
const OPEN_ID = "conflict:native/src/model.rs#matchers:ab12";

/** On, with an ambient overlap with PR #398, a logged one, and one dismissed. */
const fixture = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => storefrontStatus({
  conflicts: [
    conflict(OPEN_ID),
    conflict("conflict:lock", { level: "logged", title: "package-lock.json", detectedAt: "2026-11-02T11:50:00Z" }),
    dismissed("conflict:dates", "2026-11-02T11:40:00Z", { title: "Dates" }),
  ],
  ...over,
});

describe("Merge Assist's conflicts", () => {
  const page = mergePage();
  const { button, click, toastText, writes } = page;
  const section = () => page.root.querySelector('section[data-section="conflicts"]')!;
  const cards = () => section().querySelectorAll("article.mcard");
  const fold = () => section().querySelector("details.mfold")!;

  beforeEach(() => {
    data.mergeAssist = fixture();
    render(page.root);
  });

  it("shows an open conflict's level, title, short id, age, both sides with their hunks, its size and the evidence", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(Date.parse("2026-11-02T14:00:00Z"));
      render(page.root);
      const card = cards()[0]!;
      expect(card.className).toBe("mcard ambient");
      expect(card.querySelector(".mlvl")!.textContent).toBe("Ambient");
      expect(card.querySelector("h3")!.textContent).toBe(`Shared matchers ${OPEN_ID}`);
      expect(card.querySelector(".mshort")!.textContent).toBe("#a1b2c3");
      expect(card.querySelector(".mcage")!.textContent).toBe("Open for 3h · Told your agent and you in the session");
      const [mine, theirs] = card.querySelectorAll(".mside");
      expect(mine!.querySelector(".k")!.textContent).toBe("Mine: This branch · tbelanger");
      expect(mine!.querySelector(".msum")!.textContent).toBe("Clones each matcher per rule copy.");
      expect(mine!.querySelectorAll("pre.mhunk span").map(s => s.className)).toEqual(["meta", "del", "add"]);
      expect(mine!.querySelector(".mloc code")!.textContent).toBe("native/src/model.rs:469");
      expect(theirs!.querySelector("a")!.getAttribute("href")).toBe("https://github.com/acme/storefront/pull/398");
      expect(theirs!.querySelector(".k")!.textContent).toBe("Theirs: PR #398 · Tim");
      expect(card.querySelectorAll(".mreasons li").map(li => li.textContent))
        .toEqual(["local code: ×1", "+40 lines on theirs in 2 h", "3 callers", "even odds either side loses"]);
      expect(card.querySelectorAll(".mwhy li").map(li => li.textContent)).toEqual(["Both change native/src/model.rs lines 465–472."]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("moves a card's age on without rebuilding it", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(Date.parse("2026-11-02T11:04:00Z"));
      render(page.root);
      const card = cards()[0]!;
      expect(card.querySelector(".mago")!.textContent).toBe("4m");
      vi.setSystemTime(Date.parse("2026-11-02T14:00:00Z"));
      render(page.root);
      expect(cards()[0]).toBe(card);
      expect(card.querySelector(".mago")!.textContent).toBe("3h");
    } finally {
      vi.useRealTimers();
    }
  });

  it("lists the loudest first and shows a logged conflict muted", () => {
    data.mergeAssist = fixture({
      conflicts: [...fixture().conflicts, conflict("conflict:stop", { level: "stop", title: "Schema", detectedAt: "2026-11-01T09:00:00Z" })],
    });
    render(page.root);
    expect(cards().map(c => c.className)).toEqual(["mcard stop", "mcard ambient", "mcard logged"]);
    const logged = cards()[2]!;
    expect(logged.querySelector(".mlogged")!.textContent).toBe("Logged, no interruption");
    expect(logged.querySelector(".mcage")!.textContent).not.toContain("Told your agent");
  });

  it("marks a queued conflict and says why it is quieter", () => {
    data.mergeAssist = fixture({ conflicts: [conflict(OPEN_ID, { queued: true, queuedFor: "checkpoint" })] });
    render(page.root);
    const card = cards()[0]!;
    expect(card.querySelector(".mqueued")!.textContent).toBe("Queued");
    expect(card.querySelector(".mstate")!.textContent).toBe("Queued: over the interruption budget, so shown as ambient until it frees up");
  });

  it("says a side's tip is pushed only when it is on a remote or a pull request, else committed, and gives none for this branch", () => {
    const withTip = (pushed: boolean) => conflict(OPEN_ID, { theirs: side({ label: "jbarnes-model", ref: "jbarnes-model", tip: { at: "2026-11-02T10:00:00Z", pushed } }) });
    const tipText = (pushed: boolean): string[] => {
      data.mergeAssist = fixture({ conflicts: [withTip(pushed)] });
      render(page.root);
      return cards()[0]!.querySelectorAll(".mside").map(entry => entry.querySelector(".mloc span")?.textContent ?? "");
    };
    const [mine, pushed] = tipText(true);
    expect(mine).toBe("");
    expect(pushed).toMatch(/^pushed /u);
    expect(tipText(false)[1]).toMatch(/^committed /u);
  });

  it("offers Dismiss alone, and says who to talk it over with, linking their pull request", () => {
    const card = cards()[0]!;
    expect(card.querySelectorAll(".macts button").map(b => [b.dataset.act, b.textContent])).toEqual([["madismiss", "Dismiss"]]);
    const talk = card.querySelector(".mtalk")!;
    expect(talk.textContent).toBe("Talk it over with Tim (PR #398). Rerail never picks a side; dismiss it if it isn’t a conflict.");
    expect(talk.querySelector("a")!.getAttribute("href")).toBe("https://github.com/acme/storefront/pull/398");
    expect(section().querySelector(".ghead p")!.textContent).toContain("talk it over with the other side’s author");
  });

  it("renders other people's text as text and links only a pull request URL it can trust", () => {
    data.mergeAssist = fixture({
      conflicts: [conflict(OPEN_ID, {
        title: HOSTILE, evidence: [HOSTILE],
        summary: { text: HOSTILE, via: "judge", at: "2026-11-02T11:00:00Z", reason: null },
        theirs: side({ label: "jbarnes/model", author: HOSTILE, summary: HOSTILE, hunk: `+${HOSTILE}`, url: "javascript:alert(1)" }),
      })],
    });
    render(page.root);
    const card = cards()[0]!;
    expect(card.querySelectorAll("img")).toHaveLength(0);
    expect(card.querySelectorAll("a")).toHaveLength(0);
    expect(card.querySelector("h3")!.textContent).toBe(HOSTILE);
    expect(card.querySelector(".msummary")!.textContent).toBe(HOSTILE);
    expect(card.querySelectorAll(".msum")[1]!.textContent).toBe(HOSTILE);
    expect(card.querySelector(".mtalk")!.textContent).toBe(`Talk it over with ${HOSTILE} (jbarnes/model). Rerail never picks a side; dismiss it if it isn’t a conflict.`);
    // The one bold element is the author's name, holding the hostile text as text.
    expect(card.querySelectorAll("b").map(b => [b.className, b.textContent])).toEqual([["mauthor", HOSTILE]]);
  });

  it("shows an open conflict's summary and a failed conflict hook with its event and why", () => {
    data.mergeAssist = fixture({
      conflicts: [conflict(OPEN_ID, {
        summary: { text: "PR #398 (Tim) is changing matchers", via: "fallback", at: "2026-11-02T11:00:00Z", reason: "budget" },
        hook: { event: "level-changed", ok: false, error: "timed out after 10 s", note: null, status: null, at: "2026-11-02T11:01:00Z" },
      })],
    });
    render(page.root);
    const card = cards()[0]!;
    expect(card.querySelector(".msummary")!.textContent).toBe("PR #398 (Tim) is changing matchers");
    expect(card.querySelector(".mhookfail")!.textContent).toBe("Conflict hook failed (level-changed): timed out after 10 s");

    data.mergeAssist = fixture({ conflicts: [conflict(OPEN_ID, { hook: { event: "detected", ok: true, error: null, note: "n", status: "s", at: "2026-11-02T11:01:00Z" } })] });
    render(page.root);
    expect(cards()[0]!.querySelector(".msummary")).toBeNull();
    expect(cards()[0]!.querySelector(".mhookfail")).toBeNull();
  });

  it("Dismiss posts the conflict for the checkout, hides the card at once, and toasts once it is dismissed", async () => {
    const dismissing = click(button("madismiss", OPEN_ID));
    render(page.root);
    expect(cards().map(c => c.className)).toEqual(["mcard logged"]);
    expect(section().querySelector("h2")!.textContent).toBe("Open conflicts1");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]).toMatchObject({ method: "POST", url: expect.stringMatching(/\/api\/merge-assist\/conflicts\/dismiss$/u) });
    expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", conflictId: OPEN_ID });

    const after = fixture();
    after.conflicts = [after.conflicts[1]!, dismissed(OPEN_ID, "2026-11-02T12:00:00Z"), after.conflicts[2]!];
    writes()[0]!.answer(200, after);
    await dismissing;
    render(page.root);
    expect(toastText()).toBe(`Marked “Shared matchers ${OPEN_ID}” as not a conflict. It stays hidden for PR #398.`);
    expect(cards().map(c => c.className)).toEqual(["mcard logged"]);
    expect(fold().querySelector("summary")!.textContent).toBe("Dismissed (2)");
  });

  it("brings a dismissed card back and says why when the write fails", async () => {
    const dismissing = click(button("madismiss", OPEN_ID));
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(500, { error: "internal", message: "Disk full." });
    await dismissing;
    render(page.root);
    expect(cards()).toHaveLength(2);
    expect(toastText()).toContain(`Couldn’t mark “Shared matchers ${OPEN_ID}” as not a conflict. Disk full.`);
  });

  it("folds dismissed conflicts with who dismissed each and when, and whose work it was with", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(Date.parse("2026-11-02T12:00:00Z"));
      render(page.root);
      expect(fold().hasAttribute("open")).toBe(false);
      expect(fold().querySelector("summary")!.textContent).toBe("Dismissed (1)");
      click(fold().querySelector("summary"));
      render(page.root);
      expect(fold().hasAttribute("open")).toBe(true);
      const row = fold().querySelector("article.mclosed")!;
      expect(row.querySelector(".badge")!.textContent).toBe("Not a conflict");
      expect(row.querySelector("h3")!.textContent).toBe("Dates");
      expect(row.querySelector(".mmeta")!.textContent).toBe("Dismissed by tbelanger20m agowith PR #398 · Tim");
      expect(row.querySelectorAll("button")).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("says when nothing is open, what it compared, and where the earlier ones are", () => {
    data.mergeAssist = fixture({ conflicts: [dismissed("conflict:dates", "2026-11-02T11:40:00Z")] });
    render(page.root);
    expect(cards()).toHaveLength(0);
    expect(section().textContent).toContain("No open conflicts");
    expect(section().textContent).toContain("Rerail compared this branch with 2 local branches and 1 open pull request");
    expect(section().textContent).toContain("Earlier ones are under Dismissed.");
    data.mergeAssist = fixture({ conflicts: [] });
    render(page.root);
    expect(section().textContent).not.toContain("Earlier ones");
    expect(fold().hidden).toBe(true);
  });

  it("a link from the session's line brings that conflict into view once, and opens the fold for a dismissed one", () => {
    const card = cards()[1]!;
    const focus = vi.spyOn(card, "focus");
    enter({ view: "merge", conflict: "conflict:lock" });
    render(page.root);
    expect(card.getAttribute("tabindex")).toBe("-1");
    expect(focus).toHaveBeenCalledTimes(1);
    render(page.root);
    expect(focus).toHaveBeenCalledTimes(1); // once, not on every render

    enter({ view: "merge", conflict: "conflict:dates" });
    render(page.root);
    expect(fold().hasAttribute("open")).toBe(true);
    expect(ui.mergeOpen.has("dismissed")).toBe(true);
  });
});
