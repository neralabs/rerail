import { describe, expect, it } from "vitest";
import type { Finding, MergeAssistStatus, OwnDecision } from "../../src/engine/index.js";
import {
  authorOf, budgetText, conflictSections, conflictToast, draftProblem, draftText, githubText, hunkLines, judgeText,
  logQuestions, logStatusCounts, readinessChecks, resolutionText, resolveBody, safePeerUrl, scanSummary, scannedWhat, sectionsOf, tabOf,
  waitingCount, writeError,
} from "../../ui/merge-model.js";
import { conflict, logItem, resolved, side } from "./support/merge-conflicts.js";

const NOW = Date.parse("2026-11-02T12:00:00Z");
const at = (minutesAgo: number) => new Date(NOW - minutesAgo * 60_000).toISOString();

const finding = (id: string, tier: Finding["tier"], status: Finding["status"], updatedMinutesAgo = 0): Finding => ({
  id, tier, status,
  candidate: { id: `candidate:${id}`, decision: `Decision ${id}`, scope: ["src/**"], level: "code", author: "jbarnes", createdAt: at(120) },
  peer: { kind: "branch", ref: "jbarnes/session-retry", commit: "abc", number: null, title: null, url: null, author: null, trusted: true },
  reason: "Compatible with your decisions.",
  evidence: { violations: [], conflicts: [], mine: null, judgment: null },
  message: null,
  held: null,
  updatedAt: at(updatedMinutesAgo),
});

const own = (id: string, state: OwnDecision["state"], createdMinutesAgo = 0): OwnDecision => ({
  id, decision: `Own ${id}`, generality: "branch", scope: ["src/api/**"], level: "note", state, createdAt: at(createdMinutesAgo),
  committed: false, amends: null, amendedBy: null, source: { sessionId: null, at: null },
});

const status = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => ({
  repositoryKey: "remote:github.com/acme/storefront",
  checkout: "/work/storefront",
  branch: "tbelanger/export-api",
  baseBranch: "main",
  enabled: true,
  readiness: {
    judge: { available: true, runner: "claude", problem: null },
    github: { state: "signed-in", login: "tbelanger", repository: "acme/storefront" },
    candidatesIgnored: false,
    rulesActive: true,
  },
  budget: { used: 3, limit: 40, windowMinutes: 60 },
  scanning: false,
  lastScan: { at: at(4), peers: { branches: 3, pullRequests: 2 }, problems: [] },
  own: [],
  findings: [],
  conflicts: [],
  decisionLog: { questions: [], problems: [] },
  ...over,
});

describe("the page's sections", () => {
  const findings = [
    finding("talk-old", "talk", "open", 30),
    finding("review", "review", "open", 5),
    finding("talk-new", "talk", "open", 1),
    finding("absorbed", "absorb", "applied", 10),
    finding("adopted", "review", "applied", 2),
    finding("dismissed", "review", "dismissed"),
  ];

  it("sorts open contradictions into Decided differently and other open findings into Needs your review, newest first", () => {
    const sections = sectionsOf(status({ findings }));
    expect(sections.talk.map(f => f.id)).toEqual(["talk-new", "talk-old"]);
    expect(sections.review.map(f => f.id)).toEqual(["review"]);
  });

  it("lists absorbed and adopted findings under Added from other branches and hides dismissed ones", () => {
    const sections = sectionsOf(status({ findings }));
    expect(sections.added.map(f => f.id)).toEqual(["adopted", "absorbed"]);
    expect(Object.values(sections).flat().map(item => item.id)).not.toContain("dismissed");
  });

  it("splits this branch's decisions into active and not rules, newest first", () => {
    const sections = sectionsOf(status({ own: [own("a", "active", 60), own("b", "rejected", 5), own("c", "active", 1)] }));
    expect(sections.own.map(d => d.id)).toEqual(["c", "a"]);
    expect(sections.notRules.map(d => d.id)).toEqual(["b"]);
  });

  it("drops a finding being dismissed or undone at once, and keeps one being adopted in place, busy", () => {
    const pending = new Map([["finding:talk-new", "dismiss"], ["finding:absorbed", "undo"], ["finding:review", "adopt"]]);
    const sections = sectionsOf(status({ findings }), pending);
    expect(sections.talk.map(f => f.id)).toEqual(["talk-old"]);
    expect(sections.added.map(f => f.id)).toEqual(["adopted"]);
    expect(sections.review).toEqual([expect.objectContaining({ id: "review", busy: true })]);
    expect(sections.talk[0]!.busy).toBe(false);
  });

  it("moves a decision marked as not a rule to Not rules at once, and a restored one back", () => {
    const pending = new Map([["candidate:a", "rejected"], ["candidate:b", "active"]]);
    const sections = sectionsOf(status({ own: [own("a", "active"), own("b", "rejected")] }), pending);
    expect(sections.own).toEqual([expect.objectContaining({ id: "b", state: "active", busy: true })]);
    expect(sections.notRules).toEqual([expect.objectContaining({ id: "a", state: "rejected", busy: true })]);
  });
});

