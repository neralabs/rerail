import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { editRunTiming } from "../../../src/engine/rr/rr-edit-runs.js";
import { type RrClaim, type RrRunOwner, RrRunQueue } from "../../../src/engine/rr/rr-run-queue.js";
import { NOTHING_RUN, type RrRunEnd, RrRunSlots } from "../../../src/engine/rr/rr-run-slots.js";
import type { RrBudgets, RrRun, RrRunOutcome } from "../../../src/engine/rr/rr-runner.js";

const QUIET_MS = 10_000;
const COMPLETED: RrRunOutcome = { status: "completed", findings: [], failures: [], files: 0, wallMs: 1, peakRssBytes: 1 };
const BUDGETS: RrBudgets = { deadlineMs: 60_000, threads: 2, memoryBytes: 1024 ** 3, store: false };

/** A runner whose runs end when the test says: each run started, and how to end it. */
function heldRunner() {
  const runs: { run: RrRun; end: (outcome: RrRunOutcome) => void }[] = [];
  const run = (request: RrRun): Promise<RrRunOutcome> => {
    const { promise, resolve } = Promise.withResolvers<RrRunOutcome>();
    runs.push({ run: request, end: resolve });
    request.signal?.addEventListener("abort", () => resolve({ status: "killed", reason: "aborted", wallMs: 1, peakRssBytes: 1, budgets: BUDGETS }));
    return promise;
  };
  return { runs, run, slots: new RrRunSlots(run) };
}

/** An owner that hands over the files written since its last claim, and keeps each claim's outcome. */
class Owner implements RrRunOwner {
  readonly #policy: string;
  #written: string[] = [];
  readonly claimed: string[][] = [];
  readonly outcomes: RrRunEnd[] = [];

  constructor(policy: string) {
    this.#policy = policy;
  }

  write(queue: RrRunQueue, path: string): void {
    this.#written.push(path);
    queue.want(this);
  }

  claim(): RrClaim | undefined {
    if (this.#written.length === 0) return undefined;
    this.claimed.push(this.#written);
    this.#written = [];
    return { policies: () => [this.#policy], settle: outcome => this.outcomes.push(outcome) };
  }
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("a checkout's queue of .rerail runs after edits", () => {
  it("starts one run for a burst of writes, once the checkout has been quiet", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.slots);
    const owner = new Owner("/rules/a.rerail");
    owner.write(queue, "a.py");
    await vi.advanceTimersByTimeAsync(QUIET_MS - 1);
    owner.write(queue, "b.py");
    await vi.advanceTimersByTimeAsync(QUIET_MS - 1);
    expect(runner.runs).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    expect(runner.runs.map(({ run }) => run.policies)).toEqual([["/rules/a.rerail"]]);
    expect(owner.claimed).toEqual([["a.py", "b.py"]]);
  });

  it("starts a waiting run at once when the turn ends", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.slots);
    const owner = new Owner("/rules/a.rerail");
    owner.write(queue, "a.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(owner.claimed).toEqual([["a.py"]]);
  });

  it("runs one run at a time, and gathers every session's writes made meanwhile into the next", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.slots);
    const first = new Owner("/rules/a.rerail");
    const second = new Owner("/rules/b.rerail");
    first.write(queue, "a.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(0);
    first.write(queue, "a.py");
    second.write(queue, "b.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(QUIET_MS * 2);
    expect(runner.runs).toHaveLength(1);
    runner.runs[0]!.end(COMPLETED);
    await vi.advanceTimersByTimeAsync(0);
    expect(runner.runs.map(({ run }) => run.policies)).toEqual([["/rules/a.rerail"], ["/rules/a.rerail", "/rules/b.rerail"]]);
    expect(first.claimed).toEqual([["a.py"], ["a.py"]]);
    expect(second.claimed).toEqual([["b.py"]]);
    expect(first.outcomes).toEqual([COMPLETED]);
  });

  it("never runs a queued run in a form newer writes made stale: the run takes the writes made up to its start", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.slots);
    const owner = new Owner("/rules/a.rerail");
    owner.write(queue, "a.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(0);
    owner.write(queue, "a.py");
    await vi.advanceTimersByTimeAsync(QUIET_MS);
    owner.write(queue, "b.py");
    runner.runs[0]!.end(COMPLETED);
    // The write of b.py, after a.py's quiet period, gives the waiting run another one.
    await vi.advanceTimersByTimeAsync(QUIET_MS - 1);
    expect(runner.runs).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(owner.claimed).toEqual([["a.py"], ["a.py", "b.py"]]);
  });

  it("kills the run a leaving owner was alone in, and settles a claim at once when another owner is still in the run", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.slots);
    const first = new Owner("/rules/a.rerail");
    const second = new Owner("/rules/b.rerail");
    first.write(queue, "a.py");
    second.write(queue, "b.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(0);
    const signal = runner.runs[0]!.run.signal!;
    queue.leave(first);
    expect(first.outcomes).toEqual([{ status: "left" }]);
    expect(signal.aborted).toBe(false);
    queue.leave(second);
    expect(signal.aborted).toBe(true);
    expect(second.outcomes).toHaveLength(1);
  });
});

