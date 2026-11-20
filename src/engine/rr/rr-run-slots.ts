/**
 * At most one `.rerail` child process at a time in this process, whatever started it: a checkout's run after edits
 * (`rr-run-queue.ts`), an audit's run (`rr-audit-run.ts`), or runs of two checkouts. Each child may take half the
 * machine's memory and threads (`rr-budgets.ts`), so two side by side could take all of it. Runs wait their turn in
 * the order they asked; a run's deadline counts from its child's start, not from its wait, and a run aborted while it
 * waits starts no child.
 */
import { type RrRun, type RrRunOutcome, runRerailPolicies } from "./rr-runner.js";

/** How a run ended for its owner: the runner's outcome, or `left` when the owner left it before it ended or began. */
export type RrRunEnd = RrRunOutcome | { status: "left" };

/** A run its owner left: no child ran for it, or the one that did gave it nothing. */
export const LEFT_RUN: RrRunEnd = { status: "left" };

/** Whether `end` is that of a run its owner left, or whose child was stopped on purpose: no outcome of its rules. */
export function leftRun(end: RrRunEnd): end is { status: "left" } | Extract<RrRunOutcome, { status: "killed" }> & { reason: "aborted" } {
  return end.status === "left" || (end.status === "killed" && end.reason === "aborted");
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
   * `run`, once every run asked for before it has ended. A run aborted when asked for, or while it waits, settles at
   * once as `left` and starts no child; the runs behind it still wait for the run ahead of it.
   */
  async run(run: RrRun): Promise<RrRunEnd> {
    // An abort listener never fires for a signal aborted already.
    if (run.signal?.aborted === true) return LEFT_RUN;
    const { promise: aborted, resolve: abort } = Promise.withResolvers<RrRunEnd>();
    run.signal?.addEventListener("abort", () => abort(LEFT_RUN), { once: true });
    const turn = this.#tail.then(() => (run.signal?.aborted === true ? LEFT_RUN : this.#run(run)));
    this.#tail = turn.catch(() => undefined);
    return await Promise.race([turn, aborted]);
  }
}

const slots = new RrRunSlots();

/** `runRerailPolicies`, one child at a time in this process. */
export function runInTurn(run: RrRun): Promise<RrRunEnd> {
  return slots.run(run);
}
