import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadRepoRuleCounts, repoLine, repoRuleCounts, repoTag, ruleCountsOf } from "../../ui/repo-rules.js";
import { agentStatus, installedLabel } from "../../ui/settings-model.js";

const agent = (over: Partial<{ found: boolean; connected: boolean; problem: string | null }>) =>
  ({ id: "claude-code", name: "Claude Code", found: false, connected: false, problem: null, ...over });

/** One entry of `GET /api/repository/rules`, as `toStoredRules` reads it. */
const entry = (id: string, { on = true, folder = "", configError = null as string | null } = {}) => ({
  kind: "code", rule: { id }, folder, on, mode: "log", source: { kind: "pack", id: "typescript", packId: "typescript" },
  onSource: "default", modeSource: "default", suppressions: "honour", suppressionsSource: "default",
  config: null, configSource: {}, configError, revision: 1, payloadHash: "x", addedAt: null, updatedAt: null,
});

describe("a coding agent's state in Settings", () => {
  it("offers the disconnect command for a connected agent, by its id", () => {
    expect(agentStatus(agent({ found: true, connected: true }))).toMatchObject({
      state: "Connected", tone: "on", command: { text: "rerail install --remove --agents claude-code" },
    });
  });

  it("offers the connect command for an agent on this computer that is not connected", () => {
    expect(agentStatus(agent({ found: true }))).toMatchObject({
      state: "On this computer, not connected", command: { text: "rerail install --agents claude-code" },
    });
  });

  it("offers nothing for an agent not on this machine", () => {
    expect(agentStatus(agent({}))).toMatchObject({ state: "Not found on this machine", tone: "off", command: null });
  });

  it("says the check did not finish when there is a problem, even for an agent that reads connected", () => {
    expect(agentStatus(agent({ found: true, connected: true, problem: "settings.json is not valid JSON" }))).toMatchObject({
      state: "Couldn’t check", command: null, problem: "settings.json is not valid JSON",
    });
  });
});

describe("a repo's rule tag", () => {
  it("reads No rules added for a repo set up with nothing added", () => {
    expect(repoTag(ruleCountsOf([]))).toBe("No rules added");
  });

  it("reads 0 rules on when every rule is off, not No rules added", () => {
    expect(repoTag(ruleCountsOf([entry("a", { on: false }), entry("b", { on: false })]))).toBe("0 rules on");
  });

  it("counts a rule added at two folders once, on only where it is on in both", () => {
    const counts = ruleCountsOf([entry("a"), entry("a", { folder: "web" }), entry("b"), entry("b", { folder: "web", on: false })]);
    expect(counts).toEqual({ total: 2, on: 1, awaiting: 0 });
    expect(repoTag(counts)).toBe("1 rule on");
  });

  it("does not count a rule whose options do not apply as on", () => {
    expect(ruleCountsOf([entry("a"), entry("b", { configError: "bad option" })])).toEqual({ total: 2, on: 1, awaiting: 0 });
  });

  it("says how many rules wait for approval, also when none runs yet", () => {
    expect(repoTag(ruleCountsOf([entry("a")], 2))).toBe("1 rule on · 2 waiting for approval");
    expect(repoTag(ruleCountsOf([], 1))).toBe("0 rules on · 1 waiting for approval");
  });

  it("is unknown until the counts are read", () => {
    expect(repoTag(null)).toBeNull();
  });
});

describe("a repo's line in the repo choices", () => {
  const repo = (over: Partial<{ setUp: boolean; started: boolean; checkoutCount: number }> = {}) =>
    ({ setUp: true, started: true, checkoutCount: 1, ...over });

  it("says set up, then the rules on, then the checkouts when there are several", () => {
    expect(repoLine(repo(), { total: 12, on: 12, awaiting: 0 })).toBe("Set up · 12 rules on");
    expect(repoLine(repo({ checkoutCount: 2 }), { total: 50, on: 50, awaiting: 0 })).toBe("Set up · 50 rules on · 2 checkouts");
    expect(repoLine(repo(), { total: 0, on: 0, awaiting: 0 })).toBe("Set up · No rules added");
    expect(repoLine(repo(), { total: 3, on: 3, awaiting: 1 })).toBe("Set up · 3 rules on · 1 waiting for approval");
  });

  it("says a repo that is not set up runs nothing, whatever it holds", () => {
    expect(repoLine(repo({ setUp: false, started: true }), null)).toBe("Not set up · nothing runs");
    expect(repoLine(repo({ setUp: false, checkoutCount: 3 }), { total: 2, on: 2, awaiting: 0 })).toBe("Not set up · nothing runs · 3 checkouts");
  });

  it("says Off with the rules it holds while a repo is off, and only On or Off until the counts are read", () => {
    expect(repoLine(repo({ started: false }), { total: 12, on: 0, awaiting: 0 })).toBe("Set up · Off · 12 rules");
    expect(repoLine(repo({ started: false }), { total: 12, on: 0, awaiting: 2 })).toBe("Set up · Off · 12 rules · 2 waiting for approval");
    expect(repoLine(repo({ started: false, checkoutCount: 2 }), null)).toBe("Set up · Off · 2 checkouts");
    expect(repoLine(repo(), null)).toBe("Set up · On");
  });
});

