import { execFileSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import type * as FileSystem from "node:fs/promises";
import { cp, lstat, mkdir, mkdtemp, realpath, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { removeFolder, runningIn } from "../../../support/platform/remove-folder.js";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type * as WalkReads from "../../../../src/engine/code-rule/walk-read-buffers.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { lookMoved, WorktreeLooks } from "../../../../src/engine/code-rule/worktree-look.js";
import { WorktreeNotices } from "../../../../src/engine/code-rule/worktree-notices.js";
import { RepositoryWorktreeWalks, type WatcherWalk, WorktreeWalks, type WorktreeWatcher } from "../../../../src/engine/code-rule/worktree-walks.js";
import type { WorktreeListLearning } from "../../../../src/engine/catalog/repository-identity.js";
import { GitReads } from "../../../../src/engine/git.js";
import type * as Git from "../../../../src/engine/git.js";
import { StampTreeWatch } from "./support/stamp-tree-watch.js";

/**
 * How many times a walk read each file whole (`WalkReadBuffers.lend`), by the path it was read at; each path `lstat` was
 * asked about; the listings held back until a test lets them go, by folder; the openings held back the same way, by
 * file; how many times Git listed each checkout's changed files, and read blobs there; what runs just before and just
 * after the next such listing of a checkout, by checkout; and the files whose next opening fails as one another process
 * holds does.
 */
const counted = vi.hoisted(() => ({
  reads: new Map<string, number>(),
  stats: [] as string[],
  held: new Map<string, () => Promise<void>>(),
  heldOpenings: new Map<string, () => Promise<void>>(),
  listings: new Map<string, number>(),
  blobReads: new Map<string, number>(),
  aroundListing: new Map<string, { readonly before: () => void; readonly after: () => void }>(),
  busy: new Set<string>(),
  heads: 0,
}));
vi.mock("../../../../src/engine/code-rule/walk-read-buffers.js", async importOriginal => {
  const actual = await importOriginal<typeof WalkReads>();
  class CountedReadBuffers extends actual.WalkReadBuffers {
    override async lend<T>(absolute: string, size: number, use: (read: WalkReads.LentBytes) => Promise<T>): Promise<T> {
      counted.reads.set(absolute, (counted.reads.get(absolute) ?? 0) + 1);
      return await super.lend(absolute, size, use);
    }
  }
  return { ...actual, WalkReadBuffers: CountedReadBuffers };
});
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    open: async (...args: Parameters<typeof actual.open>) => {
      if (counted.busy.delete(String(args[0]))) throw Object.assign(new Error("EBUSY: resource busy or locked, open"), { code: "EBUSY" });
      const hold = counted.heldOpenings.get(String(args[0]));
      counted.heldOpenings.delete(String(args[0]));
      await hold?.();
      return await actual.open(...args);
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
    gitFirstSightListing: async (...args: Parameters<typeof actual.gitFirstSightListing>) => {
      counted.listings.set(args[0], (counted.listings.get(args[0]) ?? 0) + 1);
      const around = counted.aroundListing.get(args[0]);
      counted.aroundListing.delete(args[0]);
      around?.before();
      const listing = await actual.gitFirstSightListing(...args);
      around?.after();
      return listing;
    },
    readGitBlobs: async (...args: Parameters<typeof actual.readGitBlobs>) => {
      counted.blobReads.set(args[0], (counted.blobReads.get(args[0]) ?? 0) + 1);
      return await actual.readGitBlobs(...args);
    },
    // Each question of `HEAD` that starts Git: `gitHead`, or a `rev-parse` naming `HEAD`.
    gitHead: async (...args: Parameters<typeof actual.gitHead>) => {
      counted.heads += 1;
      return await actual.gitHead(...args);
    },
    runGitAsync: async (...args: Parameters<typeof actual.runGitAsync>) => {
      if (args[1].includes("rev-parse") && args[1].includes("HEAD")) counted.heads += 1;
      return await actual.runGitAsync(...args);
    },
  };
});
const { reads, stats, listings, blobReads } = counted;

// Each test opens several sessions and walks four or five checkouts many times over.
vi.setConfig({ testTimeout: 60_000 });

const bridges: CodeRuleBridge[] = [];
const directories: string[] = [];
const releases: Array<() => void> = [];
afterEach(async () => {
  releases.splice(0).forEach(release => release());
  counted.held.clear();
  counted.heldOpenings.clear();
  counted.busy.clear();
  counted.aroundListing.clear();
  await Promise.all(bridges.splice(0).map(bridge => bridge.close()));
  reads.clear();
  stats.length = 0;
  listings.clear();
  blobReads.clear();
  await Promise.all(directories.splice(0).map(async path => await removeFolder(path)));
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

/** What a test session's auditor answers beyond the defaults (`opened`). */
interface SessionOptions {
  readonly rules?: Rules;
  readonly ignores?: (path: string) => boolean;
  /** Checkouts nested in the worktrees, each owning its files, which no session watches as a worktree. */
  readonly nested?: readonly string[];
  /**
   * Where Git registers the worktrees, as the router names it: with it, as the router does, the session watches only
   * the other worktrees whose folder is there.
   */
  readonly registry?: string;
  /** What the auditor learns before each walk lists the other worktrees, as the router learns Git's list. */
  readonly learn?: (git: GitReads) => WorktreeListLearning | undefined;
}

/**
 * A session opened in `folder`, watching the other `checkouts` (read as they are at each walk) through `walks`. With
 * `rules`, it reads only files of their extensions, as they were at its last refresh of its rules; else every file. Git
 * ignores each path `ignores` names, as the session names it.
 */
async function opened(
  folder: string,
  checkouts: readonly string[],
  walks: WorktreeWalks,
  { rules, ignores, nested = [], registry, learn }: SessionOptions = {},
) {
  const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
    revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
  }));
  // As the router does: the session's own checkout is never one of the other worktrees.
  const worktreeRoots = vi.fn(() => checkouts.filter(checkout => checkout !== innermost(checkouts, folder) && (registry === undefined || existsSync(checkout))));
  let known = new Set(rules?.extensions);
  const auditor: CodeRuleAuditor = {
    openWorkspace: async () => undefined,
    acceptsPath: path => rules === undefined || known.has(extname(path)),
    refreshCodeRules: () => {
      known = new Set(rules?.extensions);
    },
    auditGeneratedEdits,
    close: async () => undefined,
    sourceRoot: path => innermost([...checkouts, ...nested], resolve(folder, path)),
    worktreeRoots,
    ...(ignores === undefined ? {} : { ignoresPath: ignores }),
    ...(registry === undefined ? {} : { worktreeRegistry: () => registry }),
    ...(learn === undefined ? {} : { learnWorktreeRoots: learn }),
  };
  const bridge = new CodeRuleBridge(folder, auditor, { worktreeWalks: walks });
  bridges.push(bridge);
  await bridge.openWorkspace(`session-${bridges.length}`);
  return { bridge, auditGeneratedEdits, worktreeRoots };
}