describe("a checkout's queue when an owner fails", () => {
  it("settles every other owner's claim when one owner's settle throws, and leaves no rejection unhandled", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.slots);
    const failing: RrRunOwner = { claim: () => ({ policies: () => ["/rules/a.rerail"], settle: () => { throw new Error("the audit store is closed"); } }) };
    const other = new Owner("/rules/b.rerail");
    const unhandled = vi.fn();
    process.on("unhandledRejection", unhandled);
    try {
      queue.want(failing);
      other.write(queue, "b.py");
      queue.flush();
      await vi.advanceTimersByTimeAsync(0);
      runner.runs[0]!.end(COMPLETED);
      await vi.advanceTimersByTimeAsync(0);
      expect(other.outcomes).toEqual([COMPLETED]);
      // The next write runs: the queue is not left holding a run that never ended.
      other.write(queue, "c.py");
      queue.flush();
      await vi.advanceTimersByTimeAsync(0);
      expect(runner.runs).toHaveLength(2);
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      process.off("unhandledRejection", unhandled);
    }
  });
});

describe("the .rerail runs of one process", () => {
  /** A turn that makes the run over `root` when its turn comes, noting when that was. */
  const turn = (root: string, made: string[], signal?: AbortSignal) => ({
    signal,
    start: () => { made.push(root); return { root, policies: [`/rules${root}.rerail`] }; },
  });

  it("start one child at a time, make each run only when its turn comes, and drop a run aborted while it waits", async () => {
    const runner = heldRunner();
    const made: string[] = [];
    const first = runner.slots.run(turn("/a", made));
    const waiting = new AbortController();
    const dropped = runner.slots.run(turn("/b", made, waiting.signal));
    const third = runner.slots.run(turn("/c", made));
    await vi.advanceTimersByTimeAsync(0);
    expect(runner.runs.map(({ run }) => run.root)).toEqual(["/a"]);
    // The runs behind are made only at their turn: what they carry is decided then.
    expect(made).toEqual(["/a"]);
    waiting.abort();
    expect(await dropped).toEqual({ status: "left" });
    runner.runs[0]!.end(COMPLETED);
    expect(await first).toEqual(COMPLETED);
    await vi.advanceTimersByTimeAsync(0);
    expect(made).toEqual(["/a", "/c"]);
    runner.runs[1]!.end(COMPLETED);
    expect(await third).toEqual(COMPLETED);
  });

  it("settles a run whose signal was aborted before it asked at once, behind a run still going", async () => {
    const runner = heldRunner();
    const made: string[] = [];
    void runner.slots.run(turn("/a", made));
    expect(await runner.slots.run(turn("/b", made, AbortSignal.abort()))).toEqual({ status: "left" });
    expect(made).toEqual(["/a"]);
  });

  it("starts no child for a turn with nothing left to run", async () => {
    const runner = heldRunner();
    expect(await runner.slots.run({ start: () => undefined })).toEqual(NOTHING_RUN);
    expect(runner.runs).toEqual([]);
  });
});

describe("the quiet period's setting", () => {
  it("refuses a value past the longest delay a timer keeps, which would start every run at once", () => {
    expect(() => editRunTiming({ RERAIL_RR_QUIET_MS: "9999999999" })).toThrow(/RERAIL_RR_QUIET_MS is "9999999999": .* from 1 to 2147483647/u);
    expect(editRunTiming({ RERAIL_RR_QUIET_MS: "2147483647" }).quietMs).toBe(2_147_483_647);
  });
});