describe("the install date", () => {
  it("shows the day with its year", () => {
    expect(installedLabel(Date.UTC(2026, 8, 3, 12))).toBe("Sep 3, 2026");
  });

  it("is unknown before the database exists", () => {
    expect(installedLabel(null)).toBeNull();
  });
});

describe("reading every repo's rule counts", () => {
  type Answer = { status: number; body?: unknown; etag?: string };
  let answers: Map<string, Answer>;
  let requests: Array<{ key: string; ifNoneMatch: string | null }>;

  beforeEach(() => {
    answers = new Map();
    requests = [];
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", async (input: URL, init: { headers?: Record<string, string> } = {}) => {
      const key = new URL(input).searchParams.get("repository") ?? "";
      requests.push({ key, ifNoneMatch: init.headers?.["if-none-match"] ?? null });
      const answer = answers.get(key) ?? { status: 404 };
      const headers = answer.etag === undefined ? {} : { etag: answer.etag };
      return new Response(answer.body === undefined ? null : JSON.stringify(answer.body), { status: answer.status, headers });
    });
  });

  afterEach(async () => {
    await loadRepoRuleCounts([]);
    vi.unstubAllGlobals();
  });

  const rules = (...entries: object[]) => ({ repositoryKey: "k", checkoutPath: null, setUp: true, rules: entries, unavailable: [], awaitingApproval: 0 });

  it("reads each repo's own count, a set-up repo with no rules included", async () => {
    answers.set("store", { status: 200, etag: "\"s1\"", body: rules(entry("a"), entry("b", { on: false })) });
    answers.set("billing", { status: 200, etag: "\"b1\"", body: rules() });

    expect(await loadRepoRuleCounts(["store", "billing", "docs"])).toBe(true);

    expect(repoRuleCounts("store")).toEqual({ total: 2, on: 1, awaiting: 0 });
    expect(repoRuleCounts("billing")).toEqual({ total: 0, on: 0, awaiting: 0 });
    expect(repoRuleCounts("docs")).toBeNull();
  });

  it("keeps how many of a repo's rules wait for approval", async () => {
    answers.set("store", { status: 200, etag: "\"s1\"", body: { ...rules(entry("a")), awaitingApproval: 2 } });

    await loadRepoRuleCounts(["store"]);

    expect(repoRuleCounts("store")).toEqual({ total: 1, on: 1, awaiting: 2 });
  });

  it("asks again with the tag it holds and keeps the counts on 304", async () => {
    answers.set("store", { status: 200, etag: "\"s1\"", body: rules(entry("a")) });
    await loadRepoRuleCounts(["store"]);
    answers.set("store", { status: 304, etag: "\"s1\"" });

    expect(await loadRepoRuleCounts(["store"])).toBe(false);

    expect(requests.at(-1)).toEqual({ key: "store", ifNoneMatch: "\"s1\"" });
    expect(repoRuleCounts("store")).toEqual({ total: 1, on: 1, awaiting: 0 });
  });

  it("drops the count of a repo that is no longer listed or no longer answers", async () => {
    answers.set("store", { status: 200, etag: "\"s1\"", body: rules(entry("a")) });
    answers.set("billing", { status: 200, etag: "\"b1\"", body: rules(entry("a")) });
    await loadRepoRuleCounts(["store", "billing"]);
    answers.delete("store");

    expect(await loadRepoRuleCounts(["store"])).toBe(true);

    expect(repoRuleCounts("store")).toBeNull();
    expect(repoRuleCounts("billing")).toBeNull();
  });
});
