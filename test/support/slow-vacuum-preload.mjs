/**
 * Loaded through NODE_OPTIONS (`--import`) into each thread or process a test starts after `slowVacuumNext`
 * (`slow-vacuum.ts`), so it reaches the thread or process that runs the move's VACUUM, which a spy set in the test's own
 * thread does not. Plain JavaScript: it loads before `tsx` does.
 *
 * With RERAIL_TEST_VACUUM_HELD, the first VACUUM any of them runs takes the write lock, creates that file, and holds the
 * lock RERAIL_TEST_VACUUM_HOLD_MS before it rolls back and runs, as a VACUUM of a large file holds it. A VACUUM that
 * finds that file already there runs at once.
 *
 * With RERAIL_TEST_VACUUM_COMMITTED, a VACUUM that has committed creates that file and waits
 * RERAIL_TEST_VACUUM_LINGER_MS before the checkpoint its commit runs, then creates `<that path>.done`: the moment after
 * the commit, when the rewritten file is still all in the WAL, made long enough for a test to act in it.
 */
import { closeSync, openSync } from "node:fs";
import { createRequire } from "node:module";

const held = process.env.RERAIL_TEST_VACUUM_HELD;
const holdMs = Number(process.env.RERAIL_TEST_VACUUM_HOLD_MS ?? "0");
const committed = process.env.RERAIL_TEST_VACUUM_COMMITTED;
const lingerMs = Number(process.env.RERAIL_TEST_VACUUM_LINGER_MS ?? "0");

if ((held !== undefined && holdMs > 0) || committed !== undefined) {
  const Database = createRequire(import.meta.url)("better-sqlite3");
  const exec = Database.prototype.exec;
  Database.prototype.exec = function (source) {
    if (source !== "VACUUM") return exec.call(this, source);
    if (held !== undefined && holdMs > 0) holdFirst(this, exec);
    if (committed === undefined) return exec.call(this, source);
    return lingerAfterCommit(this, exec);
  };
}

/** Takes the write lock on `client` and, when no VACUUM held it before, holds it `holdMs` (blocking, as VACUUM does). */
function holdFirst(client, exec) {
  exec.call(client, "BEGIN IMMEDIATE");
  try {
    if (create(held)) sleep(holdMs);
  } finally {
    exec.call(client, "ROLLBACK");
  }
}

/** VACUUM with the checkpoint after its commit put off `lingerMs`, the markers created on either side of that wait. */
function lingerAfterCommit(client, exec) {
  const automatic = client.pragma("wal_autocheckpoint", { simple: true });
  client.pragma("wal_autocheckpoint = 0");
  exec.call(client, "VACUUM");
  create(committed);
  sleep(lingerMs);
  client.pragma("wal_checkpoint(PASSIVE)");
  client.pragma(`wal_autocheckpoint = ${automatic}`);
  create(`${committed}.done`);
  return client;
}

/** Creates `path`, which must not exist yet: whether it did. */
function create(path) {
  try {
    closeSync(openSync(path, "wx"));
    return true;
  } catch {
    return false;
  }
}

function sleep(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}
