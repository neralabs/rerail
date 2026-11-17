/**
 * Waits off the calling thread for another process's write lock on a Rerail database to be given back. SQLite tells a
 * waiter in another process nothing when a holder commits: the only notice is its busy handler, which sleeps and tries
 * again until the lock is free. That handler runs here on a worker thread of its own, so the thread that answers hooks
 * never sleeps in it (`record-writes.ts`, and a quiet session's reopen in `engine-service.ts`).
 */
import { Worker } from "node:worker_threads";

/** What the thread `waitForWriteLock` starts reads (`write-lock-wait-worker.ts`). */
export interface WriteLockWaitData {
  readonly path: string;
  /** Set to 1 by the starting thread to stop the wait before the lock frees; the thread reads it between tries. */
  readonly stop: Int32Array;
}

/** What the thread posts once it took the write lock and gave it back at once. */
export const WRITE_LOCK_FREE = "write-lock-free";

/**
 * Resolves to true once the write lock of the database at `path` was free (taken and given back at once on a
 * connection of the worker's own), or to false once `signal` aborts first. The worker never keeps the process alive.
 */
export function waitForWriteLock(path: string, signal?: AbortSignal): Promise<boolean> {
  const stop = new Int32Array(new SharedArrayBuffer(4));
  // A worker that cannot start (a bundle that left its file behind) rejects, as one that fails later does.
  return new Promise<boolean>((resolve, reject) => {
    const worker = lockWaitWorker({ path, stop });
    worker.unref();
    const onAbort = (): void => {
      Atomics.store(stop, 0, 1);
      resolve(false);
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
 * The worker beside this module (`write-lock-wait-worker.ts`): its JavaScript as the package ships it, or, where this
 * module runs from source (tests), the TypeScript through tsx, as `rule-provenance-move.ts` starts its own.
 */
function lockWaitWorker(data: WriteLockWaitData): Worker {
  const fromSource = import.meta.url.endsWith(".ts");
  const entry = new URL(`./write-lock-wait-worker.${fromSource ? "ts" : "js"}`, import.meta.url);
  return new Worker(entry, { workerData: data, ...(fromSource ? { execArgv: ["--import", "tsx"] } : {}) });
}
