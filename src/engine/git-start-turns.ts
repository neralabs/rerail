/**
 * Git processes start one per turn of the event loop. Starting a process holds the thread that starts it: libuv forks
 * on Linux and macOS, and a fork copies the page tables of the whole process, about 30 ms for each GB it holds; on
 * Windows it calls CreateProcess, 15 to 40 ms. Reads asked for together (a checkout's Git view, the verdicts of a walk,
 * each session walking the other worktrees) would start all their processes in one turn, and every socket of the
 * process would wait for all of them: in the daemon, a hook's health probe and event and the dashboard's reads, which a
 * hook gives up on when they take too long, losing its event. Each start waits for a turn of its own instead, after the
 * loop has polled for I/O, so the loop answers its sockets between any two starts.
 *
 * Starts go in the order they were asked for, except that work nothing waits for (the walk of the other worktrees,
 * `inGitBackground`) lets the starts of everything else go first, a hook's own reads among them. It still gets one
 * start in every `BACKGROUND_SHARE` while others wait, so it never stops. Once something waits for it, by queueing
 * behind it in an `OperationQueue`, its starts go in turn with everyone else's.
 */
import { AsyncLocalStorage } from "node:async_hooks";

/** One start in this many goes to background work while other starts wait too. */
export const BACKGROUND_SHARE = 4;

/** Work run by `inGitBackground`, or an operation it queued, which nothing waits for until `waitedFor` is set. */
interface BackgroundWork {
  waitedFor: boolean;
}

/** A start waiting for its turn, and the background work that asked for it, if any. */
interface WaitingStart {
  readonly start: () => void;
  readonly work: BackgroundWork | undefined;
}

const background = new AsyncLocalStorage<BackgroundWork>();
const waiting: WaitingStart[] = [];
let scheduled = false;
/** Starts let go since the last one of background work nothing waited for. */
let sinceBackground = 0;

/**
 * Runs `work` as work nothing waits for: the Git processes it starts, and those its continuations start, wait behind
 * everyone else's (see the module comment).
 */
export function inGitBackground<T>(work: () => T): T {
  return background.run({ waitedFor: false }, work);
}

/** Whether the caller runs as background work that nothing waits for yet. */
function inUnwaitedWork(): boolean {
  return background.getStore()?.waitedFor === false;
}

/** Whether `start` lets everyone else's go first: background work nothing waits for asked for it. */
function behind(start: WaitingStart): boolean {
  return start.work?.waitedFor === false;
}

/** Resolves in a turn of the event loop in which no other Git process has started; start the process then. */
export async function gitStartTurn(): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  waiting.push({ start: resolve, work: background.getStore() });
  if (!scheduled) {
    scheduled = true;
    setImmediate(startNext);
  }
  await promise;
}

/**
 * Lets one waiting start go in the loop's check phase, which follows its poll for I/O. The start runs as this
 * callback's continuation, before any other check-phase callback; the next one waits for the next check phase.
 */
function startNext(): void {
  const next = takeNext();
  if (next !== undefined) {
    sinceBackground = behind(next) ? 0 : sinceBackground + 1;
    next.start();
  }
  scheduled = waiting.length > 0;
  if (scheduled) setImmediate(startNext);
}

/**
 * The waiting start to let go now, taken off the line: the first asked for, but one that lets everyone else's go first
 * only while no other waits, or when it is owed one start in every `BACKGROUND_SHARE`. A start of background work
 * that came to be waited for keeps the place it was asked for in.
 */
function takeNext(): WaitingStart | undefined {
  const other = waiting.findIndex(start => !behind(start));
  const owed = other < 0 || sinceBackground >= BACKGROUND_SHARE - 1;
  const backgroundAt = owed ? waiting.findIndex(behind) : -1;
  const at = backgroundAt >= 0 ? backgroundAt : other;
  return at >= 0 ? waiting.splice(at, 1)[0] : undefined;
}

/**
 * Operations run one at a time, each once every operation queued before it has settled (a session's queue of
 * operations), so whoever queues one waits behind all of those. An operation that background work queues is background
 * work of its own: its Git processes start behind everyone else's while nothing waits for it, so a hook's own reads
 * elsewhere go first. Once a caller that is not background work queues behind it (`run`) or waits for the queue to
 * settle (`settled`), it starts its Git in turn with everyone else's, so that caller never waits at background pace.
 */
export class OperationQueue {
  #last: Promise<void> = Promise.resolve();
  /** The operations background work queued that nothing waits behind yet. */
  readonly #unwaited = new Set<BackgroundWork>();

  /** Runs `operation` once every operation queued before it has settled, and settles as it does. */
  async run<T>(operation: () => Promise<T>): Promise<T> {
    if (!inUnwaitedWork()) {
      this.#waitBehind();
      return await this.#append(operation);
    }
    // Background work of its own, so that waiting behind this operation leaves the rest of the work that queued it
    // behind everyone else's.
    const work: BackgroundWork = { waitedFor: false };
    this.#unwaited.add(work);
    try {
      return await this.#append(async () => await background.run(work, operation));
    } finally {
      this.#unwaited.delete(work);
    }
  }

  /** Resolves once every operation queued so far has settled. */
  async settled(): Promise<void> {
    if (!inUnwaitedWork()) this.#waitBehind();
    await this.#last;
  }

  async #append<T>(operation: () => Promise<T>): Promise<T> {
    const pending = this.#last.then(operation, operation);
    this.#last = pending.then(() => undefined, () => undefined);
    return await pending;
  }

  /** The caller waits behind every operation queued so far, so each starts its Git in turn with everyone else's. */
  #waitBehind(): void {
    for (const work of this.#unwaited) work.waitedFor = true;
    this.#unwaited.clear();
  }
}
