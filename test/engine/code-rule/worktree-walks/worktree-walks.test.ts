import type * as ChildProcess from "node:child_process";
import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { appendFile, lstat, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleAuditor, CodeRuleEditBatch } from "../../../../src/engine/code-rule/code-rule.js";
import { WorktreeWalks } from "../../../../src/engine/code-rule/worktree-walks.js";

/**
 * How many times each file was read whole through `node:fs/promises`, by the path it was read at; the listings held back
 * until a test lets them go, by folder; and how many Git processes listed each checkout's changed files
 * (`git ls-files --modified --others`), by the folder Git ran in.
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
vi.mock("node:child_process", async importOriginal => {
  const actual = await importOriginal<typeof ChildProcess>();
  return {
    ...actual,
    execFile: (file: string, args: unknown, ...rest: unknown[]): unknown => {
      if (Array.isArray(args) && args.includes("ls-files") && args.includes("--modified")) {
        const folder = String(args[args.indexOf("-C") + 1]);
        counted.listings.set(folder, (counted.listings.get(folder) ?? 0) + 1);
      }
      return Reflect.apply(actual.execFile, undefined, [file, args, ...rest]);
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

/** A session opened in `folder`, watching the other `checkouts` (read as they are at each walk) through `walks`. */
async function opened(folder: string, checkouts: readonly string[], walks: WorktreeWalks) {
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
  };
  const bridge = new CodeRuleBridge(folder, auditor, { worktreeWalks: walks });
  bridges.push(bridge);
  await bridge.openWorkspace(`session-${bridges.length}`);
  return { bridge, auditGeneratedEdits };
}