describe("the sidebar's count", () => {
  const findings = [finding("t", "talk", "open"), finding("r", "review", "open"), finding("a", "absorb", "applied")];

  it("counts the findings that wait for the person: decided differently and needs review", () => {
    expect(waitingCount(status({ findings }))).toBe(2);
  });

  it("is 0 while Merge Assist is off, before it loads, and leaves out a finding being dismissed", () => {
    expect(waitingCount(status({ findings, enabled: false }))).toBe(0);
    expect(waitingCount(null)).toBe(0);
    expect(waitingCount(status({ findings }), new Map([["finding:t", "dismiss"]]))).toBe(1);
  });
});

describe("who is behind a finding", () => {
  it("names a pull request's author as GitHub reports it, never the one its decision file declares", () => {
    const pullRequest = { ...finding("p", "review", "open"), peer: { ...finding("p", "review", "open").peer, kind: "pull-request" as const, number: 7, author: "outsider" } };
    expect(authorOf({ ...pullRequest, candidate: { ...pullRequest.candidate, author: "tbelanger" } })).toEqual({ name: "outsider", declared: false });
    expect(authorOf({ ...pullRequest, peer: { ...pullRequest.peer, author: null } })).toEqual({ name: null, declared: false });
  });

  it("names a local branch's author as its decision file declares it, marked as declared", () => {
    expect(authorOf(finding("b", "review", "open"))).toEqual({ name: "jbarnes", declared: true });
  });
});

describe("a peer's link", () => {
  const onGithub = { repositoryKey: "remote:github.com/acme/storefront", repository: "acme/storefront" };
  const onEnterprise = { repositoryKey: "remote:github.example.com/acme/storefront", repository: "acme/storefront" };

  it("opens a pull request of this repository on github.com or on its own GitHub Enterprise host", () => {
    expect(safePeerUrl("https://github.com/acme/storefront/pull/12", onGithub)).toBe("https://github.com/acme/storefront/pull/12");
    expect(safePeerUrl("https://github.example.com/acme/storefront/pull/12", onEnterprise)).toBe("https://github.example.com/acme/storefront/pull/12");
  });

  it.each([
    "javascript:alert(1)",
    "http://github.com/acme/storefront/pull/12",
    "https://evil.example/acme/storefront/pull/12",
    "https://github.com.evil.example/acme/storefront/pull/12",
    "https://github.example.com/acme/storefront/pull/12",
    "https://notgithub.com/acme/storefront/pull/12",
    "https://user:token@github.com/acme/storefront/pull/12",
    "https://github.com:8443/acme/storefront/pull/12",
    "https://github.com/someone-else/storefront/pull/12",
    " https://github.com/acme/storefront/pull/12",
    "data:text/html,<script>alert(1)</script>",
    null,
    42,
  ])("refuses %s for a github.com repository", url => {
    expect(safePeerUrl(url, onGithub)).toBeNull();
  });

  it("accepts only github.com when the repository key names no GitHub host, as with an ssh alias", () => {
    const aliased = { repositoryKey: "remote:github-work/acme/storefront", repository: "acme/storefront" };
    expect(safePeerUrl("https://github.com/acme/storefront/pull/3", aliased)).toBe("https://github.com/acme/storefront/pull/3");
    expect(safePeerUrl("https://github.example.com/acme/storefront/pull/3", aliased)).toBeNull();
  });
});

