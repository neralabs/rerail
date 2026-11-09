import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Finding, MergeAssistStatus, OwnDecision } from "../../src/engine/index.js";
import { changeScope } from "../../ui/poll.js";
import { data, ui } from "../../ui/state.js";
import { actions, render } from "../../ui/views/merge.js";
import { FakeElement, FakeNode, fakeDocument, type FakeDocument } from "./support/fake-dom.js";

type Request = { method: string; url: string; body: Record<string, unknown> | undefined; answer: (status: number, body?: unknown) => void };

const HOSTILE = `<img src=x onerror="alert(1)"><b>bold</b>`;

const finding = (id: string, tier: Finding["tier"], status: Finding["status"], over: Partial<Finding> = {}): Finding => ({
  id, tier, status,
  candidate: { id: `candidate:${id}`, decision: `Decision ${id}`, scope: ["src/api/**"], level: "code", author: "jbarnes", createdAt: "2026-11-02T10:00:00Z" },
  peer: { kind: "branch", ref: "jbarnes/session-retry", commit: "abc", number: null, title: null, url: null, author: null, trusted: true },
  reason: "Your branch's changes pass its rule.",
  evidence: { violations: [], conflicts: [], mine: null, judgment: null },
  message: null,
  held: null,
  updatedAt: "2026-11-02T11:00:00Z",
  ...over,
});

const own = (id: string, state: OwnDecision["state"]): OwnDecision => ({
  id, decision: `Own ${id}`, generality: "branch", scope: ["src/api/**"], level: "note", state, createdAt: "2026-11-02T09:00:00Z",
  committed: false, amends: null, amendedBy: null, source: { sessionId: null, at: null },
});

const fixture = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => ({
  repositoryKey: "remote:github.com/acme/storefront",
  checkout: "/work/a",
  branch: "tbelanger/export-api",
  baseBranch: "main",
  enabled: false,
  readiness: {
    judge: { available: false, runner: null, problem: "Neither claude nor codex is on PATH." },
    github: { state: "signed-out", login: null, repository: null },
    candidatesIgnored: false,
    rulesActive: true,
  },
  budget: { used: 0, limit: 40, windowMinutes: 60 },
  scanning: false,
  lastScan: null,
  own: [],
  findings: [],
  conflicts: [],
  decisionLog: { questions: [], problems: [] },
  ...over,
});

/**
 * On, with one finding in each section, a decision of this branch and one marked as not a rule. A pull request's text
 * is hostile, and its decision file claims a teammate as its author.
 */
const onFixture = (): MergeAssistStatus => fixture({
  enabled: true,
  readiness: {
    judge: { available: true, runner: "claude", problem: null },
    github: { state: "signed-in", login: "tbelanger", repository: "acme/storefront" },
    candidatesIgnored: false,
    rulesActive: true,
  },
  lastScan: { at: "2026-11-02T11:58:00Z", peers: { branches: 2, pullRequests: 1 }, problems: [] },
  findings: [
    finding("talk", "talk", "open", {
      evidence: { violations: [], conflicts: [], mine: { id: "candidate:mine", decision: "Errors are thrown." }, judgment: { verdict: "contradicts", explanation: "They disagree." } },
      message: "Hi Jeremy, we decided differently about errors.",
    }),
    finding("review", "review", "open", {
      candidate: { id: "candidate:review", decision: HOSTILE, scope: ["src/**"], level: "code", author: "tbelanger", createdAt: "2026-11-02T10:00:00Z" },
      peer: { kind: "pull-request", ref: "outside/x", commit: "def", number: 7, title: HOSTILE, url: "javascript:alert(1)", author: HOSTILE, trusted: false },
      reason: "Its author is not a collaborator.",
      evidence: { violations: [{ path: "src/api/client.ts", line: 12, message: HOSTILE }], conflicts: [], mine: null, judgment: null },
    }),
    finding("added", "absorb", "applied", {
      peer: { kind: "pull-request", ref: "jbarnes/x", commit: "123", number: 12, title: "Retry layer", url: "https://github.com/acme/storefront/pull/12", author: "jbarnes", trusted: true },
    }),
  ],
  own: [own("candidate:mine", "active"), own("candidate:old", "rejected")],
});

