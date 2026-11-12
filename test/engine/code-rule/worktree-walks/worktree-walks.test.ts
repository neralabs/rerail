import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { lstat, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";
import type * as Git from "../../../../src/engine/git.js";

/**
 * How many times each file was read whole through `node:fs/promises`, by the path it was read at; the listings held back
 * until a test lets them go, by folder; and how many times Git listed each checkout's changed files.
 */
const counted = vi.hoisted(() => ({
  reads: new Map<string, number>(),
  held: new Map<string, () => Promise<void>>(),
  listings: new Map<string, number>(),
}));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    readFile: async (...args: Parameters<typeof actual.readFile>) => {
      const path = String(args[0]);
      counted.reads.set(path, (counted.reads.get(path) ?? 0) + 1);
      return await actual.readFile(...args);
    },
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      const hold = counted.held.get(String(args[0]));
      counted.held.delete(String(args[0]));
      await hold?.();
      return await actual.opendir(...args);
    },
  };
});
vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof Git>();
  return {
    ...actual,
    gitModifiedOrUntrackedPaths: async (cwd: string) => {
      counted.listings.set(cwd, (counted.listings.get(cwd) ?? 0) + 1);
      return await actual.gitModifiedOrUntrackedPaths(cwd);
    },
  };
});
const { reads, listings } = counted;

// Each test opens several sessions and walks four or five checkouts many times over.
vi.setConfig({ testTimeout: 60_000 });

const bridges: CodeRuleBridge[] = [];
const directories: string[] = [];
const releases: Array<() => void> = [];
afterEach(async () => {
  releases.splice(0).forEach(release => release());
  counted.held.clear();
  await Promise.all(bridges.splice(0).map(bridge => bridge.close()));
  reads.clear();
  listings.clear();
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** Waits until the filesystem clock has passed the stamp of `path`, so a later write is told from it. */
async function awaitClockAfter(path: string): Promise<void> {
  const info = await lstat(path);
  const since = Math.max(info.mtimeMs, info.ctimeMs);
  const probe = await mkdtemp(join(tmpdir(), "rerail-walks-clock-"));
  directories.push(probe);
  for (;;) {
    await writeFile(join(probe, "tick"), "");
    const tick = await lstat(join(probe, "tick"));
    if (Math.max(tick.mtimeMs, tick.ctimeMs) > since) return;
  }
}

/** A repository at `main` holding `src/a.py`, with worktrees named `names` beside it; physical paths, a list a test may add to. */
async function repository(names = ["w1", "w2", "w3"]): Promise<{ main: string; worktrees: string[]; checkouts: string[] }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walks-")));
  directories.push(root);
  const main = join(root, "main");
  await mkdir(join(main, "src"), { recursive: true });
  await writeFile(join(main, "src", "a.py"), "old\n");
  git(main, "init", "--quiet");
  // The worktrees hold the blob's bytes as they are, whatever line breaks the host's Git writes.
  git(main, "config", "core.autocrlf", "false");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const worktrees = names.map(name => join(root, name));
  for (const worktree of worktrees) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  await awaitClockAfter(join(worktrees.at(-1) ?? main, "src", "a.py"));
  return { main, worktrees, checkouts: [main, ...worktrees] };
}

/** The innermost of `checkouts` that holds `absolute`, as the router names a file's checkout. */
function innermost(checkouts: readonly string[], absolute: string): string | undefined {
  return checkouts.filter(checkout => absolute === checkout || absolute.startsWith(`${checkout}${sep}`)).sort((a, b) => b.length - a.length)[0];
}

type Session = Awaited<ReturnType<typeof opened>>;

/** The file extensions the rules read, as a session's router holds them once it refreshes its rules. */
interface Rules {
  readonly extensions: Set<string>;
}

/**
 * A session opened in `folder`, watching the other `checkouts` (read as they are at each walk) through `walks`. With
 * `rules`, it reads only files of their extensions, as they were at its last refresh of its rules; else every file.
 */
async function opened(folder: string, checkouts: readonly string[], walks: WorktreeWalks, rules?: Rules) {
  const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
    revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
  }));
  let known = new Set(rules?.extensions);
  const auditor: CodeRuleAuditor = {
    openWorkspace: async () => undefined,
    acceptsPath: path => rules === undefined || known.has(extname(path)),
    refreshCodeRules: () => {
      known = new Set(rules?.extensions);
    },
    auditGeneratedEdits,
    close: async () => undefined,
    sourceRoot: path => innermost(checkouts, resolve(folder, path)),
    // As the router does: the session's own checkout is never one of the other worktrees.
    worktreeRoots: () => checkouts.filter(checkout => checkout !== innermost(checkouts, folder)),
  };
  const bridge = new CodeRuleBridge(folder, auditor, { worktreeWalks: walks });
  bridges.push(bridge);
  await bridge.openWorkspace(`session-${bridges.length}`);
  return { bridge, auditGeneratedEdits };
}