describe("the status strip", () => {
  it("names the judging CLI, or says none was found with why", () => {
    expect(judgeText({ available: true, runner: "codex", problem: null })).toEqual({ ok: true, text: "Codex" });
    expect(judgeText({ available: false, runner: null, problem: "Neither claude nor codex is on PATH." }))
      .toEqual({ ok: false, text: "No agent CLI found", detail: "Neither claude nor codex is on PATH." });
  });

  it("says gh's state with the login it is signed in as", () => {
    expect(githubText({ state: "signed-in", login: "tbelanger", repository: "acme/storefront" })).toEqual({ ok: true, text: "Signed in as tbelanger" });
    expect(githubText({ state: "signed-out", login: null, repository: null }).ok).toBe(false);
  });

  it("says what a scan read and the budget used this hour", () => {
    expect(scannedWhat({ at: at(1), peers: { branches: 1, pullRequests: 2 }, problems: [] })).toBe("1 local branch and 2 open pull requests");
    expect(scannedWhat({ at: at(1), peers: { branches: 0, pullRequests: 0 }, problems: [] })).toBe("no other branches");
    expect(budgetText({ used: 12, limit: 40, windowMinutes: 60 })).toEqual({ text: "12 of 40 this hour", spent: false });
    expect(budgetText({ used: 40, limit: 40, windowMinutes: 60 }).spent).toBe(true);
  });

  it("sums up what a scan found that the page did not show before, not the totals", () => {
    const findings = [finding("t", "talk", "open"), finding("r1", "review", "open"), finding("r2", "review", "open")];
    expect(scanSummary(null, status({ findings }))).toBe("Scanned 3 local branches and 2 open pull requests. New: 1 decided differently, 2 need your review.");
    // Nothing changed: the findings that still wait are not news.
    expect(scanSummary(status({ findings }), status({ findings }))).toBe("Scanned 3 local branches and 2 open pull requests: nothing new.");
    // A finding that moved section, or whose decision changed, is.
    const moved = [finding("t", "talk", "open"), finding("r1", "absorb", "applied"), { ...finding("r2", "review", "open"), candidate: { ...finding("r2", "review", "open").candidate, decision: "Changed." } }];
    expect(scanSummary(status({ findings }), status({ findings: moved }))).toBe("Scanned 3 local branches and 2 open pull requests. New: 1 needs your review, 1 added from other branches.");
    expect(scanSummary(null, status({ lastScan: { at: at(0), peers: { branches: 1, pullRequests: 0 }, problems: ["gh: rate limited"] } })))
      .toBe("Scanned 1 local branch: nothing new. 1 thing could not be read; see Last scan.");
  });

  it("says a scan request found another scan running, and that the results are the last ones", () => {
    expect(scanSummary(status(), status({ scanning: true }))).toBe("Another scan is running; showing the last results.");
  });
});