/** A session whose first turn, and the walk of the other worktrees after it, are done. */
async function session(
  folder: string,
  checkouts: readonly string[],
  walks: WorktreeWalks,
  options: SessionOptions = {},
): Promise<Session> {
  const opening = await opened(folder, checkouts, walks, options);
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

/**
 * The session's next audit of edits from here on, waited for itself rather than for a set time: a walk's delivery
 * comes when the walks' gap allows, which on a loaded runner is later than a fixed wait.
 */
function nextAudit({ auditGeneratedEdits }: Session): Promise<void> {
  const delivered = Promise.withResolvers<void>();
  const audit = auditGeneratedEdits.getMockImplementation()!;
  auditGeneratedEdits.mockImplementation(async (sessionId, batch) => {
    delivered.resolve();
    return await audit(sessionId, batch);
  });
  return delivered.promise;
}

/** Every edit a session's auditor was given, with its mode, in order. */
function audited({ auditGeneratedEdits }: Session): Array<[string[], string | undefined]> {
  return auditGeneratedEdits.mock.calls.map(([, batch]) => [
    batch.edits.map(edit => `${edit.deleted === true ? "deleted" : "written"} ${edit.path}`),
    batch.mode,
  ]);
}

/** Runs `run` as a shell call of `bridge`, in `cwd` where its host names one. */
/** `audited`, with whom each batch's audit belongs to (`CodeRuleEditBatch.owner`; the session's when absent). */
function owned({ auditGeneratedEdits }: Session): Array<[string[], string | undefined, string | undefined]> {
  return auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits.map(edit => edit.path), batch.mode, batch.owner]);
}

