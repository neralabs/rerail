import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type RrClaim, type RrRunOwner, RrRunQueue } from "../../../src/engine/rr/rr-run-queue.js";
import type { RrRun, RrRunOutcome } from "../../../src/engine/rr/rr-runner.js";

const QUIET_MS = 10_000;
const COMPLETED: RrRunOutcome = { status: "completed", findings: [], failures: [], files: 0, wallMs: 1, peakRssBytes: 1 };

/** A runner whose runs end when the test says: each run started, and how to end it. */
function heldRunner() {
  const runs: { run: RrRun; end: (outcome: RrRunOutcome) => void }[] = [];
  const run = (request: RrRun): Promise<RrRunOutcome> => {
    const { promise, resolve } = Promise.withResolvers<RrRunOutcome>();
    runs.push({ run: request, end: resolve });
    request.signal?.addEventListener("abort", () => resolve({ status: "killed", reason: "aborted", wallMs: 1, peakRssBytes: 1 }));
    return promise;
  };
  return { runs, run };
}

/** An owner that hands over the files written since its last claim, and keeps each claim's outcome. */
class Owner implements RrRunOwner {
  readonly #policy: string;
  #written: string[] = [];
  readonly claimed: string[][] = [];
  readonly outcomes: RrRunOutcome[] = [];

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
    return { policies: [this.#policy], settle: outcome => this.outcomes.push(outcome) };
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
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.run);
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
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.run);
    const owner = new Owner("/rules/a.rerail");
    owner.write(queue, "a.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(owner.claimed).toEqual([["a.py"]]);
  });

  it("runs one run at a time, and gathers every session's writes made meanwhile into the next", async () => {
    const runner = heldRunner();
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.run);
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
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.run);
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
    const queue = new RrRunQueue("/repo", { deadlineMs: 60_000, quietMs: QUIET_MS }, runner.run);
    const first = new Owner("/rules/a.rerail");
    const second = new Owner("/rules/b.rerail");
    first.write(queue, "a.py");
    second.write(queue, "b.py");
    queue.flush();
    await vi.advanceTimersByTimeAsync(0);
    const signal = runner.runs[0]!.run.signal!;
    queue.leave(first);
    expect(first.outcomes).toEqual([{ status: "killed", reason: "aborted", wallMs: 0, peakRssBytes: 0 }]);
    expect(signal.aborted).toBe(false);
    queue.leave(second);
    expect(signal.aborted).toBe(true);
    expect(second.outcomes).toHaveLength(1);
  });
});
