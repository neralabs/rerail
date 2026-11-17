/**
 * The thread `waitForWriteLock` starts (`write-lock-wait.ts`). On a connection of its own it asks for the write lock of
 * the database named in `workerData` (`WriteLockWaitData`), letting SQLite's busy handler wait for the holder, gives the
 * lock back at once and posts `WRITE_LOCK_FREE`. Between tries, each as long as the busy handler waits for one
 * connection (better-sqlite3's default timeout), it ends quietly once the starting thread set `stop`.
 */
import Database from "better-sqlite3";
import { parentPort, workerData } from "node:worker_threads";
import { isBusy } from "./sqlite-wal.js";
import { WRITE_LOCK_FREE, type WriteLockWaitData } from "./write-lock-wait.js";

// `waitForWriteLock` starts this thread with a `WriteLockWaitData`.
const { path, stop } = workerData as WriteLockWaitData;

/** Whether the write lock was taken and given back; false when another connection still held it after one wait. */
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

const database = new Database(path, { fileMustExist: true });
try {
  let free = false;
  while (!free && Atomics.load(stop, 0) === 0) free = takeAndGiveBack(database);
  if (free) parentPort?.postMessage(WRITE_LOCK_FREE);
} finally {
  database.close();
}