async function shell(bridge: CodeRuleBridge, id: string, run: () => unknown, cwd?: string): Promise<void> {
  await bridge.begin(id, "shell", { command: "./run.sh" }, undefined, cwd);
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

/** Holds the next opening of `file` until `release` is called; `entered` resolves as the opening starts. */
function holdOpening(file: string): { entered: Promise<void>; release: () => void } {
  const entered = Promise.withResolvers<void>();
  const released = Promise.withResolvers<void>();
  releases.push(released.resolve);
  counted.heldOpenings.set(file, async () => {
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

  // #534: each session watching the worktree recorded the change, sessions that never touched it included.
  it("records a shell write in a worktree that no session watching it made once, for the repository, read once", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    const path = join(worktrees[2]!, "src", "a.py");
    // The call runs in the second session's own folder, not in `w3`: no session made the write there.
    await shell(second.bridge, "shell-1", () => writeFile(path, "old\nmine\n"), worktrees[1]!);
    await turn(second, first);
    expect(first.auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode, batch.owner])).toEqual([
      [[{ path: "../w3/src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }], "log", "repository"],
    ]);
    expect(second.auditGeneratedEdits).not.toHaveBeenCalled();
    expect(reads.get(path)).toBe(2);
  });

  // #535 and #534: the writer was told nothing, and every other session recorded its write.
  it("steers the session whose call ran in a worktree on its shell write there, and no other session records it", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    await shell(second.bridge, "shell-1", () => writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n"), join(worktrees[2]!, "src"));
    await turn(second, first);
    expect([owned(first), owned(second)]).toEqual([[], [[["../w3/src/a.py"], undefined, undefined]]]);
  });

  it("leaves a worktree a session is open in to that session: its shell write there steers it, and no other session records it", async () => {
    const { main, checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    // The session in the main checkout watches `w1` and would record for the repository there.
    const watcher = await session(main, checkouts, walks);
    const inside = await session(worktrees[0]!, checkouts, walks);
    await shell(inside.bridge, "shell-1", () => writeFile(join(worktrees[0]!, "src", "a.py"), "old\nmine\n"));
    await turn(inside, watcher);
    expect([owned(inside), owned(watcher)]).toEqual([[[["src/a.py"], undefined, undefined]], []]);
  });

  it("takes an edit tool's write into another worktree as its writer's, wherever the agent was, and no session records a copy", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const watcher = await session(worktrees[1]!, checkouts, walks);
    const writer = await session(worktrees[0]!, checkouts, walks);
    const path = join(worktrees[2]!, "src", "a.py");
    // The edit tool runs where the agent is, in its own folder, and names the file it writes.
    await writer.bridge.begin("write-1", "write", { file_path: path, content: "old\nmine\n" }, undefined, worktrees[0]!);
    await writeFile(path, "old\nmine\n");
    await writer.bridge.finish("write-1");
    await turn(writer, watcher);
    expect(owned(watcher)).toEqual([]);
    expect(owned(writer).every(([, , owner]) => owner === undefined)).toBe(true);
  });

  // Review of #738 (B1): a session opened in a subfolder judges that folder alone, not its whole checkout.
  it("records once for the repository a change in a worktree outside the folder of a session open in a subfolder of it", async () => {
    const { main, checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const watcher = await session(main, checkouts, walks);
    await mkdir(join(worktrees[0]!, "pkg"), { recursive: true });
    const inside = await session(join(worktrees[0]!, "pkg"), checkouts, walks);
    await writeFile(join(worktrees[0]!, "pkg", "own.py"), "own = 1\n");
    await writeFile(join(worktrees[0]!, "src", "a.py"), "old\nmine\n");
    // The subfolder session's edit tool writes outside its folder: its result checked it, so no one records a copy.
    const edited = join(worktrees[0]!, "src", "b.py");
    await inside.bridge.begin("write-1", "write", { file_path: edited, content: "b = 1\n" });
    await writeFile(edited, "b = 1\n");
    await inside.bridge.finish("write-1");
    await turn(inside, watcher);
    expect(owned(watcher)).toEqual([[["../w1/src/a.py"], "log", "repository"]]);
  });

  // Review of #738 (F1): the session's own folder walk recorded its write before it closed.
  it("records no copy of a session's write in its own worktree once it closed before another session's walk", async () => {
    const { main, checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const watcher = await session(main, checkouts, walks);
    const inside = await session(worktrees[0]!, checkouts, walks);
    await shell(inside.bridge, "shell-1", () => writeFile(join(worktrees[0]!, "src", "a.py"), "old\nmine\n"));
    await inside.bridge.checkpoint("turn");
    await inside.bridge.close();
    await turn(watcher);
    expect(owned(watcher)).toEqual([]);
    // A later change there, which no session made, is recorded once.
    await writeFile(join(worktrees[0]!, "src", "a.py"), "old\nmine\ntheirs\n");
    await turn(watcher);
    expect(owned(watcher)).toEqual([[["../w1/src/a.py"], "log", "repository"]]);
  });

  // Review of #738 (F4): a session whose start failed this walk is still open, in the folder its last start named.
  it("leaves a session's folder to it in a walk its start failed, as its last start named it", async () => {
    const { main, checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const watcher = await session(main, checkouts, walks);
    let failing = false;
    walks.watch({ clock: async () => Date.now(), start: () => (failing ? undefined : quietWalk(worktrees[0]!)) });
    await turn(watcher);
    failing = true;
    await writeFile(join(worktrees[0]!, "src", "a.py"), "old\nmine\n");
    await turn(watcher);
    expect(owned(watcher)).toEqual([]);
  });

  // Review of #738 (F6): until a closed session's close time is known, a walk cannot tell its writes from later ones.
  it("records a change no session made after a session closed in its folder, once the walk knows when it closed", async () => {
    const { main, checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const watcher = await session(main, checkouts, walks);
    const closedAt = Promise.withResolvers<number>();
    const closing = walks.watch({ clock: () => closedAt.promise, start: () => quietWalk(worktrees[0]!) });
    await turn(watcher);
    const closed = closing.close();
    await writeFile(join(worktrees[0]!, "src", "a.py"), "old\nmine\n");
    const walked = turn(watcher);
    // The close came before the write; its time answers only after the walk has read the write, as a slow clock can.
    await new Promise(wake => setTimeout(wake, 1_000));
    closedAt.resolve(0);
    await Promise.all([walked, closed]);
    expect(owned(watcher)).toEqual([[["../w1/src/a.py"], "log", "repository"]]);
  });

  it("credits a write in a worktree to no session whose call was running elsewhere as it was made", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    // The first session's long command runs in its own folder while the second writes in `w3` from there.
    await first.bridge.begin("long-1", "shell", { command: "npm test" }, undefined, worktrees[0]!);
    await shell(second.bridge, "shell-1", () => writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n"), worktrees[2]!);
    await first.bridge.finish("long-1");
    await turn(second, first);
    expect([owned(first), owned(second)]).toEqual([[], [[["../w3/src/a.py"], undefined, undefined]]]);
  });

  it("reads a worktree added while two sessions in the main checkout are open once for both, and records a shell write there once", async () => {
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
    // The first appends to a file there from the shell; the other runs a command that writes nothing. The command ends
    // once the filesystem clock has passed the write's stamp: a walk that begins within the write's clock tick cannot
    // tell it from a later write of the same size in that tick, and rightly reads the file again at the next walk.
    await shell(first.bridge, "shell-3", async () => {
      await writeFile(path, "old\nmine\n");
      await awaitClockAfter(path);
    });
    await turn(first);
    await shell(second.bridge, "shell-4", () => undefined);
    await turn(second, first);
    // One walk reads the file for both sessions, as the worktree first and after the write. The first session's call ran
    // in the main checkout, so no session made the write: the first session to watch the worktree records it once, for
    // the repository.
    expect(reads.get(path)).toBe(2);
    expect([owned(first), owned(second)]).toEqual([[[["../w/src/a.py"], "log", "repository"]], []]);
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
    // The walk the other two share has taken `w4` for them before the first session's turn asks for another: a walk that
    // began while they were still taking it would find `w4` new to them still, and list it for them again.
    await second.bridge.settleWorktreeWalk();
    await third.bridge.settleWorktreeWalk();
    await turn(first, second, third);
    // `w4` is new to each session: listed once for the first session's walk and once for the walk the other two share.
    expect(listings.get(worktrees[3]!)).toBe(2);
  });

  it("lists a worktree's changed files from Git once for a session still taking the walk that first showed it, whatever walks begin meanwhile", async () => {
    const { checkouts, worktrees } = await repository();
    await mkdir(join(worktrees[2]!, "deep"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await opened(worktrees[1]!, checkouts, walks);
    const walk = holdListing(join(worktrees[2]!, "deep"));
    await second.bridge.checkpoint("turn");
    await walk.entered;
    // A tool call holds the second session's lane while it reads the file it names, so the second session takes the walk
    // that first shows it the other worktrees only once that call has begun.
    const call = holdOpening(join(worktrees[1]!, "src", "held.py"));
    const begun = second.bridge.begin("write-1", "write", { path: "src/held.py" });
    await call.entered;
    walk.release();
    // The first session's next walk begins while the second still takes that one, and shows it nothing first.
    await turn(first);
    call.release();
    await begun;
    await second.bridge.finish("write-1");
    await turn(second);
    // `w3` is new to the first session's first walk and to the second session's, and to no walk after them.
    expect(listings.get(worktrees[2]!)).toBe(2);
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
    // Its own gap holds: a walk it asks for again at once waits for it. A settle would start that walk now.
    await shell(second.bridge, "shell-1", () => writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n"));
    const again = await within(walks.idle(), 2_000);
    expect({ firstWalk, again }).toEqual({ firstWalk: true, again: false });
  });

  it("reads a worktree by the rules its sessions hold now, whichever session it reads through", async () => {
    const { checkouts, worktrees } = await repository();
    const rules: Rules = { extensions: new Set([".py"]) };
    const walks = new WorktreeWalks({ gapMs: 0 });
    // The first session, which every walk reads through, has no event once a rule for `.ts` files is added.
    await session(worktrees[0]!, checkouts, walks, { rules });
    const other = await session(worktrees[1]!, checkouts, walks, { rules });
    rules.extensions.add(".ts");
    await shell(other.bridge, "shell-1", () => writeFile(join(worktrees[2]!, "src", "b.ts"), "export const b = 1;\n"), worktrees[2]!);
    await other.bridge.settleWorktreeWalk();
    expect(audited(other)).toEqual([[["written ../w3/src/b.ts"], undefined]]);
  });

  it("records a file that changed again after the walk it takes read it, from the text it last recorded", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    const { path, release } = await heldTake(first, second, worktrees[2]!);
    // `a.py` changes again before the first session reads it.
    await writeFile(path, "old\nmine\nmore\n");
    release();
    await first.bridge.settleWorktreeWalk();
    await turn(second);
    await first.bridge.settleWorktreeWalk();
    // It records the whole change from the text Git holds, which it saw last, as its own walk read it.
    expect(first.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "../w3/src/g.py", language: "python", generatedText: "g = 1\n" }],
      [{ path: "../w3/src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\nmore\n" }],
    ]);
  });

  it("takes a file it cannot read as it takes a walk at the next walk, from the text it last recorded", async () => {
    const { checkouts, worktrees } = await repository();
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    const { path, release } = await heldTake(first, second, worktrees[2]!);
    // Another process holds `a.py` as the first session reads it.
    counted.busy.add(path);
    release();
    await first.bridge.settleWorktreeWalk();
    await turn(second);
    await first.bridge.settleWorktreeWalk();
    expect(first.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "../w3/src/g.py", language: "python", generatedText: "g = 1\n" }],
      [{ path: "../w3/src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }],
    ]);
  });
});

/**
 * Holds `recorder`'s take of the walk that finds `g.py` written in `worktree`, on its audit of it; then one walk sees
 * `a.py` there change and the next sees nothing new, so `recorder`, still taking the first, takes only the last, which
 * keeps no text of `a.py`. `release` lets the take go on. `writer` writes both from calls that run elsewhere, so no
 * session made them and `recorder`, the first session watching the worktree, records them for the repository.
 */
async function heldTake(recorder: Session, writer: Session, worktree: string): Promise<{ path: string; release: () => void }> {
  const entered = Promise.withResolvers<void>();
  const held = Promise.withResolvers<void>();
  releases.push(held.resolve);
  const audit = recorder.auditGeneratedEdits.getMockImplementation()!;
  recorder.auditGeneratedEdits.mockImplementationOnce(async (sessionId, batch) => {
    entered.resolve();
    await held.promise;
    return await audit(sessionId, batch);
  });
  await shell(writer.bridge, "shell-1", () => writeFile(join(worktree, "src", "g.py"), "g = 1\n"));
  await writer.bridge.settleWorktreeWalk();
  await entered.promise;
  const path = join(worktree, "src", "a.py");
  await shell(writer.bridge, "shell-2", () => writeFile(path, "old\nmine\n"));
  await writer.bridge.settleWorktreeWalk();
  await turn(writer);
  return { path, release: held.resolve };
}
/** How many times `lstat` was asked about each of `roots` or anything in it. */
function statsUnder(...roots: string[]): number[] {
  // The entries of each root's tree: the HEAD reader's look at its `.git` entry and at what is inside a `.git` folder
  // (the owners Git checks, `git-head-reader.ts`) is no look at an entry.
  const inGit = (path: string): boolean => path.split(sep).includes(".git");
  return roots.map(root => stats.filter(path => (path === root || path.startsWith(`${root}${sep}`)) && !inGit(path.slice(root.length))).length);
}

/** Every batch of edits a session's auditor was given, whole, with its mode. */
function edits({ auditGeneratedEdits }: Session): unknown[] {
  return auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode]);
}

/**
 * Walks whose notices come from `watch`, which sees every write, watching each worktree whole as on macOS and Windows;
 * `looks` stands in for what each walk looks at.
 */
function noticedWalks(watch = new StampTreeWatch(), options: { fullWalkMs?: number; looks?: WorktreeLooks } = {}): WorktreeWalks {
  return new WorktreeWalks({ gapMs: 0, ...options, notices: new WorktreeNotices({ treeWatch: watch, platform: "darwin" }) });
}

/** Walks with change notices (`noticedWalks`) and the watch they take them from. */
interface Noticing {
  readonly walks: WorktreeWalks;
  readonly watch: StampTreeWatch;
}

/** New walks with change notices from a new watch (`noticedWalks`), with that watch. */
function noticingWalks(): Noticing {
  const watch = new StampTreeWatch();
  return { watch, walks: noticedWalks(watch) };
}

/**
 * Writes `text` to `path`, in the watched worktree `root`, as one change. A write opens the file empty (it empties the
 * file, or makes it) before it writes the text, and on a loaded machine the two are far enough apart that a walk a
 * notice asks for can read the file empty and record that too, then the text as a second edit. No walk runs as the
 * write begins, and the notices of `root` wait until it ended.
 */
async function writeAsOneChange({ walks, watch }: Noticing, root: string, path: string, text: string): Promise<void> {
  await walks.idle();
  watch.hold(root);
  try {
    await writeFile(path, text);
  } finally {
    watch.release(root);
  }
}

/**
 * Two sessions in `folder`: one whose walks read only the worktrees a change notice named (through `noticing`), and one
 * whose walks read every worktree, as all walks did before notices. Each has had two walks.
 */
async function bothWays(folder: string, checkouts: readonly string[]): Promise<{ noticed: Session; everyWorktree: Session; noticing: Noticing }> {
  const noticing = noticingWalks();
  const noticed = await session(folder, checkouts, noticing.walks);
  const everyWorktree = await session(folder, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }));
  await turn(noticed, everyWorktree);
  return { noticed, everyWorktree, noticing };
}