/** A session whose first turn, and the walk of the other worktrees after it, are done. */
async function session(folder: string, checkouts: readonly string[], walks: WorktreeWalks, rules?: Rules): Promise<Session> {
  const opening = await opened(folder, checkouts, walks, rules);
  await turn(opening);
  return opening;
}

/** Whether `settling` resolves within `ms`. */
async function within(settling: Promise<void>, ms: number): Promise<boolean> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<boolean>(wake => {
    timer = setTimeout(() => wake(false), ms);
  });
  try {
    return await Promise.race([settling.then(() => true), late]);
  } finally {
    clearTimeout(timer);
  }
}

async function turn(...sessions: Session[]): Promise<void> {
  for (const { bridge } of sessions) {
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
  }
}

/** Every edit a session's auditor was given, with its mode, in order. */
function audited({ auditGeneratedEdits }: Session): Array<[string[], string | undefined]> {
  return auditGeneratedEdits.mock.calls.map(([, batch]) => [
    batch.edits.map(edit => `${edit.deleted === true ? "deleted" : "written"} ${edit.path}`),
    batch.mode,
  ]);
}

async function shell(bridge: CodeRuleBridge, id: string, run: () => unknown): Promise<void> {
  await bridge.begin(id, "shell", { command: "./run.sh" });
  await run();
  await bridge.finish(id);
}

/** Holds the next listing of `folder` until `release` is called; `entered` resolves as the listing starts. */
function holdListing(folder: string): { entered: Promise<void>; release: () => void } {
  const entered = Promise.withResolvers<void>();
  const released = Promise.withResolvers<void>();
  releases.push(released.resolve);
  counted.held.set(folder, async () => {
    entered.resolve();
    await released.promise;
  });
  return { entered: entered.promise, release: released.resolve };
}

