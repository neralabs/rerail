/**
 * One `.rerail` run at a time per checkout, for the edits of every session working in it (plan decision 10). Each
 * session's checks of the checkout are an owner: it says it has work (`want`), and when a run's child is about to
 * start it hands over what it has gathered by then (`claim`).
 *
 * A run costs tens of seconds and gigabytes, so it does not start on the first write of a burst: it asks for its turn
 * once the checkout has had no further write for the quiet period, or at once when the agent's turn ends (`flush`).
 * Its turn may wait for another run of the process (`rr-run-slots.ts`). Writes meanwhile, while it waits and while
 * another run goes on, only add to what a run carries: owners are claimed when the turn comes, so a run never carries
 * an edit a newer one already superseded.
 */
import { resolve } from "node:path";
import type { RrRun } from "./rr-runner.js";
import { LEFT_RUN, PROCESS_SLOTS, type RrRunEnd, type RrRunSlots } from "./rr-run-slots.js";

/** What an owner hands a run as its child is about to start: the policies its files need, and where its end goes. */
export interface RrClaim {
  /** The rule files to run (absolute); none when every rule its files take is left out of this run. */
  readonly policies: readonly string[];
  /** Called once, with how the run ended; `left` when the owner left the run before it ended. */
  settle(end: RrRunEnd): void;
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

/** A run of the queue: no owner's claim until its turn comes, then the owners it carries. */
type Running = { claims: Map<RrRunOwner, RrClaim>; readonly controller: AbortController };

/** The queue of the checkout at `root`, running each run in its turn among `slots`. */
export class RrRunQueue {
  readonly #root: string;
  readonly #timing: RrRunTiming;
  readonly #slots: RrRunSlots;
  readonly #waiting = new Set<RrRunOwner>();
  #running: Running | undefined;
  #timer: NodeJS.Timeout | undefined;
  #lastWriteAt = 0;
  /** Set when the turn ended with work waiting: the next run starts without waiting for quiet. */
  #flushed = false;

  constructor(root: string, timing: RrRunTiming, slots: RrRunSlots = PROCESS_SLOTS) {
    this.#root = root;
    this.#timing = timing;
    this.#slots = slots;
  }

  /** `owner` wrote files it has gathered: it joins the next run, which starts once the checkout is quiet. */
  want(owner: RrRunOwner): void {
    this.#waiting.add(owner);
    this.#lastWriteAt = performance.now();
    this.#flushed = false;
    this.#schedule();
  }

  /** The agent's turn ended: a queued run asks for its turn now, or right after the one going on. */
  flush(): void {
    this.#flushed = true;
    this.#schedule();
  }

  /**
   * `owner` leaves: it joins no later run, nor a run still waiting for its turn, and a run it is in settles its claim
   * `left` now. The running child is killed when no other owner is left in it.
   */
  leave(owner: RrRunOwner): void {
    this.#waiting.delete(owner);
    const running = this.#running;
    const claim = running?.claims.get(owner);
    if (running === undefined || claim === undefined) return;
    running.claims.delete(owner);
    claim.settle(LEFT_RUN);
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

  /**
   * Asks for the next run's turn. The owners that want a run when the turn comes are the ones it carries (`#take`);
   * another run is set only once this one has ended.
   */
  #startNext(): void {
    this.#timer = undefined;
    this.#flushed = false;
    const running: Running = { claims: new Map(), controller: new AbortController() };
    this.#running = running;
    void this.#runOnce(running)
      .catch(error => report("a .rerail run after edits failed", error))
      .finally(() => {
        this.#running = undefined;
        this.#schedule();
      });
  }

  async #runOnce(running: Running): Promise<void> {
    const turn = { signal: running.controller.signal, start: () => this.#take(running) };
    const outcome = await this.#slots.run(turn).catch((error: unknown): RrRunEnd => ({ status: "failed", message: String(error) }));
    // Each claim settles on its own: one that throws leaves the others' audits to finish.
    for (const claim of running.claims.values()) {
      try {
        claim.settle(outcome);
      } catch (error) {
        report("a .rerail run's outcome was not recorded", error);
      }
    }
  }

  /**
   * `running`'s turn came: every waiting owner's claim of what it has gathered by now, and the run; undefined when no
   * claim leaves a policy to run.
   */
  #take(running: Running): RrRun | undefined {
    running.claims = this.#claims();
    this.#waiting.clear();
    const policies = [...new Set([...running.claims.values()].flatMap(claim => claim.policies))].sort();
    if (policies.length === 0) return undefined;
    return { root: this.#root, policies, deadlineMs: this.#timing.deadlineMs, signal: running.controller.signal };
  }

  /** What each waiting owner hands the run; an owner whose claim throws is left out of it, its error logged. */
  #claims(): Map<RrRunOwner, RrClaim> {
    const claims = new Map<RrRunOwner, RrClaim>();
    for (const owner of this.#waiting) {
      try {
        const claim = owner.claim();
        if (claim !== undefined) claims.set(owner, claim);
      } catch (error) {
        report("a session's edits could not join a .rerail run", error);
      }
    }
    return claims;
  }
}

/** Writes `what` and `error` to the daemon's log. */
function report(what: string, error: unknown): void {
  process.stderr.write(`rerail: ${what}: ${error instanceof Error ? error.message : String(error)}\n`);
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
