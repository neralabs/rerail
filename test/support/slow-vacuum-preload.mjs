/**
 * Loaded through NODE_OPTIONS (`--import`) into each thread or process a test starts after `slowVacuumNext`
 * (`slow-vacuum.ts`), so it reaches the thread or process that runs the move's VACUUM, which a spy set in the test's own
 * thread does not. The first VACUUM any of them runs takes the write lock, creates the file RERAIL_TEST_VACUUM_HELD,
 * and holds the lock RERAIL_TEST_VACUUM_HOLD_MS before it rolls back and runs, as a VACUUM of a large file holds it. A
 * VACUUM that finds that file already there runs at once. Plain JavaScript: it loads before `tsx` does.
 */
import { closeSync, openSync } from "node:fs";
import { createRequire } from "node:module";

const held = process.env.RERAIL_TEST_VACUUM_HELD;
const holdMs = Number(process.env.RERAIL_TEST_VACUUM_HOLD_MS ?? "0");

if (held !== undefined && holdMs > 0) {
  const Database = createRequire(import.meta.url)("better-sqlite3");
  const exec = Database.prototype.exec;
  Database.prototype.exec = function (source) {
    if (source === "VACUUM") holdFirst(this, exec);
    return exec.call(this, source);
  };
}

/** Takes the write lock on `client` and, when no VACUUM held it before, holds it `holdMs` (blocking, as VACUUM does). */
function holdFirst(client, exec) {
  exec.call(client, "BEGIN IMMEDIATE");
  try {
    if (first()) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, holdMs);
  } finally {
    exec.call(client, "ROLLBACK");
  }
}

/** Whether this is the first VACUUM to hold the lock: it creates `held`, which none before it did. */
function first() {
  try {
    closeSync(openSync(held, "wx"));
    return true;
  } catch {
    return false;
  }
}
