/**
 * The thread `waitForWriteLock` starts (`write-lock-wait.ts`). On a connection of its own it tries for the write lock
 * of the database named in `workerData` (`WriteLockWaitData`), gives it back at once and posts `WRITE_LOCK_FREE`. A try
 * never waits for the lock; between tries the thread sleeps on the `stop` word for SQLite's own busy handler's pauses,
 * so it wakes as often as that handler did, and the starting thread's stop (or the process's exit) wakes it at once.
 */
import Database from "better-sqlite3";
import { parentPort, workerData } from "node:worker_threads";
import { isBusy } from "./sqlite-wal.js";
import { busyPauseMs, WRITE_LOCK_FREE, type WriteLockWaitData } from "./write-lock-wait.js";

// `waitForWriteLock` starts this thread with a `WriteLockWaitData`.
const { path, stop } = workerData as WriteLockWaitData;

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

const database = new Database(path, { fileMustExist: true, timeout: 0 });
try {
  for (let tries = 0; Atomics.load(stop, 0) === 0; tries += 1) {
    if (takeAndGiveBack(database)) {
      parentPort?.postMessage(WRITE_LOCK_FREE);
      break;
    }
    Atomics.wait(stop, 0, 0, busyPauseMs(tries));
  }
} finally {
  database.close();
}