describe("a walk of the other worktrees with change notices", () => {
  it("stats no entry of the other worktrees when nothing changed in them since the last walk", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks());
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 0]);
  });

  // A walk keeps what it read of a worktree while no notice tells of a change there, and hands that read to a session
  // that first sees the worktree at a later walk: here one whose list of the repository's worktrees names it only after
  // another session's walk read it. Git added the worktree after that session opened, so each of its files is stamped
  // since, and the walk lists the kept read's index against `HEAD` for that first sight.
  it("reads no blob of a worktree's files for a session that first sees it from a kept read, Git having added it after the session opened", async () => {
    const { main, worktrees, checkouts } = await repository(["w1"]);
    const walks = noticedWalks();
    const named = [main];
    const seer = await session(main, named, walks);
    const other = await session(worktrees[0]!, checkouts, walks);
    const added = join(dirname(main), "w2");
    git(main, "worktree", "add", "--quiet", "--detach", added);
    checkouts.push(added);
    await turn(other);
    named.push(added);
    reads.clear();
    await turn(seer);
    expect({ read: reads.get(join(added, "src", "a.py")) ?? 0, blobs: blobReads.get(added) ?? 0, audits: seer.auditGeneratedEdits.mock.calls.length })
      .toEqual({ read: 0, blobs: 0, audits: 0 });
  });

  // A kept read pairs what the walk read before with Git's listing of the worktree now. A file the session's command
  // changed there before that read, put back to HEAD's text for the listing and written again after it (`git stash`,
  // then `git stash pop`), is one Git names as holding that text, though the kept read and the file hold the command's.
  // The command still runs as the session first sees the worktree, so no walk of the session answered for it before.
  it("records a file its command changed in a worktree it first sees from a kept read, put back for the walk's listing and written again", async () => {
    const { main, worktrees, checkouts } = await repository(["w1"]);
    const walks = noticedWalks();
    const named = [main];
    const seer = await session(main, named, walks);
    const other = await session(worktrees[0]!, checkouts, walks);
    const added = join(dirname(main), "w2");
    git(main, "worktree", "add", "--quiet", "--detach", added);
    const file = join(added, "src", "a.py");
    await seer.bridge.begin("shell-1", "shell", { command: "./run.sh" }, undefined, added);
    await writeFile(file, "old\nmine\n");
    checkouts.push(added);
    await turn(other);
    named.push(added);
    counted.aroundListing.set(added, { before: () => writeFileSync(file, "old\n"), after: () => writeFileSync(file, "old\nmine\n") });
    await seer.bridge.finish("shell-1");
    await turn(seer);
    expect(audited(seer).flatMap(([paths]) => paths)).toContain("written ../w2/src/a.py");
  });

  // A stamp proves a file unchanged since a walk read it only where the walk read a stamp older than its own start: a
  // write in the clock tick of the read can leave the stamp as it was. So a file written after the walk began goes to
  // Git's reading, though Git names it as holding HEAD's text and its stamp holds after the listing.
  it("reads Git's verdict of a file written after the walk began, for a session that first sees its worktree", async () => {
    const { main } = await repository([]);
    const walks = noticedWalks();
    const named = [main];
    const seer = await session(main, named, walks);
    const added = join(dirname(main), "w2");
    git(main, "worktree", "add", "--quiet", "--detach", added);
    named.push(added);
    const listing = holdListing(join(added, "src"));
    const walked = turn(seer);
    await listing.entered;
    await writeFile(join(added, "src", "a.py"), "old\n");
    listing.release();
    await walked;
    expect(blobReads.get(added) ?? 0).toBe(1);
  });

  it("stats no entry when a file Git ignores changes after a walk that read no worktree", async () => {
    const { main, worktrees, checkouts } = await repository();
    const cache = join(worktrees[2]!, "src", "cache.tmp");
    await writeFile(cache, "x\n");
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(), { ignores: path => path.endsWith(".tmp") });
    await turn(watching);
    await writeFile(cache, "y\n");
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 0]);
  });

  it("records a file Git ignored, once the index tracks it, and a shell write to it, as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    const generated = join(w3, "src", "gen.py");
    await writeFile(generated, "x = 1\n");
    // Git ignores it, by a rule of its own: the index tracking it is what tells a walk it counts.
    await writeFile(join(w3, ".gitignore"), "src/gen.py\n");
    await awaitClockAfter(join(w3, ".gitignore"));
    // As Git answers: a path the index tracks is never ignored.
    let tracked = false;
    const ignores = (path: string): boolean => path.endsWith("gen.py") && !tracked;
    const noticing = noticingWalks();
    const noticed = await session(worktrees[0]!, checkouts, noticing.walks, { ignores });
    const everyWorktree = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }), { ignores });
    await turn(noticed, everyWorktree);
    git(w3, "add", "--force", "src/gen.py");
    tracked = true;
    // Nothing in `w3` changed: only its index tells that the file counts now.
    await turn(noticed, everyWorktree);
    await writeAsOneChange(noticing, w3, generated, "x = 2\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed).at(-1)).toEqual([["written ../w3/src/gen.py"], "log"]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("stats only the worktree a file changed in, and one session watching it records the change for the repository", async () => {
    const { main, worktrees, checkouts } = await repository();
    const noticing = noticingWalks();
    const first = await session(worktrees[0]!, checkouts, noticing.walks);
    const second = await session(worktrees[1]!, checkouts, noticing.walks);
    await turn(first, second);
    stats.length = 0;
    await writeAsOneChange(noticing, worktrees[2]!, join(worktrees[2]!, "src", "a.py"), "old\nmine\n");
    await turn(first, second);
    // `w3`, its `src` and `src/a.py`, once for both sessions.
    expect(statsUnder(main, worktrees[2]!)).toEqual([0, 3]);
    expect([owned(first), owned(second)]).toEqual([[[["../w3/src/a.py"], "log", "repository"]], []]);
  });

  it("asks Git for no worktree's HEAD at the walks after their first, while the files Git reads it from hold", async () => {
    const { worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks());
    // A configuration file vouches for a checkout's files only once it is the reader's settle time (2 s) old: the clock
    // the reader measures that on runs 2.1 s ahead from here on, as after a wait.
    const now = Date.now.bind(Date);
    const clock = vi.spyOn(Date, "now").mockImplementation(() => now() + 2_100);
    try {
      await turn(watching);
      counted.heads = 0;
      for (const round of [1, 2, 3]) {
        await writeFile(join(worktrees[2]!, "src", "a.py"), `old\n${round}\n`);
        await turn(watching);
      }
    } finally {
      clock.mockRestore();
    }
    // The walks still follow the worktree: its last write is recorded in Log.
    expect(audited(watching).at(-1)).toEqual([["written ../w3/src/a.py"], "log"]);
    expect(counted.heads).toBe(0);
  });

  it("reads a worktree whose HEAD moved though no file in it changed", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks());
    stats.length = 0;
    git(worktrees[2]!, "commit", "--quiet", "--allow-empty", "-m", "empty");
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 3]);
  });

  it("records a file created in another worktree as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const { noticed, everyWorktree, noticing } = await bothWays(worktrees[0]!, checkouts);
    await writeAsOneChange(noticing, worktrees[2]!, join(worktrees[2]!, "src", "new.py"), "n = 1\n");
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
    const { noticed, everyWorktree, noticing } = await bothWays(worktrees[0]!, checkouts);
    git(w3, "checkout", "--quiet", "other");
    await turn(noticed, everyWorktree);
    await awaitClockAfter(join(w3, "src", "a.py"));
    await writeAsOneChange(noticing, w3, join(w3, "src", "a.py"), "other\nmine\n");
    await turn(noticed, everyWorktree);
    // The write after the switch is measured against the branch's text.
    expect(edits(noticed).at(-1)).toEqual([[{ path: "../w3/src/a.py", language: "python", previousText: "other\n", generatedText: "other\nmine\n" }], "log"]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("records nothing a merge wrote beside a modify/delete conflict in a checkout inside another worktree, and a shell edit of the conflicted file, as walks of every worktree do", async () => {
    const { worktrees, checkouts } = await repository();
    const nested = join(worktrees[2]!, "lib");
    await mkdir(nested);
    await writeFile(join(nested, "gone.py"), "gone\n");
    await writeFile(join(nested, "both.py"), "1\n2\n3\n4\n5\n6\n7\n");
    git(nested, "init", "--quiet", "--initial-branch=main");
    git(nested, "add", ".");
    git(nested, "commit", "--quiet", "-m", "seed");
    git(nested, "switch", "--quiet", "-c", "other");
    git(nested, "rm", "--quiet", "gone.py");
    await writeFile(join(nested, "both.py"), "theirs\n2\n3\n4\n5\n6\n7\n");
    git(nested, "commit", "--quiet", "-am", "other");
    git(nested, "switch", "--quiet", "-c", "ours", "main");
    await writeFile(join(nested, "gone.py"), "gone\nours\n");
    await writeFile(join(nested, "both.py"), "1\n2\n3\n4\n5\n6\nours\n");
    git(nested, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(nested, "both.py"));
    const noticing = noticingWalks();
    const sessions = await Promise.all([
      session(worktrees[0]!, checkouts, noticing.walks, { nested: [nested] }),
      session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }), { nested: [nested] }),
    ]);
    const [noticed, everyWorktree] = sessions;
    // A change found there first, so each walk after it reads the checkout's `HEAD` too.
    await writeFile(join(nested, "note.py"), "mine\n");
    await turn(...sessions);
    for (const { auditGeneratedEdits } of sessions) auditGeneratedEdits.mockClear();
    expect(() => git(nested, "merge", "--quiet", "other")).toThrow();
    await turn(...sessions);
    expect(edits(noticed)).toEqual([]);
    await awaitClockAfter(join(nested, "both.py"));
    await writeAsOneChange(noticing, worktrees[2]!, join(nested, "gone.py"), "gone\nours\nmine\n");
    await turn(...sessions);
    expect(edits(noticed)).toEqual([
      [[{ path: "../w3/lib/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }], "log"],
    ]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("takes a worktree added since, and records a write in it, as walks of every worktree do", async () => {
    const { main, worktrees, checkouts } = await repository();
    const { noticed, everyWorktree, noticing } = await bothWays(worktrees[0]!, checkouts);
    const added = join(main, "..", "w4");
    git(main, "worktree", "add", "--quiet", "--detach", added);
    checkouts.push(added);
    await turn(noticed, everyWorktree);
    await awaitClockAfter(join(added, "src", "a.py"));
    await writeAsOneChange(noticing, added, join(added, "src", "a.py"), "old\nmine\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed).at(-1)).toEqual([["written ../w4/src/a.py"], "log"]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("drops a worktree removed since without recording its files, and lets go of its watch, as walks of every worktree do", async () => {
    const { main, worktrees, checkouts } = await repository();
    const noticing = noticingWalks();
    const noticed = await session(worktrees[0]!, checkouts, noticing.walks);
    const everyWorktree = await session(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0, fullWalkMs: 0 }));
    git(main, "worktree", "remove", "--force", worktrees[2]!);
    checkouts.splice(checkouts.indexOf(worktrees[2]!), 1);
    await turn(noticed, everyWorktree);
    expect(noticing.watch.unwatched).toEqual([worktrees[2]!]);
    await writeAsOneChange(noticing, worktrees[1]!, join(worktrees[1]!, "src", "a.py"), "old\nmine\n");
    await turn(noticed, everyWorktree);
    expect(audited(noticed)).toEqual([[["written ../w2/src/a.py"], "log"]]);
    expect(edits(noticed)).toEqual(edits(everyWorktree));
  });

  it("finds a change made while a read ran, after the read passed the file", async () => {
    const { worktrees, checkouts } = await repository();
    const most = join(worktrees[2]!, "deep", "inner", "most");
    await mkdir(most, { recursive: true });
    const watching = await opened(worktrees[0]!, checkouts, noticedWalks());
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
    const watch = new StampTreeWatch();
    for (const checkout of checkouts) watch.unwatchable(checkout);
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch));
    stats.length = 0;
    await writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n");
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([3, 3, 3]);
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
  });

  it("reads every worktree on every walk with no period to trust notices for", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(new StampTreeWatch(), { fullWalkMs: 0 }));
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([3, 3, 3]);
  });

  it("reads a worktree again when the system says it lost notices, or the watch of it ended", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch));
    // The write's notice is lost: no take hands it, and the next says notices were lost.
    await watch.unnoticed(worktrees[2]!, () => writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n"));
    watch.lose(worktrees[2]!);
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([0, 0, 3]);
    await watch.unnoticed(worktrees[1]!, () => writeFile(join(worktrees[1]!, "src", "a.py"), "old\nmine\n"));
    watch.end(worktrees[1]!);
    stats.length = 0;
    await turn(watching);
    // An ended watch has `w2` read whole (three looks) on every walk until it is watched again, each walk looking at
    // its folder first to tell another folder there (`folderAt`), so a walk the notice pump asks for on a loaded
    // machine reads it once more; the other worktrees stay unread.
    const [mainStats, w2Stats, w3Stats] = statsUnder(main, worktrees[1]!, worktrees[2]!);
    expect({ mainStats, w3Stats, w2ReadWhole: w2Stats! >= 4 && w2Stats! % 4 === 0 }).toEqual({ mainStats: 0, w3Stats: 0, w2ReadWhole: true });
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"], [["written ../w2/src/a.py"], "log"]]);
  });

  it("records a shell write in another worktree whose notice came after the walk took the notices, with no later event of the session", async () => {
    const { worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch));
    const w3 = worktrees[2]!;
    watch.hold(w3);
    await shell(watching.bridge, "shell-1", () => writeFile(join(w3, "src", "a.py"), "old\nmine\n"));
    await turn(watching);
    // The walk the command asked for took the notices before this one came.
    expect(audited(watching)).toEqual([]);
    // No event of the session asks for a walk from here on: the notice pump asks for the walk that records the write,
    // which waits as long as the walk before it took (the walks' gap) and then reads, so the case waits for the
    // session's next audit, not for a set time.
    const delivered = nextAudit(watching);
    watch.release(w3);
    await delivered;
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
  });

  it("reads a worktree removed and added again at its path after a walk while its folder was gone, and records a write there, with no later event of the session", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const registry = join(main, ".git", "worktrees");
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch), { registry });
    const w3 = worktrees[2]!;
    // Its watch ends once its folder is gone, as the engine's does, and tells of nothing before.
    watch.hold(w3);
    await removeFolder(w3);
    watch.end(w3);
    watch.release(w3);
    // Git still lists the worktree; the walk finds no folder there and lets go of it.
    await turn(watching);
    expect([watch.folders.has(w3), watch.folders.has(registry), watch.unwatched.includes(registry)]).toEqual([false, true, false]);
    git(main, "worktree", "add", "--quiet", "--force", "--detach", w3);
    await watch.until(() => watch.folders.has(w3));
    // The session let go of the worktree, so the walk that reads it again is its first read there, its baseline: the
    // write comes after it, and its change notice alone starts the walk that records it.
    await turn(watching);
    const delivered = nextAudit(watching);
    await writeFile(join(w3, "src", "a.py"), "old\nmine\n");
    await delivered;
    expect(owned(watching)).toEqual([[["../w3/src/a.py"], "log", "repository"]]);
    await watching.bridge.close();
    expect(watch.unwatched).toContain(registry);
  });

  it("watches a worktree removed and added again at its path as the next walk reads it, and records a later write there, with no later event of the session", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch), { registry: join(main, ".git", "worktrees") });
    const w3 = worktrees[2]!;
    // The watch of the folder that went tells of nothing until it ends, after the new folder is made.
    watch.hold(w3);
    await removeFolder(w3);
    git(main, "worktree", "add", "--quiet", "--force", "--detach", w3);
    watch.end(w3);
    watch.release(w3);
    // Its watch let go, then watched again: the new folder.
    await watch.until(() => watch.unwatched.includes(w3) && watch.folders.has(w3));
    const delivered = nextAudit(watching);
    await writeFile(join(w3, "src", "a.py"), "old\nmine\n");
    await delivered;
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
  });

  it("records no deletion of a file its tool call made in another worktree when the walk after it had no notice of it yet", async () => {
    const { worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch));
    const w3 = worktrees[2]!;
    const path = join(w3, "src", "new.py");
    watch.hold(w3);
    await watching.bridge.begin("write-1", "write", { path });
    await writeFile(path, "n = 1\n");
    await watching.bridge.finish("write-1");
    await turn(watching);
    const recorded = [[["written ../w3/src/new.py"], undefined]];
    expect(audited(watching)).toEqual(recorded);
    watch.release(w3);
    await turn(watching);
    await turn(watching);
    expect(audited(watching)).toEqual(recorded);
  });
});

