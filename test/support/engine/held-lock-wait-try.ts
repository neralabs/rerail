/**
 * Loaded with `--import` into a write lock wait's thread (`write-lock-wait-worker.ts`) by a test: the first of its tries
 * that takes the lock holds it there, as a thread the system stops between its two statements holds it, until the test
 * lets it go or the starting thread asks for the turn that try holds (`apartFromLockWaits`), which changes the turn's
 * word. The test's word, `workerData.heldTry`: [0] is set to 1 and notified once the try holds the lock; the test sets
 * [1] to let it go.
 */
import Database from "better-sqlite3";
import { workerData } from "node:worker_threads";

const { heldTry, turn } = workerData as { heldTry?: Int32Array; turn?: Int32Array };
const exec = Database.prototype.exec;

/** Holds the try until the test's word [1] is set or the turn's word changes from what the try made it. */
function hold(word: Int32Array): void {
  const holding = turn === undefined ? undefined : Atomics.load(turn, 0);
  Atomics.store(word, 0, 1);
  Atomics.notify(word, 0);
  // The ask changes the turn's word and notifies nothing here, so the hold looks at it again each millisecond.
  while (Atomics.load(word, 1) === 0 && (turn === undefined || Atomics.load(turn, 0) === holding)) Atomics.wait(word, 1, 0, 1);
}

Database.prototype.exec = function heldExec(this: Database.Database, source: string): Database.Database {
  const result = exec.call(this, source);
  if (source === "BEGIN IMMEDIATE" && heldTry !== undefined && Atomics.load(heldTry, 0) === 0) hold(heldTry);
  return result;
};
