/**
 * One `.rerail` run at a time per checkout, for the edits of every session working in it (plan decision 10). Each
 * session's checks of the checkout are an owner: it says it has work (`want`), and when a run starts it hands over
 * what it has gathered by then (`claim`).
 *
 * A run costs tens of seconds and gigabytes, so it does not start on the first write of a burst: it starts once the
 * checkout has had no further write for the quiet period, or at once when the agent's turn ends (`flush`). Writes
 * meanwhile, and while a run goes on, only add to what the next run carries, so a queued run is never run in a form
 * newer edits made stale: the run reads its owners' edits only when it starts.
 */
import { resolve } from "node:path";
import { type RrRun, type RrRunOutcome, runRerailPolicies } from "./rr-runner.js";

/** What an owner hands a run as it starts: the policies its files need, and where the run's outcome goes. */
export interface RrClaim {
  /** The rule files to run (absolute). */
  readonly policies: readonly string[];
  /** Called once, with the run's outcome; `killed` for `aborted` when the owner left the run before it ended. */
  settle(outcome: RrRunOutcome): void;
}

/** A session's checks of the checkout, as the queue sees them. */
export interface RrRunOwner {
  /** What the owner has gathered for a run starting now; undefined when it has nothing left to run. */
  claim(): RrClaim | undefined;
}

/** When a queue's runs start and how long each may take. */
export interface RrRunTiming {
  readonly deadlineMs: number;
  /** How long the checkout must go without a write before a queued run starts. */
  readonly quietMs: number;
}

type Running = { readonly claims: Map<RrRunOwner, RrClaim>; readonly controller: AbortController };

/** What a claim its owner left settles with: the run went on without it, or was killed for it. */
const LEFT: RrRunOutcome = { status: "killed", reason: "aborted", wallMs: 0, peakRssBytes: 0 };

/** Whether `outcome` is that of a run its owner left, or that was stopped on purpose: no outcome of its rules. */
export function leftRun(outcome: RrRunOutcome): boolean {
  return outcome.status === "killed" && outcome.reason === "aborted";
}

/** The queue of the checkout at `root`, running each run with `run`. */
export class RrRunQueue {
  readonly #root: string;
  readonly #timing: RrRunTiming;
  readonly #run: (run: RrRun) => Promise<RrRunOutcome>;
  readonly #waiting = new Set<RrRunOwner>();
  #running: Running | undefined;
  #timer: NodeJS.Timeout | undefined;
  #lastWriteAt = 0;
  /** Set when the turn ended with work waiting: the next run starts without waiting for quiet. */
  #flushed = false;

  constructor(root: string, timing: RrRunTiming, run: (run: RrRun) => Promise<RrRunOutcome> = runRerailPolicies) {
    this.#root = root;
    this.#timing = timing;
    this.#run = run;
  }

  /** The deadline each run of this queue is given. */
  get deadlineMs(): number {
    return this.#timing.deadlineMs;
  }

  /** `owner` wrote files it has gathered: it joins the next run, which starts once the checkout is quiet. */
  want(owner: RrRunOwner): void {
    this.#waiting.add(owner);
    this.#lastWriteAt = performance.now();
    this.#flushed = false;
    this.#schedule();
  }

  /** The agent's turn ended: a queued run starts now, or right after the one going on. */
  flush(): void {
    this.#flushed = true;
    this.#schedule();
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

  /** Sets the next run to start once quiet (or at once, flushed), unless one is going on or none is wanted. */
  #schedule(): void {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    if (this.#running !== undefined || this.#waiting.size === 0) return;
    const wait = this.#flushed ? 0 : this.#lastWriteAt + this.#timing.quietMs - performance.now();
    this.#timer = setTimeout(() => this.#startNext(), Math.max(0, wait));
    this.#timer.unref();
  }

  /** Starts a run with the claims of every waiting owner that still has work; none when no owner has. */
  #startNext(): void {
    this.#timer = undefined;
    const claims = new Map<RrRunOwner, RrClaim>();
    for (const owner of this.#waiting) {
      const claim = owner.claim();
      if (claim !== undefined) claims.set(owner, claim);
    }
    this.#waiting.clear();
    this.#flushed = false;
    if (claims.size === 0) return;
    const running = { claims, controller: new AbortController() };
    this.#running = running;
    void this.#runOnce(running).finally(() => {
      this.#running = undefined;
      this.#schedule();
    });
  }

  async #runOnce(running: Running): Promise<void> {
    const policies = [...new Set([...running.claims.values()].flatMap(claim => claim.policies))].sort();
    const run = { root: this.#root, policies, deadlineMs: this.#timing.deadlineMs, signal: running.controller.signal };
    const outcome = await this.#run(run).catch((error: unknown): RrRunOutcome => ({ status: "failed", message: String(error) }));
    for (const claim of running.claims.values()) claim.settle(outcome);
  }
}

const queues = new Map<string, RrRunQueue>();

/**
 * The queue of the checkout at `root` in this process, made on first use with `timing`; every session of this process
 * working in the checkout shares it.
 */
export function rrRunQueue(root: string, timing: RrRunTiming): RrRunQueue {
  const key = resolve(root);
  let queue = queues.get(key);
  if (queue === undefined) {
    queue = new RrRunQueue(key, timing);
    queues.set(key, queue);
  }
  return queue;
}
