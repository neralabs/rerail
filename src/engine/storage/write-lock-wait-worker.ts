/**
 * The thread `waitForWriteLock` starts (`write-lock-wait.ts`). On a connection of its own it tries for the write lock
 * of the database named in `workerData` (`WriteLockWaitData`), gives it back at once and posts `WRITE_LOCK_FREE`. A try
 * never waits for the lock. The tries come at SQLite's own busy handler's times, counted from when the caller met the
 * lock, and a change of the database's write-ahead log, which a holder that changed the database makes as it commits,
 * starts those times again from the change, so the try after it comes a millisecond later, not at the next of the
 * handler's 100 ms pauses. The starting thread's stop (or the process's exit) wakes it at once.
 */
import { type FSWatcher, watch } from "node:fs";
import Database from "better-sqlite3";
import { parentPort, workerData } from "node:worker_threads";
import { isBusy } from "./sqlite-wal.js";
import { busyTryAtMs, epochNow, WRITE_LOCK_FREE, type WriteLockWaitData } from "./write-lock-wait.js";

// `waitForWriteLock` starts this thread with a `WriteLockWaitData`.
const { path, stop, since } = workerData as WriteLockWaitData;

/** Whether the write lock was taken and given back; false while another connection holds it. */
function takeAndGiveBack(database: Database.Database): boolean {
  try {
    database.exec("BEGIN IMMEDIATE");
    database.exec("ROLLBACK");
    return true;
  } catch (error) {
    if (isBusy(error)) return false;
    throw error;
  }
}

/** When the tries come: the busy handler's times from `from`, the caller's meeting of the lock or the log's last change. */
class TryTimes {
  #from: number;
  #tries = 0;

  constructor(from: number) {
    this.#from = from;
  }

  /** The log changed: the handler's times start again from now, the try at once being the change itself. */
  restart(): void {
    this.#from = epochNow();
    this.#tries = 0;
  }

  /** Ms until the first try due after now: a thread late for some (its start, a busy machine) skips them. */
  untilNext(): number {
    const now = epochNow();
    while (this.#from + busyTryAtMs(this.#tries) <= now) this.#tries += 1;
    return this.#from + busyTryAtMs(this.#tries) - now;
  }
}

const times = new TryTimes(since);
/** Sets the pause in progress again to the next try's time; ends it with `true`. */
let pauseAgain: (end?: boolean) => void = () => undefined;

/** Ends at the next try's time, which a change of the log moves (`pauseAgain`), or at once at the stop. */
function pause(): Promise<void> {
  return new Promise(resolve => {
    let timer = setTimeout(ended, times.untilNext());
    function ended(): void {
      clearTimeout(timer);
      pauseAgain = () => undefined;
      resolve();
    }
    pauseAgain = end => {
      clearTimeout(timer);
      timer = end === true ? setTimeout(ended, 0) : setTimeout(ended, times.untilNext());
    };
  });
}

/**
 * Watches the database's write-ahead log, whose change tells of a holder's commit. None where the system cannot watch
 * it: the tries then keep the busy handler's times alone, as a holder that changed nothing leaves them.
 */
function watchLog(): FSWatcher | undefined {
  try {
    const watcher = watch(`${path}-wal`, { persistent: false }, () => {
      times.restart();
      pauseAgain();
    });
    watcher.on("error", () => watcher.close());
    return watcher;
  } catch {
    // No log there yet, or no watch on this file system.
    return undefined;
  }
}

const database = new Database(path, { fileMustExist: true, timeout: 0 });
const watcher = watchLog();
const stopping = Atomics.waitAsync(stop, 0, 0);
if (stopping.async) void stopping.value.then(() => pauseAgain(true));
try {
  while (Atomics.load(stop, 0) === 0) {
    if (takeAndGiveBack(database)) {
      parentPort?.postMessage(WRITE_LOCK_FREE);
      break;
    }
    await pause();
  }
} finally {
  watcher?.close();
  database.close();
  // Ends the wait on the stop word above, which the loop's end would otherwise leave pending.
  Atomics.store(stop, 0, 1);
  Atomics.notify(stop, 0);
}
