/**
 * Waits off the calling thread for another process's write lock on a Rerail database to be given back. SQLite tells a
 * waiter in another process nothing when a holder commits, and the lock's release leaves nothing on disk. What does:
 * a holder that changed the database adds its pages to the write-ahead log as it commits, just before it gives the lock
 * back, so the log's change wakes the wait at once. A holder that changed nothing leaves only trying again, as SQLite's
 * busy handler does, at that handler's own times. That runs here on a worker thread of its own, so the thread that
 * answers hooks never sleeps in it (`record-writes.ts`, and a quiet session's reopen in `engine-service.ts`).
 */
import { join, resolve } from "node:path";
import { Worker } from "node:worker_threads";
import { PACKAGE_ROOT } from "../package-root.js";

/** The worker's JavaScript in the package, relative to its root: tsc writes it there from `src/engine/storage/`. */
const LOCK_WAIT_WORKER = "dist/engine/storage/write-lock-wait-worker.js";

/** What the thread `waitForWriteLock` starts reads (`write-lock-wait-worker.ts`). */
export interface WriteLockWaitData {
  readonly path: string;
  /**
   * Set to 1 and notified (`Atomics.notify`) by the starting thread to stop the wait before the lock frees: the thread
   * waits between tries on this word too, so the stop wakes it at once.
   */
  readonly stop: Int32Array;
  /**
   * When the wait's caller met the held lock (`epochNow`). The thread tries at the busy handler's times counted from
   * then, as the caller's own statement would have, not from its own start some milliseconds later: a holder that gives
   * the lock back at a given time is found then.
   */
  readonly since: number;
  /** The starting thread's turn at the database's write lock, which each try takes first (`apartFromLockWaits`). */
  readonly turn: Int32Array;
}

/**
 * The values of a turn's word: no one takes the lock through it; a wait's thread tries the lock; the starting thread
 * waits for that try to end, which wakes it (`Atomics.notify`); the starting thread tries the lock itself.
 */
export const TURN_FREE = 0;
export const TURN_WAIT_TRIES = 1;
export const TURN_THREAD_ASKS = 2;
export const TURN_THREAD_TRIES = 3;

/** This thread's turn at each database's write lock, by its resolved path, shared with every wait it starts there. */
const turns = new Map<string, Int32Array>();

function turnOf(path: string): Int32Array {
  const key = resolve(path);
  let turn = turns.get(key);
  if (turn === undefined) {
    turn = new Int32Array(new SharedArrayBuffer(4));
    turns.set(key, turn);
  }
  return turn;
}

/**
 * Runs `attempt`, this thread's own try for the write lock of the database at `path`, with no try of a wait this thread
 * started (`waitForWriteLock`) holding the lock meanwhile: one in progress, two statements that never wait, ends first,
 * and none begins until `attempt` returns. A wait's try takes the lock as any connection does, so before, a try of this
 * thread's that came at that moment failed as another process's holder fails it, where SQLite runs no busy handler: a
 * write whose waiting time is spent (`record-writes.ts`), and a transaction that read before it wrote (the session's
 * owner claim). A try in progress here makes the wait's next try fail as a held lock does, so it tries again later.
 */
export function apartFromLockWaits<T>(path: string, attempt: () => T): T {
  const turn = turnOf(path);
  for (;;) {
    const was = Atomics.compareExchange(turn, 0, TURN_FREE, TURN_THREAD_TRIES);
    if (was === TURN_FREE) break;
    // This thread's try within its own: the turn is already this thread's.
    if (was === TURN_THREAD_TRIES) return attempt();
    // A wait's try: marked asked, so its end wakes this thread. One that ended meanwhile leaves the word free.
    Atomics.compareExchange(turn, 0, TURN_WAIT_TRIES, TURN_THREAD_ASKS);
    Atomics.wait(turn, 0, TURN_THREAD_ASKS);
  }
  try {
    return attempt();
  } finally {
    Atomics.store(turn, 0, TURN_FREE);
  }
}

/** What the thread posts once it took the write lock and gave it back at once. */
export const WRITE_LOCK_FREE = "write-lock-free";

/** The pauses of SQLite's default busy handler (`sqliteDefaultBusyCallback`), in ms; the last one repeats. */
const BUSY_PAUSES_MS = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];
const LAST_BUSY_PAUSE_MS = 100;
/** The pauses above summed: when the handler makes the try that follows the last of them. */
const BUSY_PAUSES_SUM_MS = BUSY_PAUSES_MS.reduce((sum, pause) => sum + pause, 0);

/** The pause SQLite's busy handler makes after try `tries + 1` failed on the lock, in ms. */
export function busyPauseMs(tries: number): number {
  return BUSY_PAUSES_MS[tries] ?? LAST_BUSY_PAUSE_MS;
}

/** When SQLite's busy handler makes try `tries` (0, the first, at once), in ms from the first: the pauses before it summed. */
export function busyTryAtMs(tries: number): number {
  if (tries >= BUSY_PAUSES_MS.length) return BUSY_PAUSES_SUM_MS + (tries - BUSY_PAUSES_MS.length) * LAST_BUSY_PAUSE_MS;
  return BUSY_PAUSES_MS.slice(0, tries).reduce((sum, pause) => sum + pause, 0);
}

/** Now, in ms, on a clock every thread of the process reads alike: `performance.now()` counts from each thread's start. */
export function epochNow(): number {
  return performance.timeOrigin + performance.now();
}

/**
 * Resolves to true once the write lock of the database at `path` was free (taken and given back at once on a
 * connection of the worker's own), or to false once `signal` aborted first and the thread ended. `since` is when the
 * caller met the held lock (`WriteLockWaitData.since`). The worker never keeps the process alive, and its stop never
 * waits for a try in progress: a try does not wait for the lock.
 */
export function waitForWriteLock(path: string, signal?: AbortSignal, since = epochNow()): Promise<boolean> {
  const stop = new Int32Array(new SharedArrayBuffer(4));
  // A worker that cannot start (a package that lost its file) rejects, as one that fails later does.
  return new Promise<boolean>((resolve, reject) => {
    const worker = lockWaitWorker({ path, stop, since, turn: turnOf(path) });
    worker.unref();
    const onAbort = (): void => {
      Atomics.store(stop, 0, 1);
      Atomics.notify(stop, 0);
    };
    if (signal?.aborted === true) onAbort();
    signal?.addEventListener("abort", onAbort, { once: true });
    worker.once("message", (message: unknown) => resolve(message === WRITE_LOCK_FREE));
    worker.once("error", reject);
    worker.once("exit", () => {
      signal?.removeEventListener("abort", onAbort);
      resolve(false);
    });
  });
}

/**
 * The worker (`write-lock-wait-worker.ts`): its JavaScript where the package ships it, found from the package's root as
 * its entry points are, since the bundles of the CLI, the hooks and the event stream inline this module in another
 * folder; or, where this module runs from source (tests), the TypeScript beside it through tsx, as
 * `rule-provenance-move.ts` starts its own.
 */
function lockWaitWorker(data: WriteLockWaitData): Worker {
  if (!import.meta.url.endsWith(".ts")) return new Worker(join(PACKAGE_ROOT, LOCK_WAIT_WORKER), { workerData: data });
  return new Worker(new URL("./write-lock-wait-worker.ts", import.meta.url), { workerData: data, execArgv: ["--import", "tsx"] });
}
