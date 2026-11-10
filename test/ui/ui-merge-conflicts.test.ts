import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Conflict, MergeAssistStatus } from "../../src/engine/index.js";
import { changeScope } from "../../ui/poll.js";
import { data, ui } from "../../ui/state.js";
import { actions, enter, onInput, render } from "../../ui/views/merge.js";
import { FakeElement, FakeNode, fakeDocument, type FakeDocument } from "./support/fake-dom.js";
import { conflict, resolved, side } from "./support/merge-conflicts.js";

type Request = { method: string; url: string; body: Record<string, unknown> | undefined; answer: (status: number, body?: unknown) => void };

const HOSTILE = `<img src=x onerror="alert(1)"><b>bold</b>`;
const OPEN_ID = "conflict:native/src/model.rs#matchers:ab12";

/** On with the experimental verdicts, with an ambient overlap with PR #398, a logged one, one resolved and one dismissed. */
const fixture = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => ({
  repositoryKey: "remote:github.com/acme/storefront",
  checkout: "/work/a",
  branch: "tbelanger/export-api",
  baseBranch: "main",
  enabled: true,
  experimental: { conflictVerdicts: true },
  settings: { delivery: "both", hook: null },
  readiness: {
    judge: { available: true, runner: "claude", problem: null },
    github: { state: "signed-in", login: "tbelanger", repository: "acme/storefront" },
    candidatesIgnored: false,
    rulesActive: true,
  },
  budget: { used: 0, limit: 40, windowMinutes: 60 },
  scanning: false,
  lastScan: { at: "2026-11-02T11:58:00Z", peers: { branches: 2, pullRequests: 1 }, problems: [] },
  own: [],
  findings: [],
  conflicts: [
    conflict(OPEN_ID),
    conflict("conflict:lock", { level: "logged", title: "package-lock.json", detectedAt: "2026-11-02T11:50:00Z" }),
    resolved("conflict:retries", "theirs", { title: "Retries in API callers" }),
    conflict("conflict:dates", { status: "dismissed", title: "Dates", resolution: { action: "dismiss", name: "tbelanger", rule: null, text: null, at: "2026-11-02T11:40:00Z" } }),
  ],
  ...over,
});

/** The status the daemon answers once conflict `id` is resolved with `action`. */
function resolvedStatus(id: string, action: "mine" | "theirs" | "new"): MergeAssistStatus {
  const status = fixture();
  status.conflicts = status.conflicts.map((c): Conflict => (c.id !== id ? c : {
    ...c, status: "resolved",
    resolution: { action, name: "tbelanger", rule: ".rerail/candidates/shared-matchers.json", text: "Saved.", at: "2026-11-02T12:00:00Z" },
  }));
  return status;
}

