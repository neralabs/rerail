/**
 * Moves the rule provenance that audits recorded before `audit-rule-provenance-once` hold inline
 * (`rule_provenance_json`) to the one copy `audit_rule_provenances` keeps. It runs after the open, never inside it:
 * the daemon starts it once it serves (`daemon.ts`). Each batch of audits is one short write transaction, with a pause
 * before it, so other writers keep recording while it runs; a process stopped partway loses at most one batch, and the
 * next run goes on from what is left. Until an audit moves, the audit store reads its provenance inline.
 */
import { createHash } from "node:crypto";
import { existsSync, statfsSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { eraseFreedPages, openRerailDatabase, type RerailDatabase } from "./database.js";
import { RULE_PROVENANCE_STEP } from "./migrations.js";

/** How many audits one transaction moves: it bounds the memory the move holds and how long it keeps the write lock. */
const MOVED_PER_BATCH = 100;

/**
 * The pause before each batch. It is longer than the longest sleep of SQLite's busy handler (100 ms), so a writer in
 * another process that waits for the lock tries again, and takes it, between two batches.
 */
const PAUSE_MS = 150;

export interface RuleProvenanceMoveOptions {
  /** Audits per write transaction. */
  readonly perBatch?: number;
  /** Waits before each batch; tests pass their own. */
  readonly pause?: () => Promise<void>;
  /** Stops the move before its next batch; what it moved stays moved. */
  readonly signal?: AbortSignal;
}

/**
 * Moves the provenance of every audit that held it inline when the file ran `audit-rule-provenance-once`, then, if it
 * moved any, gives the space it freed back to the disk (`giveBackFreedSpace`). Audits that an older Rerail sharing the
 * file writes after that step keep theirs inline, where both versions read it. A file that does not exist yet holds
 * nothing to move, and the move does not create it. Resolves to how many audits it moved.
 */
export async function moveRuleProvenances(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  const { perBatch = MOVED_PER_BATCH, signal } = options;
  const pause = options.pause ?? (() => sleep(PAUSE_MS, undefined, { signal }).catch(() => undefined));
  if (!existsSync(path)) return 0;
  const database = openRerailDatabase(path);
  try {
    const upTo = stepAppliedAt(database);
    if (upTo === undefined) return 0;
    let moved = 0;
    let after = Number.MIN_SAFE_INTEGER;
    for (;;) {
      await pause();
      if (signal?.aborted === true) return moved;
      const batch = moveBatch(database, { after, upTo, limit: perBatch });
      if (batch === undefined) break;
      moved += batch.moved;
      after = batch.last;
    }
    // A run stopped before the end skips this, and the run that moves the rest gives the space back. One stopped after
    // its last batch leaves the free pages to new history.
    if (moved > 0) giveBackFreedSpace(database, path);
    return moved;
  } finally {
    database.$client.close();
  }
}

/** When the file ran `audit-rule-provenance-once` (epoch ms), or undefined when it has no record of that. */
function stepAppliedAt(database: RerailDatabase): number | undefined {
  return database.$client.prepare<[string], number>("SELECT applied_at FROM schema_migrations WHERE id = ?").pluck()
    .get(RULE_PROVENANCE_STEP);
}

/**
 * Moves up to `limit` audits after rowid `after` that started no later than `upTo`, in one write transaction: each names
 * the stored copy of its JSON text instead, stored first when no audit had the same text, and keeps `[]` inline. The
 * text moves as it is, so every audit reads the same provenance after. Its key is the SHA-256 of the text, as the
 * audit store keys the provenance it writes, so the store's later audits of the same rules name the copy moved here.
 * The last rowid it moved and how many, or undefined when none was left.
 */
function moveBatch(
  database: RerailDatabase,
  { after, upTo, limit }: { after: number; upTo: number; limit: number },
): { last: number; moved: number } | undefined {
  const client = database.$client;
  return withFreedPagesKept(client, () => client.transaction(() => {
    const rows = client.prepare<[number, number, number], { row: number; json: string }>(`SELECT rowid AS row, rule_provenance_json AS json
      FROM audits WHERE rowid > ? AND started_at <= ? AND rule_provenance_id IS NULL AND rule_provenance_json <> '[]'
      ORDER BY rowid LIMIT ?`).all(after, upTo, limit);
    const stored = client.prepare<[string], number>("SELECT provenance_id FROM audit_rule_provenances WHERE digest = ?").pluck();
    const store = client.prepare<[string, string]>("INSERT INTO audit_rule_provenances (digest, rule_provenance_json) VALUES (?, ?)");
    const name = client.prepare<[number, number]>("UPDATE audits SET rule_provenance_id = ?, rule_provenance_json = '[]' WHERE rowid = ?");
    for (const { row, json } of rows) {
      const digest = createHash("sha256").update(json).digest("hex");
      name.run(stored.get(digest) ?? Number(store.run(digest, json).lastInsertRowid), row);
    }
    const last = rows.at(-1);
    return last === undefined ? undefined : { last: last.row, moved: rows.length };
  }).immediate());
}

/**
 * Runs `work` with `secure_delete` FAST, then sets back what the connection had. Writers zero what they free (ON,
 * `database.ts`), and SQLite writes each zeroed page through the WAL, so the move would write what it moves a second
 * time: about a third of a batch's time and four fifths of the WAL it fills. FAST leaves a freed page as it was. Every
 * page a batch frees holds a copy of provenance that `audit_rule_provenances` keeps, and the `VACUUM` after the move
 * drops it; when that `VACUUM` is skipped, the copies stay in free pages until new history reuses them or
 * `rerail reset` or `rerail scrub` rewrites the file. The pragma reads FAST back as 2 but takes `= 2` as ON, so FAST is
 * set back by its keyword.
 */
function withFreedPagesKept<T>(client: RerailDatabase["$client"], work: () => T): T {
  const previous = client.pragma("secure_delete", { simple: true });
  client.pragma("secure_delete = FAST");
  try {
    return work();
  } finally {
    client.pragma(`secure_delete = ${previous === 2 ? "FAST" : String(previous)}`);
  }
}

/**
 * Once the move has freed much of the file, `VACUUM` once (`eraseFreedPages`) so the space goes back to the disk:
 * without it the file keeps its size, mostly free pages (1.3 GB of 1.4 GB for 4,000 audits). `VACUUM` copies the pages
 * in use to a temporary file, then writes them through the WAL beside the database while it holds the write lock. It is
 * skipped, and stderr says why, when another connection keeps the WAL from being emptied first, when the database's
 * folder or the temporary folder has less free disk space than the file's size, or when `VACUUM` fails (another writer
 * past the busy timeout, a full disk), which leaves the file as it was. Stderr also says when the file shrinks only
 * later (`sayShrinkWaits`). Never throws: the move has committed.
 */
function giveBackFreedSpace(database: RerailDatabase, path: string): void {
  const [checkpoint] = database.$client.pragma("wal_checkpoint(TRUNCATE)") as Array<{ busy: number }>;
  let skipped = checkpoint?.busy === 0 ? shortOfDisk(path) : "another connection was using it";
  if (skipped === undefined) {
    try {
      if (!eraseFreedPages(database)) sayShrinkWaits(database, path);
      return;
    } catch (error) {
      skipped = `VACUUM failed (${errorText(error)})`;
    }
  }
  let sizes = "";
  try {
    const pragma = (name: string): number => database.$client.pragma(name, { simple: true }) as number;
    sizes = `, ${megabytes(pragma("freelist_count") * pragma("page_size"))} of its ${megabytes(statSync(path).size)} free for new history,`;
  } catch {
    // The reason matters more than the sizes.
  }
  process.stderr.write(`rerail: ${path} keeps its size${sizes} after this Rerail's upgrade: ${skipped}.\n`);
}

/**
 * The line for a `VACUUM` that ended while another connection was reading: the checkpoint after it could not write the
 * rewritten file from the WAL into the database file, so the file keeps its size, and the WAL the size of the new copy,
 * until the first checkpoint after that read ends.
 */
function sayShrinkWaits(database: RerailDatabase, path: string): void {
  let size = "";
  try {
    const pragma = (name: string): number => database.$client.pragma(name, { simple: true }) as number;
    size = ` to ${megabytes(pragma("page_count") * pragma("page_size"))}`;
  } catch {
    // The reason matters more than the size.
  }
  process.stderr.write(`rerail: ${path} shrinks${size} at a later checkpoint, after this Rerail's upgrade: another `
    + "connection was reading it as VACUUM ended, so the rewritten file waits in its WAL.\n");
}

/**
 * Why `VACUUM` may run out of disk space for the file at `path`, or undefined when it has room. It writes a copy of the
 * pages in use to the temporary folder and another through the WAL beside the file; the file's size bounds each.
 */
function shortOfDisk(path: string): string | undefined {
  try {
    const size = statSync(path).size;
    for (const folder of new Set([dirname(path), tmpdir()])) {
      const { bavail, bsize } = statfsSync(folder);
      if (bavail * bsize < size) {
        return `${folder} has ${megabytes(bavail * bsize)} of free disk space, less than the file's size, which VACUUM may need`;
      }
    }
    return undefined;
  } catch (error) {
    return `its free disk space could not be read (${errorText(error)})`;
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function megabytes(bytes: number): string {
  return `${(bytes / 1e6).toFixed(1)} MB`;
}
