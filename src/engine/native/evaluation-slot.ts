import { isWorkspaceBusy } from "./native.js";

/**
 * The native engine evaluates one workspace apply or repository index chunk at a time across the whole process (its
 * CPU slot), and refuses a call that finds the slot held. The calls of this process take turns here first, in the
 * order they ask, so each reaches the engine once the call before it has finished: a check that arrives during another
 * session's repository scan runs after the scan's chunk in progress, never after the whole scan. A call waiting its
 * turn is a pending promise: it holds no libuv pool thread, and its turn comes from the end of the call before it,
 * never from a clock.
 *
 * A scan's first chunk first plans the whole index (a walk of the checkout and the store's reuse) without the engine's
 * slot, so it runs beside the line (`runBeside`): a check that arrives while the scan plans runs at once, as it would
 * with no scan. The slot the first chunk then takes for its batch can refuse the call holding the line; that call
 * waits for the calls beside the line to settle and asks again, still holding the line.
 */
export class EvaluationSlot {
  #held = false;
  readonly #waiting: Array<() => void> = [];
  /** The calls beside the line, each as a promise that settles with it; and how many have settled so far. */
  readonly #beside = new Set<Promise<void>>();
  #besideEnded = 0;
  readonly #refused: (error: unknown) => boolean;

  /** `refused`: whether an engine call failed only because another call held the engine's slot. */
  constructor(refused: (error: unknown) => boolean) {
    this.#refused = refused;
  }

  /** Whether no call holds the slot or waits for it, so a synchronous inline apply now overtakes no one. */
  get idle(): boolean {
    return !this.#held && this.#waiting.length === 0;
  }

  /**
   * Runs `task` once every call that asked before it has finished, and frees the slot when it settles. A `signal`
   * aborted before the turn comes takes the call out of the line: it rejects with the signal's reason and never runs.
   */
  async run<T>(task: () => Promise<T>, signal: AbortSignal): Promise<T> {
    signal.throwIfAborted();
    await this.#turn(signal);
    try {
      return await this.#untilBesideEnds(task);
    } finally {
      this.#next();
    }
  }

  /**
   * Runs `task` at once, beside the line, for an engine call that first works without the engine's slot; refused the
   * slot after that work, it asks again at its turn in the line.
   */
  async runBeside<T>(task: () => Promise<T>, signal: AbortSignal): Promise<T> {
    signal.throwIfAborted();
    const call = task();
    const settled: Promise<void> = call.then(
      () => this.#endBeside(settled),
      () => this.#endBeside(settled),
    );
    this.#beside.add(settled);
    try {
      return await call;
    } catch (error) {
      if (!this.#refused(error)) throw error;
      return await this.run(task, signal);
    }
  }

  #endBeside(settled: Promise<void>): void {
    this.#beside.delete(settled);
    this.#besideEnded += 1;
  }

  /** `task`, asked again each time a call beside the line held the engine's slot and has since settled. */
  async #untilBesideEnds<T>(task: () => Promise<T>): Promise<T> {
    for (;;) {
      const ended = this.#besideEnded;
      try {
        return await task();
      } catch (error) {
        if (!this.#mayAskAgain(error, ended)) throw error;
        if (this.#beside.size > 0) await Promise.race(this.#beside);
      }
    }
  }

  /**
   * Whether a refused call may ask again: a call beside the line is still running, or one ended since this call asked.
   * Any other refusal (a holder outside these calls) is the caller's error, never a loop.
   */
  #mayAskAgain(error: unknown, ended: number): boolean {
    return this.#refused(error) && (this.#beside.size > 0 || this.#besideEnded !== ended);
  }

  #turn(signal: AbortSignal): Promise<void> {
    if (!this.#held) {
      this.#held = true;
      return Promise.resolve();
    }
    return new Promise((resolve, reject) => {
      const take = (): void => {
        signal.removeEventListener("abort", leave);
        resolve();
      };
      const leave = (): void => {
        const place = this.#waiting.indexOf(take);
        if (place >= 0) this.#waiting.splice(place, 1);
        reject(signal.reason);
      };
      this.#waiting.push(take);
      signal.addEventListener("abort", leave, { once: true });
    });
  }

  /** Hands the slot to the first call waiting, or frees it when none waits. */
  #next(): void {
    const take = this.#waiting.shift();
    if (take === undefined) this.#held = false;
    else take();
  }
}

/** The one line of this process, shared by every session's evaluator, as the engine's slot is. */
export const evaluationSlot = new EvaluationSlot(isWorkspaceBusy);