describe("the readiness checklist", () => {
  const text = (segments: Array<string | { code: string }>) => segments.map(s => (typeof s === "string" ? s : s.code)).join("");

  it("names the supported CLIs when none is found, and says scans still apply exact checks", () => {
    const checks = readinessChecks({ ...status().readiness, judge: { available: false, runner: null, problem: null } });
    const judge = checks.find(c => c.key === "judge")!;
    expect(judge.ok).toBe(false);
    expect(text(judge.text)).toMatch(/need Claude Code or Codex on your PATH\. Without one, scans still apply exact checks\./u);
  });

  it("says whether gh is signed in, the candidates folder ignored, and rules able to steer", () => {
    const ready = readinessChecks(status().readiness);
    expect(ready.map(c => [c.key, c.ok])).toEqual([["judge", true], ["github", true], ["ignored", true], ["rules", true]]);
    expect(text(ready.find(c => c.key === "github")!.text)).toContain("gh is signed in as tbelanger");

    const blocked = readinessChecks({ ...status().readiness, github: { state: "signed-out", login: null, repository: null }, candidatesIgnored: true, rulesActive: false });
    expect(blocked.map(c => c.ok)).toEqual([true, false, false, false]);
    expect(text(blocked.find(c => c.key === "github")!.text)).toContain("Run gh auth login");
    expect(text(blocked.find(c => c.key === "ignored")!.text)).toContain("Git ignores .rerail/candidates/ here");
    expect(blocked.find(c => c.key === "rules")).toMatchObject({ overview: true, text: ["Rerail isn't set up and started in this checkout, so decisions can't steer yet."] });
  });
});

describe("a failed write's reason", () => {
  it("is the API's message when it answered one, else the error as sent", () => {
    const refused = new Error('PUT /api/merge-assist/findings answered 409: {"error":"invalid-state","message":"This finding is no longer open."}');
    expect(writeError(refused)).toBe("This finding is no longer open.");
    expect(writeError(new Error("PUT /api/merge-assist/findings answered 500: oops"))).toBe("PUT /api/merge-assist/findings answered 500: oops");
    expect(writeError(new Error("network down"))).toBe("network down");
  });
});