describe("a session's close and the watches of the other worktrees", () => {
  it("lets go of the watch of each worktree no session left watches it, so Git can remove it at once", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const walks = noticedWalks(watch);
    const first = await session(worktrees[0]!, checkouts, walks);
    const second = await session(worktrees[1]!, checkouts, walks);
    await first.bridge.close();
    // Only the closed session watched `w2`, the open one's own folder; the open one still watches the others.
    expect(watch.unwatched).toEqual([worktrees[1]!]);
    await second.bridge.close();
    expect(watch.unwatched.sort()).toEqual([main, ...worktrees].sort());
    expect(runningIn(worktrees[2]!)).toEqual([]);
    git(main, "worktree", "remove", worktrees[2]!);
  });
});

describe("a session's close and the watch of where Git registers the worktrees", () => {
  it("keeps watching where Git registers the worktrees while any session of the repository is open", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const walks = noticedWalks(watch);
    const registry = join(main, ".git", "worktrees");
    const first = await session(worktrees[0]!, checkouts, walks, { registry });
    const second = await session(worktrees[1]!, checkouts, walks, { registry });
    await first.bridge.close();
    expect(watch.unwatched).not.toContain(registry);
    await second.bridge.close();
    expect(watch.unwatched).toContain(registry);
  });
});