describe("WorktreeWalks", () => {
  it("holds one observation of each file of the worktrees its sessions watch, however many sessions watch them", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    await session(worktrees[0]!, checkouts, walks);
    expect(walks.observations()).toBe(3);
    await session(worktrees[1]!, checkouts, walks);
    // Two sessions watch `main` and `w3`: each of their files is held once, and read once.
    expect(walks.observations()).toBe(4);
    expect(reads.get(join(worktrees[2]!, "src", "a.py"))).toBe(1);
  });

  it("hands a shell write in a worktree to every session watching it, read once, and recorded by each in Log", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    const path = join(worktrees[2]!, "src", "a.py");
    await first.bridge.begin("shell-1", "shell", { command: "./write.sh" });
    await writeFile(path, "old\nmine\n");
    await first.bridge.finish("shell-1");
    await first.bridge.settleWorktreeWalk();
    await second.bridge.settleWorktreeWalk();
    for (const { auditGeneratedEdits } of [first, second]) {
      expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
        [[{ path: "../w3/src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }], "log"],
      ]);
    }
    expect(reads.get(path)).toBe(2);
  });

  it("reads a worktree added while two sessions in the main checkout are open once for both, and each still records a shell write there in Log", async () => {
    const { main, checkouts } = await repository([]);
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(main, checkouts, walks);
    const second = await session(main, checkouts, walks);
    const worktree = join(dirname(main), "w");
    git(main, "worktree", "add", "--quiet", "-b", "w", worktree);
    checkouts.push(worktree);
    const path = join(worktree, "src", "a.py");
    await awaitClockAfter(path);
    // Each session runs a command, so each has read the new worktree.
    await shell(first.bridge, "shell-1", () => undefined);
    await turn(first);
    await shell(second.bridge, "shell-2", () => undefined);
    await turn(second);
    // The first appends to a file there from the shell; the other runs a command that writes nothing.
    await shell(first.bridge, "shell-3", () => writeFile(path, "old\nmine\n"));
    await turn(first);
    await shell(second.bridge, "shell-4", () => undefined);
    await turn(second, first);
    // One walk reads the file for both sessions, as the worktree first and after the write; each session still files
    // its own record of the write, where one record of it would do.
    expect(reads.get(path)).toBe(2);
    expect([audited(first), audited(second)]).toEqual([[[["written ../w/src/a.py"], "log"]], [[["written ../w/src/a.py"], "log"]]]);
  });

  it("reads through another session once the one it read through closes, and drops what no session watches", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    await first.bridge.close();
    const path = join(worktrees[2]!, "src", "a.py");
    await writeFile(path, "old\nmine\n");
    await second.bridge.checkpoint("turn");
    await second.bridge.settleWorktreeWalk();
    expect(second.auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual(["../w3/src/a.py"]);
    await second.bridge.close();
    expect(walks.observations()).toBe(0);
  });

  it("keeps no snapshot from a walk whose sessions all closed while it read", async () => {
    const { checkouts, worktrees } = await repository();
    await mkdir(join(worktrees[2]!, "deep"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    const hold = holdListing(join(worktrees[2]!, "deep"));
    await first.bridge.checkpoint("turn");
    await hold.entered;
    await first.bridge.close();
    await second.bridge.close();
    hold.release();
    await walks.idle();
    // The walk had read `main` and `w2` when the sessions closed; with no session left to walk again, nothing would drop them.
    expect(walks.observations()).toBe(0);
  });

  it("lists a worktree's changed files from Git once for all the sessions that first take it in one walk", async () => {
    const { checkouts, worktrees } = await repository(["w1", "w2", "w3", "w4"]);
    await mkdir(join(worktrees[3]!, "deep"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await opened(worktrees[0]!, checkouts, walks);
    const hold = holdListing(join(worktrees[3]!, "deep"));
    await first.bridge.checkpoint("turn");
    await hold.entered;
    // Both ask for a walk while the first session's is held, so one walk shows them the other worktrees first.
    const second = await opened(worktrees[1]!, checkouts, walks);
    await second.bridge.checkpoint("turn");
    const third = await opened(worktrees[2]!, checkouts, walks);
    await third.bridge.checkpoint("turn");
    hold.release();
    await turn(first, second, third);
    // `w4` is new to each session: listed once for the first session's walk and once for the walk the other two share.
    expect(listings.get(worktrees[3]!)).toBe(2);
  });

  it("walks at once for a session whose own last walk is past the gap, whatever walk another session just had", async () => {
    const { checkouts, worktrees } = await repository();
    // A gap no test waits out: it spaces the walks each session asks for, not the walks of the repository.
    const walks = new WorktreeWalks({ gapMs: 60_000 });
    const first = await opened(worktrees[0]!, checkouts, walks);
    await first.bridge.checkpoint("turn");
    await vi.waitFor(() => expect(walks.observations()).toBe(3), { timeout: 10_000, interval: 20 });
    // The first session's walk has just ended. The second asks for its first walk as its folder's walk ends.
    const second = await opened(worktrees[1]!, checkouts, walks);
    await second.bridge.checkpoint("turn");
    const firstWalk = await within(second.bridge.settleWorktreeWalk(), 10_000);
    // Its own gap holds: a walk it asks for again at once waits for it.
    await shell(second.bridge, "shell-1", () => writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n"));
    const again = await within(second.bridge.settleWorktreeWalk(), 2_000);
    expect({ firstWalk, again }).toEqual({ firstWalk: true, again: false });
  });

  it("reads a worktree by the rules its sessions hold now, whichever session it reads through", async () => {
    const { checkouts, worktrees } = await repository();
    const rules: Rules = { extensions: new Set([".py"]) };
    const walks = new WorktreeWalks({ gapMs: 0 });
    // The first session, which every walk reads through, has no event once a rule for `.ts` files is added.
    await session(worktrees[0]!, checkouts, walks, rules);
    const other = await session(worktrees[1]!, checkouts, walks, rules);
    rules.extensions.add(".ts");
    await shell(other.bridge, "shell-1", () => writeFile(join(worktrees[2]!, "src", "b.ts"), "export const b = 1;\n"));
    await other.bridge.settleWorktreeWalk();
    expect(audited(other)).toEqual([[["written ../w3/src/b.ts"], "log"]]);
  });
});

describe("the other worktrees of a session in a nested worktree", () => {
  it("records no change of its checkout's other files when a session in another worktree starts or closes", async () => {
    const { main, worktrees, checkouts } = await repository(["w1"]);
    await mkdir(join(main, "lib"), { recursive: true });
    await mkdir(join(main, "pkg"), { recursive: true });
    await writeFile(join(main, "lib", "c.py"), "c = 1\n");
    await writeFile(join(main, "pkg", "b.py"), "b = 1\n");
    // The nested worktree is untracked in `main`, as a coding agent's worktrees in `.claude/worktrees` are.
    await writeFile(join(main, ".gitignore"), "nested/\n");
    git(main, "add", ".");
    git(main, "commit", "--quiet", "-m", "more");
    const nested = join(main, "nested");
    git(main, "worktree", "add", "--quiet", "--detach", nested);
    checkouts.push(nested);
    await awaitClockAfter(join(nested, "lib", "c.py"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    // Opened in `nested/pkg`: its checkout is `nested`, which it does not watch, and `main` holds it.
    const inner = await session(join(nested, "pkg"), checkouts, walks);
    const steps: Array<[string, unknown]> = [];
    const record = (step: string): void => {
      steps.push([step, audited(inner)]);
      inner.auditGeneratedEdits.mockClear();
    };
    const other = await session(worktrees[0]!, checkouts, walks);
    await turn(inner, other, inner);
    record("another session started");
    await other.bridge.close();
    await turn(inner, inner);
    record("it closed");
    const again = await session(worktrees[0]!, checkouts, walks);
    await turn(inner, again, inner);
    record("another started again");
    await shell(inner.bridge, "shell-1", () => writeFile(join(main, "lib", "c.py"), "c = 1\nmain\n"));
    await turn(inner, inner);
    record("it wrote in main");
    expect(steps).toEqual([
      ["another session started", []],
      ["it closed", []],
      ["another started again", []],
      ["it wrote in main", [[["written ../../lib/c.py"], "log"]]],
    ]);
  });

  it("records its own edit of a file in the folder that holds its worktree once, as its tool call made it", async () => {
    const { main, checkouts } = await repository([]);
    await mkdir(join(main, ".agent"));
    await writeFile(join(main, ".agent", "notes.py"), "n = 1\n");
    git(main, "add", ".");
    git(main, "commit", "--quiet", "-m", "notes");
    const nested = join(main, ".agent", "worktrees", "a");
    git(main, "worktree", "add", "--quiet", "--detach", nested);
    checkouts.push(nested);
    await awaitClockAfter(join(nested, "src", "a.py"));
    const inA = await session(nested, checkouts, new WorktreeWalks({ gapMs: 0 }));
    const path = join(main, ".agent", "notes.py");
    await inA.bridge.begin("write-1", "write", { path });
    await writeFile(path, "n = 1\nmine\n");
    await inA.bridge.finish("write-1");
    await turn(inA, inA);
    // Its walk of `main` named the file `../../../.agent/notes.py`, and recorded the edit again under that name.
    expect(audited(inA)).toEqual([[["written ../../notes.py"], undefined]]);
  });
});
