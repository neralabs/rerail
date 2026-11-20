/**
 * At most one `.rerail` child process at a time in this process, whatever started it: a checkout's run after edits
 * (`rr-run-queue.ts`), an audit's run (`rr-audit-run.ts`), or runs of two checkouts. Each child may take half the
 * machine's memory and threads (`rr-budgets.ts`), so two side by side could take all of it. Runs wait their turn in
 * the order they asked; a run's deadline counts from its child's start, not from its wait, and a run aborted while it
 * waits starts no child.
 */
import { type RrRun, type RrRunOutcome, runRerailPolicies } from "./rr-runner.js";

/** What a run aborted before its turn came gives: no child ran. */
const ABORTED_WAITING: RrRunOutcome = { status: "killed", reason: "aborted", wallMs: 0, peakRssBytes: 0 };

/** Runs one run at a time with `run`, in the order they are asked for. */
export class RrRunSlots {
  readonly #run: (run: RrRun) => Promise<RrRunOutcome>;
  /** The last run asked for, settled once it has ended: the next waits for it. */
  #tail: Promise<unknown> = Promise.resolve();

  constructor(run: (run: RrRun) => Promise<RrRunOutcome> = runRerailPolicies) {
    this.#run = run;
  }

  /**
   * `run`, once every run asked for before it has ended. A run aborted while it waits settles at once and starts no
   * child; the runs behind it still wait for the run ahead of it.
   */
  async run(run: RrRun): Promise<RrRunOutcome> {
    const { promise: aborted, resolve: abort } = Promise.withResolvers<RrRunOutcome>();
    run.signal?.addEventListener("abort", () => abort(ABORTED_WAITING), { once: true });
    const turn = this.#tail.then(() => (run.signal?.aborted === true ? ABORTED_WAITING : this.#run(run)));
    this.#tail = turn.catch(() => undefined);
    return await Promise.race([turn, aborted]);
  }
}

const slots = new RrRunSlots();

/** `runRerailPolicies`, one child at a time in this process. */
export function runInTurn(run: RrRun): Promise<RrRunOutcome> {
  return slots.run(run);
}