describe("conflicts", () => {
  const conflicts = [
    conflict("logged-new", { level: "logged", detectedAt: at(1) }),
    conflict("ambient-old", { detectedAt: at(60) }),
    resolved("by-precedent", "theirs", "precedent"),
    conflict("ambient-new", { detectedAt: at(5) }),
    conflict("dismissed", { status: "dismissed", resolution: { action: "dismiss", by: "person", name: "tbelanger", logId: null, rule: null, text: null, at: at(2) } }),
  ];

  it("lists open conflicts that reach the session first, then logged ones, each newest first; closed ones as the status orders them", () => {
    const { open, closed } = conflictSections(status({ conflicts }));
    expect(open.map(c => c.id)).toEqual(["ambient-new", "ambient-old", "logged-new"]);
    expect(closed.map(c => c.id)).toEqual(["by-precedent", "dismissed"]);
    expect(open.every(c => !c.busy)).toBe(true);
  });

  it("hides a conflict marked as not a conflict at once and keeps one being resolved, busy", () => {
    const pending = new Map([["conflict:ambient-old", "dismiss"], ["conflict:ambient-new", "theirs"]]);
    const { open } = conflictSections(status({ conflicts }), pending);
    expect(open.map(c => [c.id, c.busy])).toEqual([["ambient-new", true], ["logged-new", false]]);
  });

  it("opens on Conflicts while any is open, else Decisions, and keeps the tab the person chose", () => {
    expect(tabOf(status({ conflicts }), null)).toBe("conflicts");
    expect(tabOf(status({ conflicts: [resolved("r", "mine", "person")] }), null)).toBe("decisions");
    expect(tabOf(status({ conflicts }), "log")).toBe("log");
    expect(tabOf(status({ conflicts }), "constructor")).toBe("conflicts");
  });

  it("counts every open conflict in the sidebar as the Conflicts tab does, and a decided-differently finding only when no conflict carries it", () => {
    const talk = [finding("carried", "talk", "open"), finding("alone", "talk", "open")];
    const carrier = conflict("decisions", { kind: "decisions", theirs: side({ label: "jbarnes/x", decisionId: "candidate:carried" }) });
    const all = status({ findings: talk, conflicts: [...conflicts, carrier] });
    // ambient-old, ambient-new, the logged one, the decisions conflict, and the finding no conflict carries.
    expect(conflictSections(all).open).toHaveLength(4);
    expect(waitingCount(all)).toBe(conflictSections(all).open.length + 1);
    expect(waitingCount(status({ conflicts, enabled: false }))).toBe(0);
  });

  it("says how a closed conflict ended, who decided, and the rule it saved", () => {
    expect(resolutionText(resolved("p", "theirs", "precedent"))).toMatchObject({
      badge: "Resolved by precedent", precedent: true, how: "Took theirs (PR #398)", who: "by precedent", rule: ".rerail/candidates/p.json",
    });
    expect(resolutionText(resolved("m", "mine", "person"))).toMatchObject({ badge: "Resolved", how: "Kept mine", who: "by tbelanger" });
    expect(resolutionText(resolved("n", "new", "person"))).toMatchObject({ how: "New decision" });
    expect(resolutionText(conflicts[4]!)).toMatchObject({ badge: "Not a conflict", how: null, who: "by tbelanger", rule: null });
  });

  it("opens Keep mine and Take theirs on that side's summary, a new decision empty, and asks for 1 to 600 characters", () => {
    const c = conflict("c");
    expect(draftText(c, "mine")).toBe("Clones each matcher per rule copy.");
    expect(draftText(c, "theirs")).toBe("Shares compiled matchers; no rule copy deep-copies them.");
    expect(draftText(c, "new")).toBe("");
    expect(draftProblem("   ")).toBe("Write the decision first.");
    expect(draftProblem("x".repeat(600))).toBeNull();
    expect(draftProblem("x".repeat(602))).toBe("2 characters over the 600-character limit.");
  });

  it("sends the conflict, the action and the trimmed text; Not a conflict sends no text", () => {
    expect(resolveBody("conflict:q:ab", "theirs", "  Share matchers.  ")).toEqual({ conflictId: "conflict:q:ab", action: "theirs", text: "Share matchers." });
    expect(resolveBody("conflict:q:ab", "new", "Both, behind a flag.")).toEqual({ conflictId: "conflict:q:ab", action: "new", text: "Both, behind a flag." });
    expect(resolveBody("conflict:q:ab", "dismiss")).toEqual({ conflictId: "conflict:q:ab", action: "dismiss" });
  });

  it("says what a resolve did, and suggests committing only when Git does not ignore the candidates", () => {
    const c = conflict("c");
    expect(conflictToast(c, "theirs", "Share matchers.", false).done)
      .toBe("Took theirs. Saved “Share matchers.” as a decision on this branch. It steers your coding agent from its next turn. Commit it to share it with teammates.");
    expect(conflictToast(c, "mine", "Clone.", true).done).toBe("Kept mine. Saved “Clone.” as a decision on this branch. It steers your coding agent from its next turn.");
    expect(conflictToast(c, "new", "Both.", true).fail).toBe("Couldn’t save the decision for “Shared matchers c”");
    expect(conflictToast(c, "dismiss", "", false)).toEqual({
      done: "Marked “Shared matchers c” as not a conflict. It stays hidden for PR #398.",
      fail: "Couldn’t mark “Shared matchers c” as not a conflict",
    });
  });

  it("colours a hunk's added, removed and header lines", () => {
    expect(hunkLines("@@ -1 +1 @@\n-old\n+new\n same\n").map(l => l.kind)).toEqual(["meta", "del", "add", "ctx"]);
  });

  it("lists each question's ledger entries newest first, filtered by status, and counts them per status", () => {
    const items = [logItem("old", at(600)), logItem("new", at(1), { status: "in-force" }), logItem("mid", at(30), { status: "void" })];
    const decisionLog = { questions: [{ question: "q", title: "Q", items }, { question: "r", title: "R", items: [logItem("other", at(5))] }], problems: [] };
    expect(logQuestions(status({ decisionLog })).map(q => [q.question, q.items.map(e => e.id)])).toEqual([["q", ["new", "mid", "old"]], ["r", ["other"]]]);
    expect(logQuestions(status({ decisionLog }), "void").map(q => [q.question, q.items.map(e => e.id)])).toEqual([["q", ["mid"]]]);
    expect(logStatusCounts(status({ decisionLog }))).toEqual({ "in-force": 1, provisional: 2, reopened: 0, flagged: 0, superseded: 0, void: 1 });
  });
});
