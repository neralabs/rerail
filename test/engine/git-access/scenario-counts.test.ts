import { execFileSync } from "node:child_process";
import { mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { checkoutGit, type GitAccessCounts, type GitQuestion, type QuestionStarts } from "../../../src/engine/git-access/checkout-git.js";
import { RerailRuntime } from "../../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

// One scenario, pinned: the Git processes Rerail starts in it, by question. A change that asks Git less lowers a number
// here; a change that should ask the same leaves every number as it is.

/** Every bridge the scenario's runtimes open, so the scenario can wait for the walks no boundary waits for. */
const bridges = vi.hoisted(() => new Set<{ settleWorktreeWalk(): Promise<void> }>());

vi.mock("../../../src/engine/code-rule/code-rule-bridge.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/code-rule/code-rule-bridge.js")>();
  /**
   * A bridge that walks the other worktrees with no gap between two walks. The daemon waits 5 s between them, so on a
   * slow machine more of them fall inside the scenario; here each boundary is followed by one walk, waited for.
   */
  class SettledBridge extends actual.CodeRuleBridge {
    constructor(...[cwd, auditor, options]: ConstructorParameters<typeof actual.CodeRuleBridge>) {
      super(cwd, auditor, { ...options, worktreeWalkGapMs: 0 });
      bridges.add(this);
    }
  }
  return { ...actual, CodeRuleBridge: SettledBridge };
});

const temporaryDirectory = temporaryDirectories("rerail-git-scenario-");

afterEach(() => {
  vi.unstubAllEnvs();
  bridges.clear();
});

/** The environment of the test's own Git commands, which Git's trace must not count as Rerail's. */
function untraced(): NodeJS.ProcessEnv {
  return { ...process.env, GIT_TRACE2_EVENT: undefined };
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore", env: untraced() });
}

/** Waits for each walk of the other worktrees a boundary started, which runs behind the boundary and is never waited for. */
async function settled(): Promise<void> {
  for (const bridge of bridges) await bridge.settleWorktreeWalk();
}

/** A steer rule that flags `NEEDLE` in any Python file. */
const NEEDLE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{
    id: "needle",
    violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
    forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
  }],
});

/** The Git processes each question started between `before` and `after`; a question that started none is left out. */
function startsBetween(before: GitAccessCounts, after: GitAccessCounts): Partial<Record<GitQuestion, QuestionStarts>> {
  const between: Partial<Record<GitQuestion, QuestionStarts>> = {};
  for (const [question, count] of Object.entries(after.questions) as Array<[GitQuestion, QuestionStarts]>) {
    const earlier = before.questions[question] ?? { starts: 0, blocking: 0 };
    if (count.starts > earlier.starts) between[question] = { starts: count.starts - earlier.starts, blocking: count.blocking - earlier.blocking };
  }
  return between;
}

/**
 * The Git processes Git's own trace saw Rerail start: each `start` event of a process no other Git started, whose
 * arguments carry the configuration every Git process Rerail starts is given.
 */
async function tracedRerailStarts(trace: string): Promise<number> {
  const events = (await readFile(trace, "utf8")).split(/\r?\n/u).filter(line => line !== "").map(line => JSON.parse(line) as {
    event: string; sid: string; argv?: string[];
  });
  return events.filter(event => event.event === "start" && !event.sid.includes("/") && event.argv?.includes("core.fsmonitor=false") === true).length;
}

/** A clone with no remote, its main checkout and two linked worktrees, with one rule started on its repository. */
async function clone(root: string, databasePath: string): Promise<{ main: string; one: string }> {
  const main = join(root, "main");
  await mkdir(main);
  await writeFile(join(main, "app.py"), "print('ok')\n");
  git(main, "init", "--quiet", "--initial-branch=main");
  // The same line endings on every system: Git for Windows sets core.autocrlf=true in its own configuration.
  git(main, "config", "core.autocrlf", "false");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  git(main, "worktree", "add", "--quiet", "-b", "one", join(root, "one"));
  git(main, "worktree", "add", "--quiet", "-b", "two", join(root, "two"));
  setUpTestRepository(main, { library: [{ kind: "code", rule: NEEDLE }] }, databasePath);
  return { main, one: join(root, "one") };
}

/**
 * The Git processes of each question in the scenario, as Rerail starts them today: 96, the same commands, argument for
 * argument, as before the Git access module, by Git's own trace.
 */
