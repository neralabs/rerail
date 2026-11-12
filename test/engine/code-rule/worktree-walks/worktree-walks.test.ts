import { execFileSync } from "node:child_process";
import { watch as watchPath } from "node:fs";
import type * as FileSystem from "node:fs/promises";
import { lstat, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { WatchedWorktrees, type WatchPath } from "../../../../src/engine/code-rule/worktree-notices.js";
import { WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";
import type * as Git from "../../../../src/engine/git.js";

/**
 * How many times each file was read whole through `node:fs/promises`, by the path it was read at; each path `lstat` was
 * asked about; the listings held back until a test lets them go, by folder; and how many times Git listed each
 * checkout's changed files.
 */
const counted = vi.hoisted(() => ({
  reads: new Map<string, number>(),
  stats: [] as string[],
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
    lstat: async (...args: Parameters<typeof actual.lstat>) => {
      counted.stats.push(String(args[0]));
      return await actual.lstat(...args);
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
const { reads, stats, listings } = counted;

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
  stats.length = 0;
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

/**
 * A session opened in `folder`, watching the other `checkouts` (read as they are at each walk) through `walks`; Git
 * ignores each path `ignores` names, as the session names it.
 */
async function opened(folder: string, checkouts: readonly string[], walks: WorktreeWalks, ignores?: (path: string) => boolean) {
  const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
    revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
  }));
  const auditor: CodeRuleAuditor = {
    openWorkspace: async () => undefined,
    acceptsPath: () => true,
    auditGeneratedEdits,
    close: async () => undefined,
    sourceRoot: path => innermost(checkouts, resolve(folder, path)),
    // As the router does: the session's own checkout is never one of the other worktrees.
    worktreeRoots: () => checkouts.filter(checkout => checkout !== innermost(checkouts, folder)),
    ...(ignores === undefined ? {} : { ignoresPath: ignores }),
  };
  const bridge = new CodeRuleBridge(folder, auditor, { worktreeWalks: walks });
  bridges.push(bridge);
  await bridge.openWorkspace(`session-${bridges.length}`);
  return { bridge, auditGeneratedEdits };
}

/** A session whose first turn, and the walk of the other worktrees after it, are done. */
async function session(folder: string, checkouts: readonly string[], walks: WorktreeWalks, ignores?: (path: string) => boolean): Promise<Session> {
  const opening = await opened(folder, checkouts, walks, ignores);
  await turn(opening);
  return opening;
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

  it("reads another worktree once for two sessions in the main checkout, and each still records a shell write there in Log", async () => {
    const { main, checkouts, worktrees } = await repository(["w"]);
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(main, checkouts, walks);
    const second = await session(main, checkouts, walks);
    const path = join(worktrees[0]!, "src", "a.py");
    await shell(first.bridge, "shell-1", () => writeFile(path, "old\nmine\n"));
    await turn(first);
    // The other session runs a command that writes nothing.
    await shell(second.bridge, "shell-2", () => undefined);
    await turn(second, first);
    // Both sessions' walks find the write in one read. Each session still files its own record of it.
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
    // A write in each makes the next walk read them, `w3` after `main` and `w2`.
    for (const checkout of [checkouts[0]!, worktrees[1]!, worktrees[2]!]) await writeFile(join(checkout, "src", "a.py"), "old\nmine\n");
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
});

/** How many times `lstat` was asked about each of `roots` or anything in it. */
function statsUnder(...roots: string[]): number[] {
  return roots.map(root => stats.filter(path => path === root || path.startsWith(`${root}${sep}`)).length);
}

/** Every batch of edits a session's auditor was given, whole, with its mode. */
function edits({ auditGeneratedEdits }: Session): unknown[] {
  return auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode]);
}

/**
 * Two sessions in `folder`: one whose walks read only the worktrees a change notice named, and one whose walks read every
 * worktree, as all walks did before notices. Each has had two walks, so the first one's worktrees are watched whole.
 */
async function bothWays(folder: string, checkouts: readonly string[]): Promise<{ noticed: Session; everyWorktree: Session }> {
  const noticed = await session(folder, checkouts, new WorktreeWalks({ gapMs: 0 }));
  const everyWorktree = await session(folder, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }));
  await turn(noticed, everyWorktree);
  return { noticed, everyWorktree };
}

/** `fs.watch` for the walks, keeping the name of each notice it passes on. */
function namedWatches(): { named: Array<string | null>; watch: WatchPath } {
  const named: Array<string | null> = [];
  const watch: WatchPath = (path, { recursive }, listener) => watchPath(path, { recursive, persistent: false }, (event, name) => {
    named.push(name);
    listener(event, name);
  });
  return { named, watch };
}