describe("what a walk looks at in a watched worktree that no notice tells of", () => {
  it("reads a worktree whose folder was put in place of another though no notice came", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch));
    const w3 = worktrees[2]!;
    await watch.unnoticed(w3, async () => {
      await rename(w3, `${w3}-moved`);
      await cp(`${w3}-moved`, w3, { recursive: true });
    });
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, w3)).toEqual([0, 0, 3]);
  });

  it("reads every worktree once the repository's info/exclude stops ignoring a file, though no notice counted", async () => {
    const { main, worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    await writeFile(join(w3, "src", "gen.py"), "x = 1\n");
    const exclude = join(main, ".git", "info", "exclude");
    await writeFile(exclude, "gen.py\n");
    let excluded = true;
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch), { ignores: path => excluded && path.endsWith("gen.py") });
    await writeFile(exclude, "");
    excluded = false;
    stats.length = 0;
    await turn(watching);
    // Each worktree's `src/gen.py` or `src/a.py` as Git now sees it, `main` included.
    expect(statsUnder(main, worktrees[1]!, w3)).toEqual([3, 3, 4]);
  });

  it("reads a worktree once its configuration names another excludes file, though no notice counted", async () => {
    const { main, worktrees, checkouts } = await repository();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks());
    git(main, "config", "core.excludesFile", join(dirname(main), "global-ignore"));
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, worktrees[2]!)).toEqual([3, 3, 3]);
  });

  it("asks Git for the ignore files again after it could not name them, so a later change to one counts", async () => {
    const { main } = await repository();
    const looks = new WorktreeLooks();
    // A stopped `GitReads` starts no Git: each read answers as one past its time limit.
    const stopped = new GitReads();
    await stopped.stop();
    const failed = await looks.look(stopped, looks.begin(), main, []);
    const git = new GitReads();
    const named = await looks.look(git, looks.begin(), main, []);
    expect(failed.ignores).toBeUndefined();
    expect(lookMoved(failed, named)).toBe(true);
    expect(lookMoved(named, await looks.look(git, looks.begin(), main, []))).toBe(false);
    await writeFile(join(main, ".git", "info", "exclude"), "gen.py\n");
    expect(lookMoved(named, await looks.look(git, looks.begin(), main, []))).toBe(true);
  });

  // Only Linux has a mount table: the look reads one there alone, and its mount points are POSIX paths.
  it.runIf(process.platform === "linux")("reads a worktree a filesystem was mounted in, named as the mount table escapes it", async () => {
    const { main, worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    let table = "22 1 8:1 / / rw - ext4 /dev/sda1 rw\n";
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(new StampTreeWatch(), { looks: new WorktreeLooks({ mountInfo: () => table }) }));
    // A folder whose name holds a space, which the table writes as `\040`.
    table += `40 22 0:42 / ${join(w3, "build out").replaceAll(" ", "\\040")} rw - tmpfs tmpfs rw\n`;
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, w3)).toEqual([0, 0, 3]);
    table += `41 22 0:43 / ${join(dirname(w3), "elsewhere")} rw - tmpfs tmpfs rw\n`;
    stats.length = 0;
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, w3)).toEqual([0, 0, 0]);
  });

  it("reads a worktree whose nested checkout's merge stopped on a conflict that changed no file, though no notice counted", async () => {
    const { main, worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    const nested = join(w3, "lib");
    await mkdir(nested);
    await writeFile(join(nested, "b.py"), "b = 1\n");
    await writeFile(join(nested, "gone.py"), "gone = 1\n");
    git(nested, "init", "--quiet", "--initial-branch=main");
    git(nested, "add", ".");
    git(nested, "commit", "--quiet", "-m", "vendor");
    git(nested, "switch", "--quiet", "-c", "other");
    git(nested, "rm", "--quiet", "gone.py");
    git(nested, "commit", "--quiet", "-m", "other");
    git(nested, "switch", "--quiet", "-c", "ours", "main");
    await writeFile(join(nested, "gone.py"), "gone = 2\n");
    git(nested, "commit", "--quiet", "-am", "ours");
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(), { nested: [nested] });
    await turn(watching);
    stats.length = 0;
    // A modify/delete conflict keeps `gone.py` as it was: only the nested index's unmerged entries tell of it.
    expect(() => git(nested, "merge", "--quiet", "other")).toThrow();
    await turn(watching);
    const [mainStats, w2Stats, w3Stats] = statsUnder(main, worktrees[1]!, w3);
    expect({ mainStats, w2Stats, w3Read: w3Stats! > 0 }).toEqual({ mainStats: 0, w2Stats: 0, w3Read: true });
  });

  it("reads no worktree whose nested checkout's index was written again with nothing in it changed", async () => {
    const { main, worktrees, checkouts } = await repository();
    const w3 = worktrees[2]!;
    const nested = join(w3, "lib");
    await mkdir(nested);
    await writeFile(join(nested, "b.py"), "b = 1\n");
    git(nested, "init", "--quiet");
    git(nested, "add", ".");
    git(nested, "commit", "--quiet", "-m", "vendor");
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(), { nested: [nested] });
    await turn(watching);
    stats.length = 0;
    // As an editor's `git status` does when it refreshes the files' stat data.
    git(nested, "read-tree", "HEAD");
    git(nested, "status", "--porcelain");
    await turn(watching);
    expect(statsUnder(main, worktrees[1]!, w3)).toEqual([0, 0, 0]);
  });
});