describe("Merge Assist's conflicts", () => {
  let requests: Request[] = [];
  let root: FakeElement;
  let document: FakeDocument;
  const writes = () => requests.filter(r => r.method !== "GET");
  const button = (act: string, id?: string) =>
    root.querySelectorAll(`button[data-act="${act}"]`).find(b => id === undefined || b.dataset.i === id) ?? null;
  const toastText = () => document.getElementById("toast")!.textContent;
  const click = (el: FakeElement | null) => {
    if (el === null) throw new Error("no such button");
    if (el.disabled) return undefined;
    return actions[el.dataset.act!]!(el.dataset.i, el);
  };
  const panel = (name: string) => root.querySelectorAll("div.mpanel").find(p => p.id === `ma-panel-${name}`)!;
  const cards = () => panel("conflicts").querySelectorAll("article.mcard");
  const draft = () => root.querySelector("textarea.mdraft") as (FakeElement & { value: string }) | null;
  const type = (text: string) => {
    const area = draft()!;
    area.value = text;
    onInput(area);
    render(root);
  };

  beforeEach(() => {
    requests = [];
    document = fakeDocument();
    vi.stubGlobal("document", document);
    vi.stubGlobal("Node", FakeNode);
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", (input: URL, init: { method?: string; body?: string } = {}) => {
      const { promise, resolve } = Promise.withResolvers<Response>();
      requests.push({
        method: init.method ?? "GET",
        url: String(input),
        body: init.body === undefined ? undefined : JSON.parse(init.body) as Record<string, unknown>,
        answer: (status, body) => resolve(new Response(body === undefined ? null : JSON.stringify(body), { status })),
      });
      return promise;
    });
    data.repository = null;
    changeScope({ repository: "A", checkout: "/work/a" });
    data.repository = { setUp: true, repositoryKey: "A", name: "storefront" };
    data.mergeAssist = fixture();
    root = new FakeElement("div");
    render(root);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("opens on Conflicts with its open count, the Decisions panel hidden", () => {
    expect(root.querySelectorAll("button.mtab").map(b => [b.textContent, b.getAttribute("aria-pressed"), b.hidden]))
      .toEqual([["Conflicts · 2", "true", false], ["Decisions", "false", false]]);
    expect([panel("conflicts").hidden, panel("decisions").hidden]).toEqual([false, true]);
    click(button("matab", "decisions"));
    render(root);
    expect([panel("conflicts").hidden, panel("decisions").hidden]).toEqual([true, false]);
    expect(button("matab", "decisions")!.getAttribute("aria-pressed")).toBe("true");
  });

  it("shows an open conflict's kind, title, age, both sides with their hunks, and the evidence", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(Date.parse("2026-11-02T14:00:00Z"));
      render(root);
      const card = cards()[0]!;
      expect(card.className).toBe("mcard ambient");
      expect(card.querySelector(".mkind")!.textContent).toBe("Overlapping work");
      expect(card.querySelector("h3")!.textContent).toBe(`Shared matchers ${OPEN_ID}`);
      expect(card.querySelector(".mcage")!.textContent).toBe("Open for 3h · Told your agent and you in the session");
      const [mine, theirs] = card.querySelectorAll(".mside");
      expect(mine!.querySelector(".k")!.textContent).toBe("Mine: This branch · tbelanger");
      expect(mine!.querySelector(".msum")!.textContent).toBe("Clones each matcher per rule copy.");
      expect(mine!.querySelectorAll("pre.mhunk span").map(s => s.className)).toEqual(["meta", "del", "add"]);
      expect(mine!.querySelector(".mloc code")!.textContent).toBe("native/src/model.rs:469");
      expect(theirs!.querySelector("a")!.getAttribute("href")).toBe("https://github.com/acme/storefront/pull/398");
      expect(theirs!.querySelector(".k")!.textContent).toBe("Theirs: PR #398 · Tim");
      expect(card.querySelectorAll(".mwhy li").map(li => li.textContent)).toEqual(["Both change native/src/model.rs lines 465–472.", "No decision in force answers it."]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("says a side's tip is pushed only when it is on a remote or a pull request, else committed, and gives none for this branch", () => {
    const withTip = (pushed: boolean) => conflict(OPEN_ID, { theirs: side({ label: "jbarnes-model", ref: "jbarnes-model", tip: { at: "2026-11-02T10:00:00Z", pushed } }) });
    const tipText = (pushed: boolean): string[] => {
      data.mergeAssist = fixture({ conflicts: [withTip(pushed)] });
      render(root);
      return cards()[0]!.querySelectorAll(".mside").map(entry => entry.querySelector(".mloc span")?.textContent ?? "");
    };
    const [mine, pushed] = tipText(true);
    expect(mine).toBe("");
    expect(pushed).toMatch(/^pushed /u);
    expect(tipText(false)[1]).toMatch(/^committed /u);
  });

  it("offers the four choices alike, none styled as recommended, and shows a logged conflict muted", () => {
    const acts = cards()[0]!.querySelectorAll(".macts button").map(b => [b.dataset.act, b.textContent, b.className]);
    expect(acts).toEqual([
      ["mamine", "Keep mine", "btn sm"], ["matheirs", "Take theirs", "btn sm"], ["manew", "Write a new decision…", "btn sm"],
      ["manotconflict", "Not a conflict", "btn sm ghost"],
    ]);
    const logged = cards()[1]!;
    expect(logged.className).toBe("mcard logged");
    expect(logged.querySelector(".mlogged")!.textContent).toBe("Logged, no interruption");
    expect(logged.querySelector(".mcage")!.textContent).not.toContain("Told your agent");
  });

  it("renders other people's text as text and links only a pull request URL it can trust", () => {
    data.mergeAssist = fixture({
      conflicts: [conflict(OPEN_ID, {
        title: HOSTILE, evidence: [HOSTILE],
        theirs: side({ label: HOSTILE, author: HOSTILE, summary: HOSTILE, hunk: `+${HOSTILE}`, url: "javascript:alert(1)" }),
      })],
    });
    render(root);
    const card = cards()[0]!;
    expect(card.querySelectorAll("img")).toHaveLength(0);
    expect(card.querySelectorAll("b")).toHaveLength(0);
    expect(card.querySelectorAll("a")).toHaveLength(0);
    expect(card.querySelector("h3")!.textContent).toBe(HOSTILE);
    expect(card.querySelectorAll(".msum")[1]!.textContent).toBe(HOSTILE);
  });

  it("Keep mine opens the decision prefilled with this side's summary; Confirm sends it and the card moves to Resolved", async () => {
    click(button("mamine", OPEN_ID));
    render(root);
    expect(draft()!.value).toBe("Clones each matcher per rule copy.");
    expect(root.querySelector(".medit label")!.textContent).toBe("Keep mine: the decision to save");
    expect(button("mamine", OPEN_ID)!.getAttribute("aria-expanded")).toBe("true");
    expect(button("maconfirm", OPEN_ID)!.disabled).toBe(false);

    const confirming = click(button("maconfirm", OPEN_ID));
    render(root);
    expect(cards()[0]!.getAttribute("aria-busy")).toBe("true");
    expect(button("maconfirm", OPEN_ID)!.disabled).toBe(true);
    expect(button("manotconflict", OPEN_ID)!.disabled).toBe(true);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]).toMatchObject({
      method: "PUT", url: expect.stringMatching(/\/api\/merge-assist\/conflicts$/u),
    });
    expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", conflictId: OPEN_ID, action: "mine", text: "Clones each matcher per rule copy." });

    writes()[0]!.answer(200, resolvedStatus(OPEN_ID, "mine"));
    await confirming;
    render(root);
    expect(toastText()).toBe("Kept mine. Saved “Clones each matcher per rule copy.” as a decision on this branch. It steers your coding agent from its next turn. Commit it to share it with teammates.");
    expect(draft()).toBeNull();
    expect(cards().map(c => c.className)).toEqual(["mcard logged"]);
    expect(button("matab", "conflicts")!.textContent).toBe("Conflicts · 1");
    expect(panel("conflicts").querySelector("details.mfold summary")!.textContent).toBe("Resolved and dismissed (3)");
    expect(ui.mergeDraft).toBeNull();
  });

  it("Take theirs sends the text as the person edited it", async () => {
    click(button("matheirs", OPEN_ID));
    render(root);
    expect(draft()!.value).toBe("Shares compiled matchers; no rule copy deep-copies them.");
    type("  Compiled matchers are shared; copies never clone them.  ");
    const confirming = click(button("maconfirm", OPEN_ID));
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toMatchObject({ conflictId: OPEN_ID, action: "theirs", text: "Compiled matchers are shared; copies never clone them." });
    writes()[0]!.answer(200, resolvedStatus(OPEN_ID, "theirs"));
    await confirming;
    expect(toastText()).toMatch(/^Took theirs\. Saved “Compiled matchers are shared; copies never clone them\.”/u);
  });

  it("a new decision starts empty and can be confirmed only with 1 to 600 characters", async () => {
    click(button("manew", OPEN_ID));
    render(root);
    expect(draft()!.value).toBe("");
    expect(root.querySelector(".medit label")!.textContent).toBe("Your new decision");
    expect(button("maconfirm", OPEN_ID)!.disabled).toBe(true);
    expect(root.querySelector(".mhint")!.textContent).toBe("1 to 600 characters.");

    type("x".repeat(601));
    expect(button("maconfirm", OPEN_ID)!.disabled).toBe(true);
    expect(root.querySelector(".mhint")!.textContent).toBe("1 character over the 600-character limit.");
    expect(draft()!.getAttribute("aria-invalid")).toBe("true");

    const area = draft();
    type("Keep both, behind a flag.");
    expect(draft()).toBe(area); // typing patches the draft in place; it is never rebuilt
    expect(root.querySelector(".mhint")!.textContent).toBe("25 of 600 characters.");
    expect(button("maconfirm", OPEN_ID)!.disabled).toBe(false);
    const confirming = click(button("maconfirm", OPEN_ID));
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", conflictId: OPEN_ID, action: "new", text: "Keep both, behind a flag." });
    writes()[0]!.answer(200, resolvedStatus(OPEN_ID, "new"));
    await confirming;
    expect(toastText()).toMatch(/^Saved “Keep both, behind a flag\.” as a decision on this branch\./u);
  });

  it("Cancel closes the draft and sends nothing; a failed save keeps the draft to try again", async () => {
    click(button("matheirs", OPEN_ID));
    render(root);
    click(button("macancel", OPEN_ID));
    render(root);
    expect(draft()).toBeNull();
    expect(button("matheirs", OPEN_ID)!.getAttribute("aria-expanded")).toBe("false");
    expect(writes()).toHaveLength(0);

    click(button("matheirs", OPEN_ID));
    render(root);
    const confirming = click(button("maconfirm", OPEN_ID));
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(409, { error: "invalid-state", message: "Only a person can resolve a conflict; the person decides." });
    await confirming;
    render(root);
    expect(toastText()).toContain(`Couldn’t save the decision for “Shared matchers ${OPEN_ID}”. Only a person can resolve a conflict; the person decides.`);
    expect(draft()!.value).toBe("Shares compiled matchers; no rule copy deep-copies them.");
    expect(button("maconfirm", OPEN_ID)!.disabled).toBe(false);
  });

  it("Not a conflict hides the card at once, sends no text, and brings it back when the write fails", async () => {
    const dismissing = click(button("manotconflict", OPEN_ID));
    render(root);
    expect(cards().map(c => c.className)).toEqual(["mcard logged"]);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", conflictId: OPEN_ID, action: "dismiss" });
    writes()[0]!.answer(500, { error: "internal", message: "Disk full." });
    await dismissing;
    render(root);
    expect(cards()).toHaveLength(2);
    expect(toastText()).toContain(`Couldn’t mark “Shared matchers ${OPEN_ID}” as not a conflict. Disk full.`);
  });

  it("shows an open conflict's summary and a failed conflict hook with its event and why", () => {
    data.mergeAssist = fixture({
      conflicts: [conflict(OPEN_ID, {
        summary: { text: `PR #398 (Tim) is changing matchers ${HOSTILE}`, via: "fallback", at: "2026-11-02T11:00:00Z", reason: "budget" },
        hook: { event: "level-changed", ok: false, error: "timed out after 10 s", note: null, status: null, at: "2026-11-02T11:01:00Z" },
      })],
    });
    render(root);
    const card = cards()[0]!;
    expect(card.querySelector(".msummary")!.textContent).toBe(`PR #398 (Tim) is changing matchers ${HOSTILE}`);
    expect(card.querySelectorAll("img")).toHaveLength(0);
    expect(card.querySelector(".mhookfail")!.textContent).toBe("Conflict hook failed (level-changed): timed out after 10 s");

    data.mergeAssist = fixture({ conflicts: [conflict(OPEN_ID, { hook: { event: "detected", ok: true, error: null, note: "n", status: "s", at: "2026-11-02T11:01:00Z" } })] });
    render(root);
    expect(cards()[0]!.querySelector(".msummary")).toBeNull();
    expect(cards()[0]!.querySelector(".mhookfail")).toBeNull();
  });

  describe("without the experimental verdicts", () => {
    const off = (over: Partial<MergeAssistStatus> = {}) =>
      fixture({ experimental: { conflictVerdicts: false }, ...over });

    beforeEach(() => {
      data.mergeAssist = off();
      render(root);
    });

    it("offers Dismiss alone, and says who to talk to with a link to their pull request", () => {
      const card = cards()[0]!;
      expect(card.querySelectorAll(".macts button").map(b => [b.dataset.act, b.textContent])).toEqual([["manotconflict", "Dismiss"]]);
      expect(card.querySelector(".medit")).toBeNull();
      const talk = card.querySelector(".mtalk")!;
      expect(talk.textContent).toBe("Talk it over with Tim (PR #398). Rerail never picks a side; dismiss it if it isn’t a conflict.");
      expect(talk.querySelector("a")!.getAttribute("href")).toBe("https://github.com/acme/storefront/pull/398");
      expect(panel("conflicts").querySelector(".ghead p")!.textContent).toContain("talk it over with the other side’s author");
    });

    it("names a branch without a link, and links no URL it can't trust", () => {
      data.mergeAssist = off({
        conflicts: [conflict(OPEN_ID, { theirs: side({ label: "jbarnes/model", author: HOSTILE, url: "javascript:alert(1)" }) })],
      });
      render(root);
      const talk = cards()[0]!.querySelector(".mtalk")!;
      expect(talk.querySelectorAll("a")).toHaveLength(0);
      expect(talk.querySelectorAll("img")).toHaveLength(0);
      expect(talk.textContent).toBe(`Talk it over with ${HOSTILE} (jbarnes/model). Rerail never picks a side; dismiss it if it isn’t a conflict.`);
    });

    it("Dismiss marks it as not a conflict, sending no text", async () => {
      const dismissing = click(button("manotconflict", OPEN_ID));
      render(root);
      expect(cards().map(c => c.className)).toEqual(["mcard logged"]);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", conflictId: OPEN_ID, action: "dismiss" });
      writes()[0]!.answer(200, off({ conflicts: fixture().conflicts.filter(c => c.id !== OPEN_ID) }));
      await dismissing;
      expect(toastText()).toContain(`Marked “Shared matchers ${OPEN_ID}” as not a conflict.`);
    });

    it("drops the verdict choices from a card already shown once the verdicts are turned off", () => {
      data.mergeAssist = fixture();
      render(root);
      expect(cards()[0]!.querySelectorAll(".macts button")).toHaveLength(4);
      data.mergeAssist = off();
      render(root);
      expect(cards()[0]!.querySelectorAll(".macts button").map(b => b.textContent)).toEqual(["Dismiss"]);
    });
  });

  it("folds resolved and dismissed conflicts with who decided, how, and the rule path", () => {
    const fold = panel("conflicts").querySelector("details.mfold")!;
    expect(fold.hasAttribute("open")).toBe(false);
    click(fold.querySelector("summary"));
    render(root);
    expect(fold.hasAttribute("open")).toBe(true);
    const [taken, dismissed] = fold.querySelectorAll("article.mclosed");
    expect(taken!.querySelector(".badge")!.textContent).toBe("Resolved");
    expect(taken!.querySelector(".mmeta")!.textContent).toContain("Took theirs (PR #398)by tbelanger");
    expect(taken!.querySelector("code.mrule")!.textContent).toBe(".rerail/candidates/conflict:retries.json");
    expect(taken!.querySelector(".mq")!.textContent).toBe("Decision for conflict:retries.");
    expect(dismissed!.querySelector(".badge")!.textContent).toBe("Not a conflict");
    expect(dismissed!.querySelector(".mmeta")!.textContent).toContain("by tbelanger");
    expect(dismissed!.querySelector("code.mrule")).toBeNull();
  });

  it("says when nothing is open, and opens on Decisions then", () => {
    data.mergeAssist = fixture({ conflicts: [] });
    render(root);
    expect(button("matab", "decisions")!.getAttribute("aria-pressed")).toBe("true");
    expect(button("matab", "conflicts")!.textContent).toBe("Conflicts");
    click(button("matab", "conflicts"));
    render(root);
    expect(panel("conflicts").textContent).toContain("No open conflicts");
    expect(panel("conflicts").textContent).toContain("Rerail compared this branch with 2 local branches and 1 open pull request");
    expect(panel("conflicts").querySelector("details.mfold")!.hidden).toBe(true);
  });

  it("a link from the session's line opens Conflicts on that conflict, even from another tab, and a closed one's fold", () => {
    click(button("matab", "decisions"));
    render(root);
    const card = cards()[1]!;
    const focus = vi.spyOn(card, "focus");
    enter({ view: "merge", conflict: "conflict:lock" });
    render(root);
    expect(panel("conflicts").hidden).toBe(false);
    expect(card.getAttribute("tabindex")).toBe("-1");
    expect(focus).toHaveBeenCalledTimes(1);
    render(root);
    expect(focus).toHaveBeenCalledTimes(1); // once, not on every render

    enter({ view: "merge", conflict: "conflict:retries" });
    render(root);
    expect(panel("conflicts").querySelector("details.mfold")!.hasAttribute("open")).toBe(true);
  });

  it("shows no tabs while off or while the status cannot be read", () => {
    data.mergeAssistError = "/api/merge-assist?repository=A answered 500";
    render(root);
    expect(root.querySelector("div.mtabs")!.hidden).toBe(true);
    expect(root.querySelectorAll("div.mpanel").every(p => p.hidden)).toBe(true);
    data.mergeAssistError = null;
    data.mergeAssist = fixture({ enabled: false });
    render(root);
    expect(root.querySelector("div.mtabs")!.hidden).toBe(true);
  });
});
