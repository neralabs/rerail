import { describe, expect, it } from "vitest";
import type { Finding, MergeAssistStatus, OwnDecision } from "../../src/engine/index.js";
import {
  authorOf, budgetText, githubText, judgeText, readinessChecks, safePeerUrl, scanSummary, scannedWhat, sectionsOf,
  waitingCount, writeError,
} from "../../ui/merge-model.js";

const NOW = Date.parse("2026-11-02T12:00:00Z");
const at = (minutesAgo: number) => new Date(NOW - minutesAgo * 60_000).toISOString();

const finding = (id: string, tier: Finding["tier"], status: Finding["status"], updatedMinutesAgo = 0): Finding => ({
  id, tier, status,
  candidate: { id: `candidate:${id}`, decision: `Decision ${id}`, scope: ["src/**"], level: "code", author: "jbarnes", createdAt: at(120) },
  peer: { kind: "branch", ref: "jbarnes/session-retry", commit: "abc", number: null, title: null, url: null, author: null, trusted: true },
  reason: "Compatible with your decisions.",
  evidence: { violations: [], conflicts: [], mine: null, judgment: null },
  message: null,
  updatedAt: at(updatedMinutesAgo),
});

const own = (id: string, state: OwnDecision["state"], createdMinutesAgo = 0): OwnDecision => ({
  id, decision: `Own ${id}`, generality: "branch", scope: ["src/api/**"], level: "note", state, createdAt: at(createdMinutesAgo),
  committed: false, source: { sessionId: null, at: null },
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