/** A session's part in a walk where it watches no worktree, its folder the checkout at `folder`, and claims nothing. */
function quietWalk(folder: string): WatcherWalk {
  return {
    worktrees: new Set(), checkout: folder, folder, firstSight: new Set(), fresh: new Set(), startedAt: 0, heads: new Set(), unmerged: new Set(),
    scan: async () => undefined, holdsRefusal: () => false, claims: async () => new Set(), take: async () => undefined,
  };
}

/**
 * A session watching `roots` whose reads find nothing, each read of a root put in `scans`, and which takes nothing. With
 * `gaveUp`, each read gave up on a path it met (`SourceScan.gaveUp`).
 */
function watcherOf(roots: readonly string[], scans: string[], gaveUp = false): WorktreeWatcher {
  return {
    clock: async () => Date.now(),
    start: () => ({
      worktrees: new Set(roots), checkout: undefined, folder: "/nowhere", firstSight: new Set(), fresh: new Set(), startedAt: 0, heads: new Set(), unmerged: new Set(),
      scan: async ({ root }) => {
        scans.push(root);
        return {
          sources: new Map(), failedRoots: new Set(), errors: [], unread: new Map(), firstSeen: new Map(), checkouts: new Set([root]),
          unmerged: new Map(), lateListings: new Map(), seen: { folders: new Set([""]), leftOut: new Set() }, ...(gaveUp ? { gaveUp: true as const } : {}),
        };
      },
      holdsRefusal: () => false,
      claims: async () => new Set(),
      take: async () => undefined,
    }),
  };
}

/**
 * Resolves `ms` after `startedAt` (`Date.now()`). The reread is timed by the walks' own timers against the wall clock,
 * between reads that spawn Git: fake timers would hold those back, so these tests wait in real time.
 */
function until(startedAt: number, ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, Math.max(0, startedAt + ms - Date.now()));
  return promise;
}

describe("the reread of a worktree no notice told of a change in", () => {
  it("reads a worktree whose last read is the period old though every session watching it is idle", async () => {
    const { worktrees, checkouts } = await repository();
    const watch = new StampTreeWatch();
    const watching = await session(worktrees[0]!, checkouts, noticedWalks(watch, { fullWalkMs: 300 }));
    // The edit the reread finds reaches the session's auditor. That reread is due 300 ms after the last read, and its
    // walk then waits as long as the walk before it took (the walks' gap): on a loaded Windows runner that was more
    // than the 5 s this test once waited. So the test waits for the delivery itself, up to the test's own limit.
    const delivered = Promise.withResolvers<void>();
    const audit = watching.auditGeneratedEdits.getMockImplementation()!;
    watching.auditGeneratedEdits.mockImplementation(async (sessionId, batch) => {
      delivered.resolve();
      return await audit(sessionId, batch);
    });
    // A write through a hard link: the system tells of none.
    await watch.unnoticed(worktrees[2]!, () => writeFile(join(worktrees[2]!, "src", "a.py"), "old\nmine\n"));
    // No event of the session's asks for a walk from here on.
    await delivered.promise;
    expect(audited(watching)).toEqual([[["written ../w3/src/a.py"], "log"]]);
  });

  it("walks no worktree while every session is idle when no notice tells of a change, as every walk reads them all", async () => {
    const { worktrees } = await repository(["w1"]);
    const scans: string[] = [];
    const watch = new WorktreeWalks({ gapMs: 0, fullWalkMs: 300 }).watch(watcherOf([worktrees[0]!], scans));
    watch.request();
    await watch.settled();
    await until(Date.now(), 1_000);
    expect(scans).toHaveLength(1);
    await watch.close();
  });

  // A path gone as the read looked, no place, may have moved where it did not look: the next walk reads the worktree
  // again, as after a folder it could not list.
  it("reads a watched worktree again at the next walk, though no notice tells of a change, after a read that gave up on a path it met", async () => {
    const { worktrees } = await repository(["w1"]);
    for (const gaveUp of [false, true]) {
      const scans: string[] = [];
      const watch = noticedWalks().watch(watcherOf([worktrees[0]!], scans, gaveUp));
      watch.request();
      await watch.settled();
      watch.request();
      await watch.settled();
      expect(scans).toHaveLength(gaveUp ? 2 : 1);
      await watch.close();
    }
  });

  it("rereads in time though the timer fired before the read was due", async () => {
    const { worktrees } = await repository(["w1"]);
    const scans: string[] = [];
    const watch = noticedWalks(new StampTreeWatch(), { fullWalkMs: 600 }).watch(watcherOf([worktrees[0]!], scans));
    watch.request();
    await watch.settled();
    // The clock the reads are timed by runs 300 ms behind the timer from here on.
    const now = Date.now.bind(Date);
    const clock = vi.spyOn(Date, "now").mockImplementation(() => now() - 300);
    try {
      await vi.waitFor(() => expect(scans).toHaveLength(2), { timeout: 3_000 });
    } finally {
      clock.mockRestore();
      await watch.close();
    }
  });

  it("rereads for the sessions left when the one a reread was timed for closes before its walk began", async () => {
    const { worktrees } = await repository(["w1", "w2"]);
    const [w1, w2] = [worktrees[0]!, worktrees[1]!];
    const scans: string[] = [];
    const walks = new WorktreeWalks({
      gapMs: 1_500, fullWalkMs: 3_000, notices: new WorktreeNotices({ treeWatch: new StampTreeWatch(), platform: "darwin" }),
    });
    const [first, second] = [walks.watch(watcherOf([w1], scans)), walks.watch(watcherOf([w2], scans))];
    const startedAt = Date.now();
    const at = (ms: number): Promise<void> => until(startedAt, ms);
    first.request();
    await first.settled();
    await at(750);
    second.request();
    await second.settled();
    // A walk that reads nothing: the first session's next may begin only once its gap is over, at 3,750 ms.
    await at(2_250);
    first.request();
    await first.settled();
    // At 3,000 ms the first session's worktree is due, and the reread waits for its gap; it closes meanwhile.
    await at(3_350);
    await first.close();
    // The second session's worktree is due at 3,750 ms.
    await vi.waitFor(() => expect(scans.filter(root => root === w2)).toHaveLength(2), { timeout: 3_000 });
    expect(Date.now() - startedAt).toBeGreaterThan(3_700);
    await second.close();
  });
});

