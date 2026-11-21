import { existsSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import Database from "better-sqlite3";
import type { RerailDatabase } from "../engine/storage/database.js";
import { RERAIL_DATABASE_FILE, REMOTE_OUTBOX_FILE } from "../engine/storage/rerail-home.js";

/**
 * The outbox's record, per remote, of the last event number a batch carried out, written before the batch goes and
 * waiting for the disk (`remote-outbox.ts`): a remote may hold every event up to it, whatever a power cut takes after.
 */
export const POSTED_EVENTS_STREAM = "event-posted";

/**
 * The highest event number any remote of the Rerail home that holds `databasePath` may hold: the larger of the outbox's
 * `event` cursors, moved once a remote acknowledged a batch, and its records of the batches it sent
 * (`POSTED_EVENTS_STREAM`), which a cut between the remote's answer and the cursor's move leaves on disk. 0 when the
 * home has no outbox, as it has none until a remote is added, or the file is not a home's database. An outbox that
 * cannot be read counts as none: the outbox itself reports why when it next runs.
 */
function takenByRemotes(databasePath: string): number {
  if (basename(databasePath) !== RERAIL_DATABASE_FILE) return 0;
  const outboxPath = join(dirname(databasePath), REMOTE_OUTBOX_FILE);
  if (!existsSync(outboxPath)) return 0;
  let outbox: Database.Database | undefined;
  try {
    outbox = new Database(outboxPath, { readonly: true, fileMustExist: true });
    const row = outbox.prepare("SELECT max(value) AS value FROM cursors WHERE stream IN ('event', ?)").get(POSTED_EVENTS_STREAM) as { value: number | null };
    return row.value ?? 0;
  } catch {
    return 0;
  } finally {
    outbox?.close();
  }
}

/** The highest number the database has handed an event (`events` is AUTOINCREMENT: SQLite keeps it in `sqlite_sequence`). */
function handedOut(database: RerailDatabase): number {
  const row = database.$client.prepare("SELECT seq FROM sqlite_sequence WHERE name = 'events'").get() as { seq: number } | undefined;
  return row?.seq ?? 0;
}

/**
 * Makes the next event of `database` take a number past every one a remote of its home has taken. Numbers are never
 * handed out twice while the file keeps every commit, even past `rerail reset`'s deletes. A power cut or an operating
 * system crash can take the last commits (`synchronous = NORMAL`, `database.ts`): the counter goes back with the rows,
 * and the next events would take the numbers of rows a remote already holds. The outbox sends an event once, by number
 * (`<machine id>:<sequence>`), so it would never send them, and a remote keeps the latest record of an id, so sending
 * them under those numbers would replace the lost events it holds with others. Run at a writer's first session open,
 * before that session's first event (`SqliteSessionStore.openSession`): a power cut ends every process, and every
 * event after one belongs to a session a process opened after it. Reads one file's existence when the home has no
 * remote; writes only when the counter is behind.
 */
export function keepEventNumbersPastRemotes(database: RerailDatabase, databasePath: string): void {
  const taken = takenByRemotes(databasePath);
  if (taken === 0 || handedOut(database) >= taken) return;
  // Under the write lock: `sqlite_sequence` holds no row for a table until its first insert, and two writers each adding
  // one would leave two.
  database.$client.transaction(() => {
    const moved = database.$client.prepare("UPDATE sqlite_sequence SET seq = max(seq, ?) WHERE name = 'events'").run(taken).changes;
    if (moved === 0) database.$client.prepare("INSERT INTO sqlite_sequence (name, seq) VALUES ('events', ?)").run(taken);
  }).immediate();
}
