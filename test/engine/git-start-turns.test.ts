import { execFileSync } from "node:child_process";
import type * as ChildProcess from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { gitHead, readGitBlobIds, readGitBlobs } from "../../src/engine/git.js";
import { BACKGROUND_SHARE, inGitBackground, OperationQueue } from "../../src/engine/git-start-turns.js";

/**
 * The turn of the event loop each Git process started in, and how it started, in order: `gitHead` starts with
 * `execFile`, `readGitBlobIds` and `readGitBlobs` with `spawn`. `turn` counts the loop's check phases, which come after
 * its poll for I/O, so two processes with the same turn started with no poll between them.
 */
const starts = vi.hoisted(() => ({ turn: 0, turns: [] as number[], kinds: [] as string[] }));
vi.mock("node:child_process", async importOriginal => {
  const actual = await importOriginal<typeof ChildProcess>();
  const execFile = (...call: Parameters<typeof actual.execFile>) => {
    starts.turns.push(starts.turn);
    starts.kinds.push("execFile");
    return (actual.execFile as (...forwarded: unknown[]) => ReturnType<typeof actual.execFile>)(...call);
  };
  const spawn = (...call: Parameters<typeof actual.spawn>) => {
    starts.turns.push(starts.turn);
    starts.kinds.push("spawn");
    return (actual.spawn as (...forwarded: unknown[]) => ReturnType<typeof actual.spawn>)(...call);
  };
  return { ...actual, execFile, spawn };
});

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

async function repository(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-turns-"));
  roots.push(root);
  git(root, "init", "--quiet");
  await writeFile(join(root, "a.py"), "print(1)\n");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

/** Counts the loop's turns while `work` runs. */
async function countingTurns<T>(work: () => Promise<T>): Promise<T> {
  let counting = true;
  const tick = (): void => {
    starts.turn += 1;
    if (counting) setImmediate(tick);
  };
  setImmediate(tick);
  try {
    return await work();
  } finally {
    counting = false;
  }
}

describe("Git process starts", () => {
  it("start one at a time, each in a turn of the event loop of its own, however many reads are asked for at once", async () => {
    const root = await repository();
    const head = git(root, "rev-parse", "HEAD").trim();
    const blob = git(root, "rev-parse", "HEAD:a.py").trim();
    starts.turns.length = 0;
    const answers = await countingTurns(async () => await Promise.all([
      ...Array.from({ length: 8 }, async () => await gitHead(root)),
      ...Array.from({ length: 6 }, async () => (await readGitBlobIds(root, ["HEAD:./a.py"])).get("HEAD:./a.py")),
      ...Array.from({ length: 6 }, async () => (await readGitBlobs(root, ["HEAD:./a.py"], { maxBlobBytes: 1_000 })).get("HEAD:./a.py")?.toString("utf8")),
    ]));
    expect(answers).toEqual([...Array<string>(8).fill(head), ...Array<string>(6).fill(blob), ...Array<string>(6).fill("print(1)\n")]);
    expect(starts.turns).toHaveLength(20);
    // Each process holds the thread while it starts, so turns that start more than one keep the process's sockets
    // waiting for all of them.
    const perTurn = new Map<number, number>();
    for (const turn of starts.turns) perTurn.set(turn, (perTurn.get(turn) ?? 0) + 1);
    expect([...perTurn.values()].filter(count => count > 1)).toEqual([]);
  });

  it("let a read start before the reads of background work asked for earlier", async () => {
    const root = await repository();
    starts.kinds.length = 0;
    const background = inGitBackground(() => Array.from({ length: 6 }, async () => await gitHead(root)));
    const own = readGitBlobIds(root, ["HEAD:./a.py"]);
    await Promise.all([...background, own]);
    // At most one background read goes first, when it is owed its share of the starts.
    expect(starts.kinds.indexOf("spawn")).toBeLessThan(2);
  });

  it("still start a background read while other reads keep waiting", async () => {
    const root = await repository();
    starts.kinds.length = 0;
    const own = Array.from({ length: 12 }, async () => await gitHead(root));
    const background = inGitBackground(async () => await readGitBlobIds(root, ["HEAD:./a.py"]));
    await Promise.all([...own, background]);
    expect(starts.kinds).toHaveLength(13);
    expect(starts.kinds.indexOf("spawn")).toBeLessThan(BACKGROUND_SHARE);
  });

  it("let a read start before the reads of an operation background work queued, while nothing waits behind it", async () => {
    const root = await repository();
    const queue = new OperationQueue();
    starts.kinds.length = 0;
    const queued = inGitBackground(async () => await queue.run(async () => await Promise.all(Array.from({ length: 6 }, async () => await gitHead(root)))));
    // The operation asks for its reads as it starts, before the loop's next turn.
    await nextTurn();
    const own = readGitBlobIds(root, ["HEAD:./a.py"]);
    await Promise.all([queued, own]);
    expect(starts.kinds).toHaveLength(7);
    expect(starts.kinds.indexOf("spawn")).toBeLessThan(2);
  });

  it("start the waiting reads of an operation background work queued in turn once an operation queues behind it", async () => {
    const root = await repository();
    const queue = new OperationQueue();
    starts.kinds.length = 0;
    const background = inGitBackground(() => Array.from({ length: 6 }, async () => await gitHead(root)));
    const queued = inGitBackground(async () => await queue.run(async () => await readGitBlobIds(root, ["HEAD:./a.py"])));
    // One background read has started, and the operation's read waits behind the other five.
    await nextTurn();
    const behind = queue.run(async () => await Promise.resolve("behind"));
    await Promise.all([...background, queued]);
    expect(await behind).toBe("behind");
    expect(starts.kinds.indexOf("spawn")).toBeLessThan(2);
  });

  it("start the reads of an operation background work queued in turn when the queue is waited on to settle", async () => {
    const root = await repository();
    const queue = new OperationQueue();
    starts.kinds.length = 0;
    const background = inGitBackground(() => Array.from({ length: 6 }, async () => await gitHead(root)));
    const queued = inGitBackground(async () => await queue.run(async () => await readGitBlobIds(root, ["HEAD:./a.py"])));
    const settled = queue.settled();
    await Promise.all([...background, queued, settled]);
    expect(starts.kinds.indexOf("spawn")).toBeLessThan(2);
  });
});

/** Resolves in the loop's next check phase, after any check-phase callback scheduled before it. */
async function nextTurn(): Promise<void> {
  await new Promise(resolve => { setImmediate(resolve); });
}