/** A session whose first turn, and the walk of the other worktrees after it, are done. */
async function session(folder: string, checkouts: readonly string[], walks: WorktreeWalks): Promise<Session> {
  const opening = await opened(folder, checkouts, walks);
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

/** Each edit a session's auditor was given, by path: the text the session measured it from, else why it had none. */
function told({ auditGeneratedEdits }: Session): Array<[string, string | undefined]> {
  return auditGeneratedEdits.mock.calls
    .flatMap(([, batch]) => batch.edits.map(edit => [edit.path, edit.previousText ?? edit.previousReadError] as [string, string | undefined]))
    .sort(([left], [right]) => left.localeCompare(right));
}

/** How many times Git listed the changed files of any of `folders`. */
function listedIn(folders: readonly string[]): number {
  return folders.reduce((sum, folder) => sum + (listings.get(folder) ?? 0), 0);
}

/**
 * Gives the worktree at `worktree` a file changed since Git staged it, an untracked file, a file Git ignores and a file
 * deleted since Git staged it, all before any session opens.
 */
async function dirty(worktree: string): Promise<void> {
  await writeFile(join(worktree, "src", "gone.py"), "gone\n");
  git(worktree, "add", "src/gone.py");
  git(worktree, "commit", "--quiet", "-m", "gone");
  await rm(join(worktree, "src", "gone.py"));
  await writeFile(join(worktree, "src", "a.py"), "old\nbefore\n");
  await writeFile(join(worktree, "src", "new.py"), "draft\n");
  await writeFile(join(worktree, ".gitignore"), "src/ignored.py\n");
  await writeFile(join(worktree, "src", "ignored.py"), "kept\n");
  await awaitClockAfter(join(worktree, "src", "ignored.py"));
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
    // `w4` is new to each session: listed for the first session's walk, and kept for the walk the other two share.
    expect(listings.get(worktrees[3]!)).toBe(1);
  });

  it("lists each worktree's changed files from Git once for the sessions that first take it, and not for one joining later", async () => {
    const { main, checkouts, worktrees } = await repository(["w1", "w2", "w3", "w4"]);
    const walks = new WorktreeWalks({ gapMs: 0 });
    // Three sessions in `main` open one after the other, so each first takes the four worktrees in a walk of its own.
    for (let opened = 0; opened < 3; opened += 1) await session(main, checkouts, walks);
    expect(listedIn(worktrees)).toBe(4);
    await session(main, checkouts, walks);
    expect(listedIn(worktrees)).toBe(4);
  });

  it("tells each session that first takes a worktree what Git lists of it, as when each session listed it itself", async () => {
    const { main, checkouts, worktrees } = await repository(["w1", "w2", "w3", "w4"]);
    const w4 = worktrees[3]!;
    await dirty(w4);
    const walks = new WorktreeWalks({ gapMs: 0 });
    const sessions: Session[] = [];
    for (const folder of [worktrees[0]!, worktrees[1]!, worktrees[2]!, main]) sessions.push(await session(folder, checkouts, walks));
    await turn(...sessions);
    for (const name of ["a.py", "new.py", "ignored.py", "gone.py"]) await appendFile(join(w4, "src", name), "after\n");
    await turn(...sessions);
    // What each session measured the writes from when it listed the worktree itself: its first sight of the modified and
    // untracked files; none of the file Git ignores, which Git does not list; Git's text of the deleted one.
    const expected = [
      ["../w4/src/a.py", "old\nbefore\n"],
      ["../w4/src/gone.py", "gone\n"],
      ["../w4/src/ignored.py", "Source changed outside a tool with a captured preimage"],
      ["../w4/src/new.py", "draft\n"],
    ];
    expect(sessions.map(told)).toEqual(sessions.map(() => expected));
  });

  it("lists a worktree again for a session joining after its index changed", async () => {
    const { main, checkouts, worktrees } = await repository(["w1", "w2"]);
    const w2 = worktrees[1]!;
    await writeFile(join(w2, "src", "a.py"), "old\nbefore\n");
    await awaitClockAfter(join(w2, "src", "a.py"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    await session(worktrees[0]!, checkouts, walks);
    git(w2, "add", "src/a.py");
    await session(main, checkouts, walks);
    expect(listings.get(w2)).toBe(2);
  });

  it("tells a session joining after the repository's info/exclude changed what Git lists now", async () => {
    const { main, checkouts, worktrees } = await repository(["w1", "w2"]);
    const w2 = worktrees[1]!;
    await writeFile(join(w2, "src", "new.py"), "draft\n");
    await awaitClockAfter(join(w2, "src", "new.py"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    await mkdir(join(main, ".git", "info"), { recursive: true });
    await appendFile(join(main, ".git", "info", "exclude"), "\nsrc/new.py\n");
    const second = await session(main, checkouts, walks);
    await appendFile(join(w2, "src", "new.py"), "after\n");
    await turn(first, second);
    // Git listed the file to the first session only: only it measures the write from what it first saw.
    expect([told(first), told(second)]).toEqual([
      [["../w2/src/new.py", "draft\n"]],
      [["../w2/src/new.py", "Source changed outside a tool with a captured preimage"]],
    ]);
  });

  it("tells a session joining after core.excludesFile or the file it names changed what Git lists now", async () => {
    const { main, checkouts, worktrees } = await repository(["w1", "w2", "w3"]);
    const w3 = worktrees[2]!;
    const excludes = join(main, "..", "excludes");
    await writeFile(excludes, "");
    await writeFile(join(w3, "src", "new.py"), "draft\n");
    await awaitClockAfter(join(w3, "src", "new.py"));
    const walks = new WorktreeWalks({ gapMs: 0 });
    const first = await session(worktrees[0]!, checkouts, walks);
    git(main, "config", "core.excludesFile", excludes);
    const second = await session(worktrees[1]!, checkouts, walks);
    await writeFile(excludes, "src/new.py\n");
    const third = await session(main, checkouts, walks);
    await appendFile(join(w3, "src", "new.py"), "after\n");
    await turn(first, second, third);
    expect([told(first), told(second), told(third)]).toEqual([
      [["../w3/src/new.py", "draft\n"]],
      [["../w3/src/new.py", "draft\n"]],
      [["../w3/src/new.py", "Source changed outside a tool with a captured preimage"]],
    ]);
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