describe("the Merge Assist page", () => {
  let requests: Request[] = [];
  let root: FakeElement;
  let document: FakeDocument;
  const writes = () => requests.filter(r => r.method !== "GET");
  const button = (act: string, id?: string) =>
    root.querySelector(id === undefined ? `button[data-act="${act}"]` : `button[data-act="${act}"][data-i="${id}"]`);
  const toastText = () => document.getElementById("toast")!.textContent;
  /** What a click on `el` runs, as `app.js` dispatches it; nothing for a disabled button. */
  const click = (el: FakeElement | null) => {
    if (el === null) throw new Error("no such button");
    if (el.disabled) return undefined;
    return actions[el.dataset.act!]!(el.dataset.i, el);
  };
  const section = (name: string) => root.querySelector(`section[data-section="${name}"]`)!;

  beforeEach(() => {
    requests = [];
    document = fakeDocument();
    vi.stubGlobal("document", document);
    vi.stubGlobal("Node", FakeNode);
    vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn(async () => {}) } });
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
    root = new FakeElement("div");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  describe("while off", () => {
    beforeEach(() => {
      data.mergeAssist = fixture();
      render(root);
    });

    it("says what turning it on does and what Rerail found, with the one button", () => {
      const card = root.querySelector(".mconsent")!;
      expect(card.textContent).toContain("What turning it on does");
      expect(card.querySelectorAll("ul.mbul li")).toHaveLength(4);
      expect(card.textContent).toContain(".rerail/candidates/");
      const checks = card.querySelectorAll("ul.mchecks li");
      expect(checks.map(li => [li.dataset.check, li.className])).toEqual([["judge", "mnotready"], ["github", "mnotready"], ["ignored", "mready"], ["rules", "mready"]]);
      expect(checks[0]!.textContent).toContain("need Claude Code or Codex on your PATH");
      expect(button("maon")!.textContent).toBe("Turn on Merge Assist");
      expect(root.querySelector(".mstrip")).toBeNull();
      expect(section("own").hidden).toBe(true);
    });

    it("turning on sends the setting for the checkout the page shows, holds the button, then shows the page on", async () => {
      const turning = click(button("maon"));
      render(root);
      expect(button("maon")!.disabled).toBe(true);
      expect(button("maon")!.textContent).toBe("Turning on…");
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]).toMatchObject({
        method: "PUT", url: expect.stringMatching(/\/api\/merge-assist\/settings$/u),
        body: { repository: "A", checkout: "/work/a", enabled: true },
      });

      writes()[0]!.answer(200, { ...fixture(), enabled: true });
      await turning;
      render(root);

      expect(toastText()).toMatch(/^Merge Assist is on in storefront\./u);
      expect(root.querySelector(".mconsent")).toBeNull();
      expect(root.querySelector(".mstrip")!.textContent).toContain("Not scanned yet");
      expect(section("own").textContent).toContain("No decisions on this branch yet");
      expect(section("own").textContent).toContain("don’t wrap API calls in retry loops");
    });

    it("says why when turning on fails, and offers the button again", async () => {
      const turning = click(button("maon"));
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      writes()[0]!.answer(403, { error: "agent-not-permitted", message: "Only a person can turn Merge Assist on." });
      await turning;
      render(root);

      expect(toastText()).toContain("Couldn’t turn on Merge Assist. Only a person can turn Merge Assist on.");
      expect(button("maon")!.disabled).toBe(false);
      expect(data.mergeAssist!.enabled).toBe(false);
    });
  });

  describe("while on", () => {
    beforeEach(() => {
      data.mergeAssist = onFixture();
      render(root);
    });

    it("shows the sections in order with their counts, the decision marked as not a rule folded", () => {
      const shown = root.querySelectorAll("section.msec").filter(s => s.closest("[hidden]") === null).map(s => s.querySelector("h2")!.textContent);
      expect(shown).toEqual(["Decided differently1", "Needs your review1", "Added from other branches1", "Decisions on this branch1"]);
      const fold = section("own").querySelector("details.mfold")!;
      expect(fold.hidden).toBe(false);
      expect(fold.hasAttribute("open")).toBe(false);
      expect(fold.querySelector("summary")!.textContent).toBe("Not rules (1)");
      expect(root.querySelector(".mstrip")!.textContent).toContain("Signed in as tbelanger");
      expect(root.querySelector(".mstrip")!.textContent).toContain("0 of 40 this hour");
    });

    it("shows both decisions of a disagreement and copies the message for its author", async () => {
      const talk = section("talk");
      expect(talk.querySelectorAll(".mside").map(side => side.textContent)).toEqual(["Your decisionErrors are thrown.", "jbarnes’s decisionDecision talk"]);
      await click(button("macopy", "talk"));
      await vi.waitFor(() => expect(toastText()).toBe("Copied. Send it to jbarnes."));
      expect(vi.mocked(navigator.clipboard.writeText)).toHaveBeenCalledWith("Hi Jeremy, we decided differently about errors.");
    });

    it("renders a peer's text as text and links only a pull request URL it can trust", () => {
      const review = section("review");
      expect(review.querySelectorAll("img")).toHaveLength(0);
      expect(review.querySelectorAll("b").map(b => b.className)).toEqual(["mauthor"]);
      expect(review.querySelector(".mq")!.textContent).toBe(HOSTILE);
      expect(review.textContent).toContain(`src/api/client.ts:12 — ${HOSTILE}`);
      expect(review.textContent).toContain("Author not verified");
      expect(review.querySelectorAll("a")).toHaveLength(0);

      const link = section("added").querySelector("a")!;
      expect(link.getAttribute("href")).toBe("https://github.com/acme/storefront/pull/12");
      expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    });

    it("names a pull request's author as GitHub reports it, and a local branch's as its decision file declares it", () => {
      const review = section("review");
      expect(review.querySelector(".mauthor")!.textContent).toBe(HOSTILE);
      expect(review.textContent).toContain(`by ${HOSTILE}`);
      // The teammate the pull request's decision file names never shows.
      expect(review.textContent).not.toContain("tbelanger");
      expect(section("added").textContent).toContain("by jbarnes");
      expect(section("talk").querySelector(".mmeta")!.textContent).toContain("declared by jbarnes");
    });

    it("Adopt sends the finding's adopt, keeps the row busy, and toasts once it applies", async () => {
      const adopting = click(button("maadopt", "review"));
      render(root);
      expect(button("maadopt", "review")!.disabled).toBe(true);
      expect(button("madismiss", "review")!.disabled).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]).toMatchObject({
        method: "PUT", url: expect.stringMatching(/\/api\/merge-assist\/findings$/u),
        body: { repository: "A", checkout: "/work/a", findingId: "review", action: "adopt" },
      });

      const after = onFixture();
      after.findings = after.findings.map(f => (f.id === "review" ? { ...f, status: "applied" } : f));
      writes()[0]!.answer(200, after);
      await adopting;
      render(root);

      expect(toastText()).toMatch(/^Adopted “/u);
      expect(section("review").hidden).toBe(true);
      expect(section("added").querySelectorAll("article")).toHaveLength(2);
    });

    it("Dismiss hides the finding at once and brings it back when the write fails", async () => {
      const dismissing = click(button("madismiss", "talk"));
      render(root);
      expect(section("talk").hidden).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", findingId: "talk", action: "dismiss" });

      writes()[0]!.answer(409, { error: "invalid-state", message: "This finding is no longer open." });
      await dismissing;
      render(root);

      expect(section("talk").hidden).toBe(false);
      expect(toastText()).toContain("Couldn’t dismiss “Decision talk”. This finding is no longer open.");
    });

    it("Undo removes an added decision from this checkout", async () => {
      const undoing = click(button("maundo", "added"));
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]!.body).toEqual({ repository: "A", checkout: "/work/a", findingId: "added", action: "undo" });
      const after = onFixture();
      after.findings = after.findings.filter(f => f.id !== "added");
      writes()[0]!.answer(200, after);
      await undoing;
      render(root);

      expect(toastText()).toBe("Removed “Decision added” from this checkout. It no longer steers your coding agent.");
      expect(section("added").textContent).toContain("Nothing added yet");
    });

    it("Not a rule moves the decision at once, and its toast's Undo restores it", async () => {
      const marking = click(button("mareject", "candidate:mine"));
      render(root);
      expect(section("own").querySelector("details.mfold summary")!.textContent).toBe("Not rules (2)");
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]).toMatchObject({
        method: "PUT", url: expect.stringMatching(/\/api\/merge-assist\/candidates$/u),
        body: { repository: "A", checkout: "/work/a", candidateId: "candidate:mine", state: "rejected" },
      });
      const after = onFixture();
      after.own = after.own.map(d => ({ ...d, state: "rejected" as const }));
      writes()[0]!.answer(200, after);
      await marking;
      expect(toastText()).toContain("Marked “Own candidate:mine” as not a rule. It no longer steers here. Commit the change so teammates who added it drop it too.");

      const undo = document.getElementById("toast")!.querySelector("button.tact")!;
      expect(undo.dataset.act).toBe("marestore");
      expect(undo.dataset.i).toBe("candidate:mine");
      const restoring = click(undo);
      await vi.waitFor(() => expect(writes()).toHaveLength(2));
      expect(writes()[1]!.body).toEqual({ repository: "A", checkout: "/work/a", candidateId: "candidate:mine", state: "active" });
      writes()[1]!.answer(200, onFixture());
      await restoring;
      expect(toastText()).toBe("“Own candidate:mine” is a decision on this branch again.");
    });

    it("Scan now posts a scan for the checkout and says what it found that was not shown before", async () => {
      data.mergeAssist = { ...onFixture(), findings: onFixture().findings.filter(f => f.id !== "review") };
      render(root);
      const scanning = click(button("mascan"));
      render(root);
      expect(button("mascan")!.textContent).toBe("Scanning…");
      expect(button("mascan")!.disabled).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      expect(writes()[0]).toMatchObject({ method: "POST", url: expect.stringMatching(/\/api\/merge-assist\/scan$/u), body: { repository: "A", checkout: "/work/a" } });
      writes()[0]!.answer(200, onFixture());
      await scanning;

      expect(toastText()).toBe("Scanned 2 local branches and 1 open pull request. New: 1 needs your review.");
    });

    it("says when another scan holds the checkout, and shows the last results", async () => {
      const scanning = click(button("mascan"));
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      writes()[0]!.answer(200, { ...onFixture(), scanning: true });
      await scanning;
      render(root);

      expect(toastText()).toBe("Another scan is running; showing the last results.");
      expect(button("mascan")!.disabled).toBe(true);
    });

    it("keeps the focused button through Scan now and Turn off: it is updated in place, not rebuilt", async () => {
      const scanButton = button("mascan"), offButton = button("maoff");
      const scanning = click(scanButton);
      render(root);
      expect(button("mascan")).toBe(scanButton);
      expect(scanButton!.disabled).toBe(true);
      await vi.waitFor(() => expect(writes()).toHaveLength(1));
      writes()[0]!.answer(200, onFixture());
      await scanning;
      render(root);
      expect(button("mascan")).toBe(scanButton);
      expect([scanButton!.disabled, scanButton!.textContent]).toEqual([false, "Scan now"]);

      const turningOff = click(offButton);
      render(root);
      expect(button("maoff")).toBe(offButton);
      expect([offButton!.disabled, offButton!.textContent]).toEqual([true, "Turning off…"]);
      await vi.waitFor(() => expect(writes()).toHaveLength(2));
      writes()[1]!.answer(500, { error: "internal", message: "Disk full." });
      await turningOff;
      render(root);
      expect(button("maoff")).toBe(offButton);
      expect(offButton!.disabled).toBe(false);
    });

    it("moves the times it shows on without rebuilding their rows", () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      try {
        vi.setSystemTime(Date.parse("2026-11-02T11:04:00Z"));
        render(root);
        const row = section("added").querySelector("article")!;
        expect(row.textContent).toContain("Added automatically 4m ago");
        vi.setSystemTime(Date.parse("2026-11-02T14:00:00Z"));
        render(root);
        expect(section("added").querySelector("article")).toBe(row);
        expect(row.textContent).toContain("Added automatically 3h ago");
      } finally {
        vi.useRealTimers();
      }
    });

    it("shows why it cannot read Merge Assist and nothing to act on, not the last status", () => {
      data.mergeAssistError = "/api/merge-assist?repository=A answered 500";
      render(root);
      expect(root.querySelector('[role="alert"]')!.textContent).toContain("Couldn’t read Merge Assist for this checkout.");
      expect(root.querySelectorAll("button[data-act]").filter(b => b.closest("[hidden]") === null).map(b => b.dataset.act)).toEqual([]);
      expect(root.querySelectorAll("section.msec").filter(s => s.closest("[hidden]") === null)).toEqual([]);
      expect(root.querySelector(".mstrip")).toBeNull();

      data.mergeAssistError = null;
      render(root);
      expect(root.querySelector('[role="alert"]')).toBeNull();
      expect(button("mascan")!.disabled).toBe(false);
    });

    it("forgets a scan or a switch in flight when the page moves to another repository", async () => {
      const scanning = click(button("mascan"));
      const turningOff = click(button("maoff"));
      await vi.waitFor(() => expect(writes()).toHaveLength(2));
      changeScope({ repository: "B", checkout: "/work/b" });
      data.repository = { setUp: true, repositoryKey: "B", name: "billing" };
      data.mergeAssist = { ...onFixture(), checkout: "/work/b" };
      render(root);
      expect([button("mascan")!.disabled, button("mascan")!.textContent]).toEqual([false, "Scan now"]);
      expect(button("maoff")!.disabled).toBe(false);

      // The first repository's answers land nowhere, and leave the second's buttons alone.
      writes()[0]!.answer(200, onFixture());
      writes()[1]!.answer(200, { ...onFixture(), enabled: false });
      await Promise.all([scanning, turningOff]);
      render(root);
      expect(data.mergeAssist!.checkout).toBe("/work/b");
      expect([ui.mergeScanning, ui.mergeSwitching]).toEqual([false, false]);
    });

    it("opens the decisions marked as not a rule from their summary, and keeps them open across renders", () => {
      click(section("own").querySelector("details.mfold summary"));
      render(root);
      expect(section("own").querySelector("details.mfold")!.hasAttribute("open")).toBe(true);
      expect(ui.mergeOpen.has("notRules")).toBe(true);
    });

    it("says decisions can't steer and links Overview when Rerail is not started here", () => {
      data.mergeAssist = { ...onFixture(), readiness: { ...onFixture().readiness, rulesActive: false } };
      render(root);
      expect(root.textContent).toContain("Rerail isn’t set up and started in this checkout, so decisions can’t steer yet.");
      expect(root.querySelector(`button[data-act="nav"][data-i="overview"]`)).not.toBeNull();
    });
  });
});
