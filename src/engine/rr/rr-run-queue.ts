/**
 * One `.rerail` run at a time per checkout, for the edits of every session working in it (plan decision 10). Each
 * session's checks of the checkout are an owner: it says it has work (`want`), and when a run starts it hands over
 * what it has gathered by then (`claim`). Work an owner gathers while a run goes on waits for the next run, which
 * carries all of it: edits that came in while one run went on are checked together, and a queued run is never run in
 * a form newer edits made stale, since the next run reads its edits only when it starts.
 */
import { resolve } from "node:path";
import { type RrRun, type RrRunOutcome, runRerailPolicies } from "./rr-runner.js";

/** What an owner hands a run as it starts: the policies its files need, and where the run's outcome goes. */
export interface RrClaim {
  /** The rule files to run (absolute). */
  readonly policies: readonly string[];
  /** Called once, with the run's outcome; `aborted` when the owner left the run before it ended. */
  settle(outcome: RrRunOutcome): void;
}

/** A session's checks of the checkout, as the queue sees them. */
export interface RrRunOwner {
  /** What the owner has gathered for a run starting now; undefined when it has nothing left to run. */
  claim(): RrClaim | undefined;
}

type Running = { readonly claims: Map<RrRunOwner, RrClaim>; readonly controller: AbortController };

/** What a claim its owner left settles with: the run went on without it, or was killed for it. */
const LEFT: RrRunOutcome = { status: "killed", reason: "aborted", wallMs: 0, peakRssBytes: 0 };

/** Whether `outcome` is that of a run its owner left, or that was stopped on purpose: no outcome of its rules. */
export function leftRun(outcome: RrRunOutcome): boolean {
  return outcome.status === "killed" && outcome.reason === "aborted";
}

/** The queue of the checkout at `root`, running each run with `run` under the deadline `deadlineMs`. */
export class RrRunQueue {
  readonly #root: string;
  readonly #deadlineMs: number;
  readonly #run: (run: RrRun) => Promise<RrRunOutcome>;
  readonly #waiting = new Set<RrRunOwner>();
  #running: Running | undefined;
  #idle: Promise<void> = Promise.resolve();

  constructor(root: string, deadlineMs: number, run: (run: RrRun) => Promise<RrRunOutcome> = runRerailPolicies) {
    this.#root = root;
    this.#deadlineMs = deadlineMs;
    this.#run = run;
  }

  /** The deadline each run of this queue is given. */
  get deadlineMs(): number {
    return this.#deadlineMs;
  }

  /** `owner` has work: it joins the next run, which starts at once when none is running. */
  want(owner: RrRunOwner): void {
    this.#waiting.add(owner);
    if (this.#running === undefined) this.#idle = this.#drain();
  }

  /**
   * `owner` leaves: it joins no later run, and a run it is in settles its claim `aborted` now. The running child is
   * killed when no other owner is left in it.
   */
  leave(owner: RrRunOwner): void {
    this.#waiting.delete(owner);
    const running = this.#running;
    const claim = running?.claims.get(owner);
    if (running === undefined || claim === undefined) return;
    running.claims.delete(owner);
    claim.settle(LEFT);
    if (running.claims.size === 0) running.controller.abort();
  }

  /** Whether no run is going on and none is wanted. */
  get idle(): boolean {
    return this.#running === undefined && this.#waiting.size === 0;
  }

  /** Settles once the runs going on or wanted now have ended. */
  async settled(): Promise<void> {
    while (!this.idle) await this.#idle;
  }

  /** Runs one run after another while some owner has work. */
  async #drain(): Promise<void> {
    for (let running = this.#start(); running !== undefined; running = this.#start()) await this.#runOnce(running);
  }

  /** The next run, with the claims of every waiting owner that still has work; undefined when none has. */
  #start(): Running | undefined {
    const claims = new Map<RrRunOwner, RrClaim>();
    for (const owner of this.#waiting) {
      const claim = owner.claim();
      if (claim !== undefined) claims.set(owner, claim);
    }
    this.#waiting.clear();
    this.#running = claims.size === 0 ? undefined : { claims, controller: new AbortController() };
    return this.#running;
  }

  async #runOnce(running: Running): Promise<void> {
    const policies = [...new Set([...running.claims.values()].flatMap(claim => claim.policies))].sort();
    const run = { root: this.#root, policies, deadlineMs: this.#deadlineMs, signal: running.controller.signal };
    const outcome = await this.#run(run).catch((error: unknown): RrRunOutcome => ({ status: "failed", message: String(error) }));
    for (const claim of running.claims.values()) claim.settle(outcome);
  }
}

const queues = new Map<string, RrRunQueue>();

/**
 * The queue of the checkout at `root` in this process, made on first use with the edit runs' deadline `deadlineMs`;
 * every session of this process working in the checkout shares it.
 */
export function rrRunQueue(root: string, deadlineMs: number): RrRunQueue {
  const key = resolve(root);
  let queue = queues.get(key);
  if (queue === undefined) {
    queue = new RrRunQueue(key, deadlineMs);
    queues.set(key, queue);
  }
  return queue;
}
