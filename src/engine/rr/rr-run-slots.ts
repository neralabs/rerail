/**
 * At most one `.rerail` child process at a time in this process, whatever started it: a checkout's run after edits
 * (`rr-run-queue.ts`), an audit's run (`rr-audit-run.ts`), or runs of two checkouts. Each child may take half the
 * machine's memory and threads (`rr-budgets.ts`), so two side by side could take all of it. Runs wait their turn in
 * the order they asked. A run is made only when its turn comes (`RrTurn.start`), so what it carries is decided then,
 * not when it asked; its deadline counts from its child's start; and a run aborted before its turn starts no child.
 */
import { type RrRun, type RrRunOutcome, runRerailPolicies } from "./rr-runner.js";

/** How a run ended for its owner: the runner's outcome, or `left` when the owner left it before it ended or began. */
export type RrRunEnd = RrRunOutcome | { status: "left" };

/** A run its owner left: no child ran for it, or the one that did gave it nothing. */
export const LEFT_RUN: RrRunEnd = { status: "left" };

/** The outcome of a turn with nothing left to run: nothing found, no child started. */
export const NOTHING_RUN: RrRunOutcome = { status: "completed", findings: [], failures: [], files: 0, wallMs: 0, peakRssBytes: 0 };

/** Whether `end` is that of a run its owner left, or whose child was stopped on purpose: no outcome of its rules. */
export function leftRun(end: RrRunEnd): end is { status: "left" } | Extract<RrRunOutcome, { status: "killed" }> & { reason: "aborted" } {
  return end.status === "left" || (end.status === "killed" && end.reason === "aborted");
}

/** A run waiting for its turn: what stops it, and how to make it once its turn comes (undefined: nothing to run). */
export interface RrTurn {
  readonly signal?: AbortSignal | undefined;
  start(): RrRun | undefined;
}

/** Runs one run at a time with `run`, in the order they are asked for. */
export class RrRunSlots {
  readonly #run: (run: RrRun) => Promise<RrRunOutcome>;
  /** The last run asked for, settled once it has ended: the next waits for it. */
  #tail: Promise<unknown> = Promise.resolve();

  constructor(run: (run: RrRun) => Promise<RrRunOutcome> = runRerailPolicies) {
    this.#run = run;
  }

  /**
   * `turn`'s run, made and started once every run asked for before it has ended. A turn aborted before then settles
   * at once as `left` and starts no child; the runs behind it still wait for the run ahead of it.
   */
  async run(turn: RrTurn): Promise<RrRunEnd> {
    if (turn.signal?.aborted === true) return LEFT_RUN;
    const { promise: aborted, resolve: abort } = Promise.withResolvers<RrRunEnd>();
    turn.signal?.addEventListener("abort", () => abort(LEFT_RUN), { once: true });
    const taken = this.#tail.then(() => this.#take(turn));
    this.#tail = taken.catch(() => undefined);
    return await Promise.race([taken, aborted]);
  }

  /** `turn`'s run, its turn come: none when it was aborted meanwhile or has nothing left to run. */
  async #take(turn: RrTurn): Promise<RrRunEnd> {
    if (turn.signal?.aborted === true) return LEFT_RUN;
    const run = turn.start();
    return run === undefined ? NOTHING_RUN : await this.#run(run);
  }
}

/** The slots of this process: every `.rerail` run of the process takes its turn here. */
export const PROCESS_SLOTS = new RrRunSlots();