const PINNED: Partial<Record<GitQuestion, QuestionStarts>> = {
  "checkout-location": { starts: 24, blocking: 24 },
  "legacy-repository-id": { starts: 4, blocking: 4 },
  "origin-url": { starts: 4, blocking: 4 },
  "remote-names": { starts: 4, blocking: 4 },
  "submodule-paths": { starts: 8, blocking: 8 },
  "worktree-list": { starts: 1, blocking: 1 },
  head: { starts: 21, blocking: 0 },
  "index-file": { starts: 6, blocking: 0 },
  "unmerged-entries": { starts: 7, blocking: 0 },
  "modified-or-untracked": { starts: 6, blocking: 0 },
  "exclude-file": { starts: 2, blocking: 0 },
  "excludes-file-config": { starts: 2, blocking: 0 },
  "config-files": { starts: 2, blocking: 0 },
  blobs: { starts: 1, blocking: 0 },
  "blob-ids": { starts: 1, blocking: 0 },
  autocrlf: { starts: 1, blocking: 0 },
  "core-eol": { starts: 1, blocking: 0 },
  "line-ending-attributes": { starts: 1, blocking: 0 },
};

/** A Git process takes tens of milliseconds to start on Windows, and the scenario starts 96. */
const SCENARIO_TIMEOUT_MS = 60_000;

describe("Git starts by question", () => {
  it("in two runtimes, three worktrees of a clone with no remote, one session, a shell write and a commit", async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    // Git reads the test's home, so the developer's own configuration plays no part.
    vi.stubEnv("HOME", join(root, "home"));
    vi.stubEnv("XDG_CONFIG_HOME", "");
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(root, "home", "gitconfig"));
    const databasePath = join(root, "rerail.sqlite");
    const { main, one } = await clone(root, databasePath);
    const options = { storageRoot: join(root, "sessions"), databasePath, autostart: false, embedder: testEmbedder };
    // The daemon makes one runtime per connection; the session moves to the second once the first has closed it.
    const first = new RerailRuntime({ ...options, workspaceRoot: main });
    const second = new RerailRuntime({ ...options, workspaceRoot: one });
    const trace = join(root, "trace2.json");
    const before = checkoutGit.counts();
    vi.stubEnv("GIT_TRACE2_EVENT", trace);
    const session = { host: "codex" as const, sessionId: "pinned", cwd: main };
    const shell = async (runtime: RerailRuntime, id: string, act: () => Promise<void>) => {
      await runtime.ingest({ kind: "call", eventId: `${id}-call`, sessionId: session.sessionId, toolCallId: id, toolName: "bash", input: { command: `./${id}.sh` } });
      await settled();
      await act();
      const deliveries = await runtime.ingest({ kind: "result", eventId: `${id}-result`, sessionId: session.sessionId, toolCallId: id, toolName: "bash", output: "" });
      await settled();
      return deliveries;
    };
    const turnEnd = async (runtime: RerailRuntime, eventId: string) => {
      const deliveries = await runtime.ingest({ kind: "turn-end", eventId, sessionId: session.sessionId, text: "done" });
      await settled();
      return deliveries;
    };
    try {
      await first.ingest({ kind: "session-start", eventId: "start-1", session });
      await first.codeRulesOpened(session.sessionId);
      expect(await turnEnd(first, "baseline")).toEqual([]);
      const steered = await shell(first, "write", () => writeFile(join(main, "shell.py"), "NEEDLE\n"));
      expect(steered.map(delivery => delivery.content).join("\n")).toContain("shell.py:1:1");
      await turnEnd(first, "written");
      await shell(first, "commit", async () => {
        git(main, "add", "shell.py");
        git(main, "commit", "--quiet", "-m", "shell");
      });
      await turnEnd(first, "committed");
      await first.closeSession(session.sessionId);
      await second.ingest({ kind: "session-start", eventId: "start-2", session });
      await second.codeRulesOpened(session.sessionId);
      await turnEnd(second, "reopened");
    } finally {
      await first.dispose();
      await second.dispose();
    }
    const after = checkoutGit.counts();
    vi.stubEnv("GIT_TRACE2_EVENT", "");
    expect(startsBetween(before, after)).toEqual(PINNED);
    // Git's own trace sees no Rerail start that counts() missed.
    expect(await tracedRerailStarts(trace)).toBe(after.starts - before.starts);
  }, SCENARIO_TIMEOUT_MS);
});
