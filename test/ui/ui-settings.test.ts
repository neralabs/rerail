import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope } from "../../ui/poll.js";
import { loadRepoRuleCounts, repoRuleCounts } from "../../ui/repo-rule-counts.js";
import { repoLine, repoTag, ruleCountsOf } from "../../ui/repo-rules.js";
import { agentStatus, installedLabel, isNewerVersion, newerRelease, releaseUrl } from "../../ui/settings-model.js";
import { data } from "../../ui/state.js";
import { actions as settingsActions } from "../../ui/views/settings.js";
import type * as Shell from "../../ui/shell.js";

const { toast } = vi.hoisted(() => ({ toast: vi.fn() }));
// The page's toast, and no redraw: Settings' switches are driven without a page.
vi.mock("../../ui/shell.js", async importOriginal => ({ ...(await importOriginal<typeof Shell>()), invalidate: () => {}, toast }));

type DataAccess = "kept-out" | "file-tools" | "open";
const agent = (over: Partial<{ found: boolean; connected: boolean; problem: string | null; access: DataAccess; fix: string | null }>) => {
  const { access = "open", fix = null, ...rest } = over;
  return { id: "claude-code", name: "Claude Code", found: false, connected: false, problem: null, rerailData: { access, why: `${access} because`, fix }, ...rest };
};

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

  it("says whether a connected agent's tools can read Rerail's data, warning only when there is something to do", () => {
    expect(agentStatus(agent({ found: true, connected: true, access: "kept-out" })).data).toEqual({ text: "kept-out because", warn: false, fix: null });
    // Not wholly kept out, with nothing the person can change (Cursor, Codex, Claude Code on Windows): a fact, not a warning.
    expect(agentStatus(agent({ found: true, connected: true, access: "file-tools" })).data).toEqual({ text: "file-tools because", warn: false, fix: null });
    expect(agentStatus(agent({ found: true, connected: true, access: "open" })).data).toEqual({ text: "open because", warn: false, fix: null });
    expect(agentStatus(agent({ found: true, connected: true, access: "file-tools", fix: "Run /sandbox." })).data).toEqual({ text: "file-tools because", warn: true, fix: "Run /sandbox." });
    expect(agentStatus(agent({ found: true, connected: true, problem: "settings.json is not valid JSON" })).data).toBeUndefined();
    expect(agentStatus(agent({ found: true })).data).toBeUndefined();
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

describe("the version in About Rerail and the top bar", () => {
  it("links a version to its release page on GitHub", () => {
    expect(releaseUrl("0.2.0")).toBe("https://github.com/neralabs/rerail/releases/tag/v0.2.0");
  });

  it("orders versions by number, a prerelease before its release", () => {
    expect(isNewerVersion("0.2.0", "0.1.15")).toBe(true);
    expect(isNewerVersion("0.10.0", "0.9.9")).toBe(true);
    expect(isNewerVersion("0.2.0", "0.2.0")).toBe(false);
    expect(isNewerVersion("0.1.9", "0.2.0")).toBe(false);
    expect(isNewerVersion("0.2.0", "0.2.0-beta.1")).toBe(true);
    expect(isNewerVersion("0.2.0-beta.2", "0.2.0-beta.10")).toBe(false);
    expect(isNewerVersion("latest", "0.2.0")).toBe(false);
  });

  it("orders prereleases by semver precedence, the release candidates' -next.N included", () => {
    // semver 11.4's own example, each one after the one before.
    const chain = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0"];
    for (const [i, later] of chain.entries()) {
      if (i === 0) continue;
      expect(isNewerVersion(later, chain[i - 1]!)).toBe(true);
      expect(isNewerVersion(chain[i - 1]!, later)).toBe(false);
    }
    expect(isNewerVersion("0.3.0-next.10", "0.3.0-next.9")).toBe(true);
    // ASCII order, not the browser's locale: uppercase sorts before lowercase.
    expect(isNewerVersion("1.0.0-alpha", "1.0.0-Beta")).toBe(true);
  });

  it("names a newer version only when the daemon reports one above the version running", () => {
    expect(newerRelease({ version: "0.2.0", update: { latest: "0.3.0", checkedAt: 1 } })).toBe("0.3.0");
    expect(newerRelease({ version: "0.2.0", update: { latest: "0.2.0", checkedAt: 1 } })).toBeNull();
    expect(newerRelease({ version: "0.3.0", update: { latest: "0.2.0", checkedAt: 1 } })).toBeNull();
    // A daemon that doesn't check, or hasn't had an answer yet, says nothing; a page that has not heard from it has no health.
    expect(newerRelease({ version: "0.2.0" })).toBeNull();
    expect(newerRelease({ version: "0.2.0", update: null })).toBeNull();
    expect(newerRelease(null)).toBeNull();
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
    expect(counts).toEqual({ total: 2, on: 1 });
    expect(repoTag(counts)).toBe("1 rule on");
  });

  it("does not count a rule whose options do not apply as on", () => {
    expect(ruleCountsOf([entry("a"), entry("b", { configError: "bad option" })])).toEqual({ total: 2, on: 1 });
  });

  it("is unknown until the counts are read", () => {
    expect(repoTag(null)).toBeNull();
  });
});

describe("a repo's line in the repo choices", () => {
  const repo = (over: Partial<{ setUp: boolean; started: boolean; checkoutCount: number }> = {}) =>
    ({ setUp: true, started: true, checkoutCount: 1, ...over });

  it("says set up, then the rules on, then the checkouts when there are several", () => {
    expect(repoLine(repo(), { total: 12, on: 12 })).toBe("Set up · 12 rules on");
    expect(repoLine(repo({ checkoutCount: 2 }), { total: 50, on: 50 })).toBe("Set up · 50 rules on · 2 checkouts");
    expect(repoLine(repo(), { total: 0, on: 0 })).toBe("Set up · No rules added");
  });

  it("says a repo that is not set up runs nothing, whatever it holds", () => {
    expect(repoLine(repo({ setUp: false, started: true }), null)).toBe("Not set up · no rules run");
    expect(repoLine(repo({ setUp: false, checkoutCount: 3 }), { total: 2, on: 2 })).toBe("Not set up · no rules run · 3 checkouts");
  });

  it("says Off with the rules it holds while a repo is off, and only On or Off until the counts are read", () => {
    expect(repoLine(repo({ started: false }), { total: 12, on: 0 })).toBe("Set up · Off · 12 rules");
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

  const rules = (...entries: object[]) => ({ repositoryKey: "k", checkoutPath: null, setUp: true, rules: entries, unavailable: [] });

  it("reads each repo's own count, a set-up repo with no rules included", async () => {
    answers.set("store", { status: 200, etag: "\"s1\"", body: rules(entry("a"), entry("b", { on: false })) });
    answers.set("billing", { status: 200, etag: "\"b1\"", body: rules() });

    expect(await loadRepoRuleCounts(["store", "billing", "docs"])).toBe(true);

    expect(repoRuleCounts("store")).toEqual({ total: 2, on: 1 });
    expect(repoRuleCounts("billing")).toEqual({ total: 0, on: 0 });
    expect(repoRuleCounts("docs")).toBeNull();
  });

  it("asks again with the tag it holds and keeps the counts on 304", async () => {
    answers.set("store", { status: 200, etag: "\"s1\"", body: rules(entry("a")) });
    await loadRepoRuleCounts(["store"]);
    answers.set("store", { status: 304, etag: "\"s1\"" });

    expect(await loadRepoRuleCounts(["store"])).toBe(false);

    expect(requests.at(-1)).toEqual({ key: "store", ifNoneMatch: "\"s1\"" });
    expect(repoRuleCounts("store")).toEqual({ total: 1, on: 1 });
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

describe("a Settings switch after its write", () => {
  const NOT_ON_DISK = "the change was made, but Rerail could not confirm it is on the disk (EIO)";
  let answer: { status: number; body: unknown };
  /** The writes sent. The page's own reads, which `changeScope` starts, are not under test: they stay unanswered. */
  let writes: unknown[];

  beforeEach(() => {
    writes = [];
    toast.mockReset();
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", async (_input: URL, init: { method?: string; body?: string } = {}) => {
      if (init.method !== "PUT") return await new Promise<Response>(() => {});
      writes.push(JSON.parse(init.body ?? "null"));
      return new Response(JSON.stringify(answer.body), { status: answer.status });
    });
    changeScope({ repository: "A", checkout: "/work/a" });
    data.presence = "compact";
    data.followGitignore = true;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const switches = [
    { name: "the footer", act: () => settingsActions.presence("silent"), shown: () => data.presence, was: "compact", now: "silent" },
    { name: "Follow .gitignore", act: () => settingsActions.gitignore("off"), shown: () => data.followGitignore, was: true, now: false },
  ];

  it.each(switches)("$name keeps its new value when the daemon made the change but could not confirm it on the disk", async ({ act, shown, now }) => {
    answer = { status: 503, body: { error: "not-on-disk", message: NOT_ON_DISK } };
    await act();

    expect(writes).toHaveLength(1);
    expect(shown()).toBe(now);
    expect(toast).toHaveBeenCalledExactlyOnceWith("The change was made, but Rerail could not confirm it is on the disk (EIO).");
  });

  it.each(switches)("$name goes back to its old value when the write fails", async ({ act, shown, was }) => {
    answer = { status: 403, body: { error: "agent-not-permitted", message: "not yours" } };
    await act();

    expect(shown()).toBe(was);
    expect(toast).toHaveBeenCalledExactlyOnceWith(expect.stringMatching(/^Couldn’t change .+ answered 403/u));
  });
});