/** Resolves once a notice naming a path that ends in `suffix` has reached the walks. */
async function noticed(named: ReadonlyArray<string | null>, suffix: string): Promise<void> {
  while (!named.some(name => name?.endsWith(suffix) === true)) await new Promise(resolve => { setTimeout(resolve, 10); });
}

describe("a walk of the other worktrees with change notices", () => {
  it("stats no entry of the other worktrees when nothing changed in them since the last walk", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0 }));
    // On Linux the folders of a worktree are watched once its first read is over, so the next walk reads it again.
    await turn(watching);
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 0]);
  });

  it("stats no entry when a file Git ignores changes after a walk that read no worktree", async () => {
    const { main, worktrees, checkouts } = await repository();
    const cache = join(worktrees[2]!, "src", "cache.tmp");
    await writeFile(cache, "x\n");
    const { named, watch } = namedWatches();
    const walks = new WorktreeWalks({ gapMs: 0, notices: new WatchedWorktrees({ watch }) });
    const watching = await session(worktrees[0]!, checkouts, walks, path => path.endsWith(".tmp"));
    // The second walk reads again on Linux, where folders are watched once a read is over; the third reads none.
    await turn(watching);
    await turn(watching);
    named.length = 0;
    await writeFile(cache, "y\n");
    await noticed(named, "cache.tmp");
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 0]);
  });

  it("records a file Git ignored, once the index tracks it, and a shell write to it, as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    const generated = join(w3, "src", "gen.py");
    await writeFile(generated, "x = 1\n");
    await awaitClockAfter(generated);
    // As Git answers: a path the index tracks is never ignored.
    let tracked = false;
    const ignores = (path: string): boolean => path.endsWith("gen.py") && !tracked;
    const noticed = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0 }), ignores);
    const everyWorktree = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }), ignores);
    // The second walk reads again on Linux, where folders are watched once a read is over; the third reads none.
    await turn(noticed, everyWorktree);
    await turn(noticed, everyWorktree);
    git(w3, "add", "--force", "src/gen.py");
    tracked = true;
    // Nothing in `w3` changed: only its index tells that the file counts now.
    await turn(noticed, everyWorktree);
    await writeFile(generated, "x = 2\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed).at(-1)).toEqual([["written ../w3/src/gen.py"], "log"]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("stats only the worktree a file changed in, and each session watching it records the change", async () => {
    const { main, worktrees, checkouts } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    await turn(first, second);
    stats.length = 0;
    await writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n");
    await turn(first, second);
    // `w3`, its `src` and `src/a.py`, once for both sessions.
    expect(statsUnder(main, worktrees[2]!)).toEqual([0, 3]);
    expect([audited(first), audited(second)]).toEqual([[[["written ../w3/src/a.py"], "log"]], [[["written ../w3/src/a.py"], "log"]]]);
  });

  it("reads a worktree whose HEAD moved though no file in it changed", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0 }));
    await turn(watching);
    stats.length = 0;
    git(worktrees[2]!, "commit", "--quiet", "--allow-empty", "-m", "empty");
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 3]);
  });

  it("records a file created in another worktree as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const { noticed, everyWorktree } = await bothWays(worktrees[0]!, checkouts);
    await writeFile(join(worktrees[2]!, "src", "new.py"), "n = 1\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed)).toEqual([[["written ../w3/src/new.py"], "log"]]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("records a file removed from another worktree as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const { noticed, everyWorktree } = await bothWays(worktrees[0]!, checkouts);
    await rm(join(worktrees[2]!, "src", "a.py"));
    await turn(noticed, everyWorktree);
    expect(audited(noticed)).toEqual([[["deleted ../w3/src/a.py"], "log"]]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("follows a branch switch in another worktree as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    git(w3, "checkout", "--quiet", "-b", "other");
    await writeFile(join(w3, "src", "a.py"), "other\n");
    git(w3, "commit", "--quiet", "-am", "other");
    git(w3, "checkout", "--quiet", "--detach", "HEAD~1");
    await awaitClockAfter(join(w3, "src", "a.py"));
    const { noticed, everyWorktree } = await bothWays(worktrees[0]!, checkouts);
    git(w3, "checkout", "--quiet", "other");
    await turn(noticed, everyWorktree);
    await awaitClockAfter(join(w3, "src", "a.py"));
    await writeFile(join(w3, "src", "a.py"), "other\nmine\n");
    await turn(noticed, everyWorktree);
    // The write after the switch is measured against the branch's text.
    expect(edits(noticed).at(-1)).toEqual([[{ path: "../w3/src/a.py", language: "python", previousText: "other\n", generatedText: "other\nmine\n" }], "log"]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("takes a worktree added since, and records a write in it, as walks of every worktree do", async () => {
    const { main, worktrees, checkouts } = await repository();
    const { noticed, everyWorktree } = await bothWays(worktrees[0]!, checkouts);
    const added = join(main, "..", "w4");
    git(main, "worktree", "add", "--quiet", "--detach", added);
    checkouts.push(added);
    await turn(noticed, everyWorktree);
    await awaitClockAfter(join(added, "src", "a.py"));
    await writeFile(join(added, "src", "a.py"), "old\nmine\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed).at(-1)).toEqual([["written ../w4/src/a.py"], "log"]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("drops a worktree removed since without recording its files, as walks of every worktree do", async () => {
    const { main, worktrees, checkouts } = await repository();
    const { noticed, everyWorktree } = await bothWays(worktrees[0]!, checkouts);
    // Removed while the walks watch it: on Windows a watch holds the folder until it closes.
    git(main, "worktree", "remove", "--force", worktrees[2]!);
    checkouts.splice(checkouts.indexOf(worktrees[2]!), 1);
    await turn(noticed, everyWorktree);
    await writeFile(join(worktrees[1]!, "src", "a.py"), "old\nmine\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed)).toEqual([[["written ../w2/src/a.py"], "log"]]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("finds a change made while a read ran, after the read passed the file and before its folders were watched", async () => {
    const { worktrees, checkouts } = await repository();
    const most = join(worktrees[2]!, "deep", "inner", "most");
    await mkdir(most, { recursive: true });
    const watching = await opened(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0 }));
    const hold = holdListing(most);
    await watching.bridge.checkpoint("turn");
    // `w3` is read last; its listing of `most` waits until the read has observed `src/a.py`.
    await hold.entered;
    await writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n");
    hold.release();
    await watching.bridge.settleWorktreeWalk();
    await turn(watching);
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
  });

  it("reads on every walk a worktree it cannot watch, and a change there reaches the session", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch: WatchPath = () => {
      throw Object.assign(new Error("too many open files"), { code: "EMFILE" });
    };
    const walks = new WorktreeWalks({ gapMs: 0, notices: new WatchedWorktrees({ watch }) });
    const watching = await session(worktrees[0]!, checkouts, walks);
    await turn(watching);
    stats.length = 0;
    await writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n");
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([3, 3, 3]);
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
  });

  it("reads every worktree again once the full walk is due", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }));
    await turn(watching);
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([3, 3, 3]);
  });

  // FSEvents coalesces notices rather than dropping them, so macOS has no overflow to provoke.
  it.skipIf(process.platform === "darwin")("reads a worktree again when the system dropped its notices", async () => {
    const { worktrees, checkouts } = await repository();
    const src = join(worktrees[2]!, "src");
    // More notices than the system keeps: inotify queues max_queued_events (16,384 by default), ReadDirectoryChangesW
    // fills a 64 KB buffer. In files Git ignores, whose own notices never count.
    const files = process.platform === "linux" ? 20_000 : 1_500;
    /** Writes `text` to every ignored file, then `last` to `a.py`, in another process: the loop waits meanwhile. */
    const flood = (text: string, last?: string): void => {
      const script = [
        `const fs = require("node:fs");`,
        `for (let i = 0; i < ${files}; i += 1) fs.writeFileSync(${JSON.stringify(src)} + "/flood-" + i + ".tmp", ${JSON.stringify(text)});`,
        last === undefined ? "" : `fs.writeFileSync(${JSON.stringify(join(src, "a.py"))}, ${JSON.stringify(last)});`,
      ];
      execFileSync(process.execPath, ["-e", script.join("\n")]);
    };
    flood("x");
    const { named, watch } = namedWatches();
    const walks = new WorktreeWalks({ gapMs: 0, notices: new WatchedWorktrees({ watch }) });
    const watching = await session(worktrees[0]!, checkouts, walks, path => path.endsWith(".tmp"));
    await turn(watching);
    await awaitClockAfter(join(src, "a.py"));
    named.length = 0;
    flood("y", "old\nmine\n");
    await new Promise(resolve => { setTimeout(resolve, 500); });
    // The system dropped the notice of `a.py`, the last write.
    expect(named.some(name => name?.endsWith("a.py") === true)).toBe(false);
    await turn(watching);
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
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