describe("the time between a session's walks", () => {
  /**
   * The wait the walk after one of the worktree in `roots` is timed for (`setTimeout`'s delay, in ms), its session
   * asking for it as that walk ends. That walk works `work` ms on every clock, then the time of day moves on `stood` ms
   * while the clock timers run on stands still: a suspend inside the walk on Linux, or the system's clock set on.
   */
  async function waitAfter(roots: readonly string[], { work, stood }: { work: number; stood: number }): Promise<unknown[]> {
    vi.useFakeTimers({ toFake: ["Date", "performance"] });
    const walks = new WorktreeWalks({ gapMs: 0 });
    let walked = 0;
    const watch = walks.watch(scanning(watcherOf(roots, []), () => {
      if (walked++ > 0) return;
      vi.advanceTimersByTime(work);
      vi.setSystemTime(Date.now() + stood);
    }));
    try {
      watch.request();
      await walks.idle();
      const timed = vi.spyOn(globalThis, "setTimeout");
      watch.request();
      const delays = timed.mock.calls.map(([, ms]) => ms);
      timed.mockRestore();
      return delays;
    } finally {
      await watch.close();
      vi.useRealTimers();
    }
  }

  it("waits as long after a walk as the walk took", async () => {
    const { worktrees } = await repository(["w1"]);
    expect(await waitAfter(worktrees, { work: 300, stood: 0 })).toEqual([300]);
  });

  it("waits as long after a walk as the walk took though the time of day moved on an hour more inside it", async () => {
    const { worktrees } = await repository(["w1"]);
    expect(await waitAfter(worktrees, { work: 300, stood: 3_600_000 })).toEqual([300]);
  });

  // What settles a session is what the walk it asked for finds. Its gap is the walks' pace, and a machine that stood
  // still inside the walk before it lengthens that gap by as long as it stood.
  it("settles a session's ask once the walk for it has run, with no wait for its gap", async () => {
    const { worktrees } = await repository(["w1"]);
    const scans: string[] = [];
    const watch = new WorktreeWalks({ gapMs: 3_600_000 }).watch(watcherOf(worktrees, scans));
    watch.request();
    await watch.settled();
    watch.request();
    await watch.settled();
    expect(scans).toHaveLength(2);
    await watch.close();
  });

  it("keeps the gap of a session that settles while it takes a walk it did not ask for", async () => {
    const { worktrees } = await repository(["w1"]);
    const walks = new WorktreeWalks({ gapMs: 3_600_000 });
    const held = Promise.withResolvers<void>();
    const taking = Promise.withResolvers<void>();
    let takes = 0;
    const first = walks.watch(taken(watcherOf(worktrees, []), async () => {
      if (takes++ === 0) return;
      taking.resolve();
      await held.promise;
    }));
    first.request();
    await first.settled();
    // The second session's first walk reads the worktree the first watches, so the first takes it too.
    const second = walks.watch(watcherOf(worktrees, []));
    second.request();
    await taking.promise;
    const settled = first.settled();
    held.resolve();
    await settled;
    const timed = vi.spyOn(globalThis, "setTimeout");
    first.request();
    const delays = timed.mock.calls.map(([, ms]) => Number(ms));
    timed.mockRestore();
    expect(delays.filter(ms => ms > 3_000_000)).toHaveLength(1);
    await Promise.all([first.close(), second.close()]);
  });
});

/** `watcher`, calling `during` as each read of a worktree begins. */
function scanning(watcher: WorktreeWatcher, during: () => void): WorktreeWatcher {
  return { ...watcher, start: () => {
    const walk = watcher.start()!;
    return { ...walk, scan: async request => {
      during();
      return await walk.scan(request);
    } };
  } };
}

/** `watcher`, taking each walk through `take`. */
function taken(watcher: WorktreeWatcher, take: () => Promise<void>): WorktreeWatcher {
  return { ...watcher, start: () => ({ ...watcher.start()!, take }) };
}

describe("the walks of each repository", () => {
  it("drops a repository's walk once no session watches it, and takes it back for a session it was handed to", async () => {
    const repositories = new RepositoryWorktreeWalks();
    const walks = repositories.of("r");
    const watch = walks.watch(watcherOf([], []));
    expect(repositories.of("r")).toBe(walks);
    await watch.close();
    const handed = repositories.of("r");
    expect(handed).not.toBe(walks);
    await handed.watch(watcherOf([], [])).close();
    // Dropped again, then watched by a session it was handed to before that.
    handed.watch(watcherOf([], []));
    expect(repositories.of("r")).toBe(handed);
  });
});

describe("what a session learns before a walk lists the other worktrees", () => {
  it("takes a throw of a session's learn step as nothing to learn, and walks once while the event loop turns", async () => {
    const { worktrees } = await repository(["w1"]);
    const scans: string[] = [];
    let learnt = 0;
    const watcher: WorktreeWatcher = {
      ...watcherOf([worktrees[0]!], scans),
      // Throws at its first 49 calls only, so walks begun again at each throw still end.
      prepare: () => {
        learnt += 1;
        if (learnt < 50) throw new Error("the checkout's placement cannot be read");
        return undefined;
      },
    };
    const watch = new WorktreeWalks({ gapMs: 0 }).watch(watcher);
    watch.request();
    const atFirstTurn = await new Promise<number>(turned => {
      setImmediate(() => turned(learnt));
    });
    await watch.settled();
    expect({ atFirstTurn, learnt, scans }).toEqual({ atFirstTurn: 1, learnt: 1, scans: [worktrees[0]] });
    await watch.close();
  });

  it("stops the session folder's walk once the list is learnt when the session closed meanwhile, and lists no worktree for it", async () => {
    const { worktrees, checkouts } = await repository(["w1"]);
    const entered = Promise.withResolvers<void>();
    const learnt = Promise.withResolvers<void>();
    releases.push(learnt.resolve);
    const { bridge, worktreeRoots } = await opened(worktrees[0]!, checkouts, new WorktreeWalks({ gapMs: 0 }), {
      learn: () => {
        entered.resolve();
        return { kind: "learning", done: learnt.promise };
      },
    });
    void bridge.checkpoint("turn").catch(() => undefined);
    await entered.promise;
    await bridge.close();
    learnt.resolve();
    // A walk that went on would list the worktrees in the steps that run before the event loop turns.
    await new Promise(turn => {
      setImmediate(turn);
    });
    expect(worktreeRoots).not.toHaveBeenCalled();
  });
});

describe("walks of sessions out of step", () => {
  it("reads a worktree for the first time once, though a session asks for a walk while another's walk reads it", async () => {
    const { worktrees, checkouts } = await repository();
    const deep = join(worktrees[2]!, "deep");
    await mkdir(deep);
    const walks = noticedWalks();
    const first = await opened(worktrees[0]!, checkouts, walks);
    const hold = holdListing(deep);
    await first.bridge.checkpoint("turn");
    // The first session's walk is reading `w3` for the first time as the second session opens and asks for its own.
    await hold.entered;
    const second = await opened(worktrees[1]!, checkouts, walks);
    await second.bridge.checkpoint("turn");
    hold.release();
    await first.bridge.settleWorktreeWalk();
    await second.bridge.settleWorktreeWalk();
    await turn(second);
    expect(reads.get(join(worktrees[2]!, "src", "a.py"))).toBe(1);
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
