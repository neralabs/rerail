/**
 * Moves the rule provenance that audits recorded before `audit-rule-provenance-once` hold inline
 * (`rule_provenance_json`) to the one copy `audit_rule_provenances` keeps. It runs after the open, never inside it:
 * the daemon starts it once it serves (`daemon.ts`). Each batch of audits is one short write transaction, with a pause
 * before it, so other writers keep recording while it runs; a process stopped partway loses at most one batch, and the
 * next run goes on from what is left. Until an audit moves, the audit store reads its provenance inline. The move runs
 * on the daemon's only thread, so it never waits inside SQLite for another connection: it awaits a pause and tries
 * again (`whenFree`), and the daemon answers in the meantime.
 */
import { createHash } from "node:crypto";
import { existsSync, statfsSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import Database from "better-sqlite3";
import { openRerailDatabase, type RerailDatabase } from "./database.js";
import { RULE_PROVENANCE_STEP } from "./migrations.js";

/** How many audits one transaction moves: it bounds the memory the move holds and how long it keeps the write lock. */
const MOVED_PER_BATCH = 100;

/**
 * The pause before each batch, and between two tries while another connection is in the move's way. It is longer than
 * the longest sleep of SQLite's busy handler (100 ms), so a writer in another process that waits for the lock tries
 * again, and takes it, between two batches.
 */
const PAUSE_MS = 150;

/**
 * How many times the move tries a step while another connection is in its way: with the daemon's pause, the 5 s every
 * Rerail connection waits (`busy_timeout`, `database.ts`). Past that it gives up as such a connection does.
 */
const BUSY_TRIES = Math.ceil(5_000 / PAUSE_MS);

export interface RuleProvenanceMoveOptions {
  /** Audits per write transaction. */
  readonly perBatch?: number;
  /** Waits before each batch, and between tries while another connection is in the way; tests pass their own. */
  readonly pause?: () => Promise<void>;
  /** Stops the move before its next batch; what it moved stays moved. */
  readonly signal?: AbortSignal;
}

type Client = RerailDatabase["$client"];

/** Runs a step on the move's connection, trying it again after a pause while another connection is in its way. */
type Free = <T>(step: () => T) => Promise<T>;

/**
 * Moves the provenance of every audit that held it inline when the file ran `audit-rule-provenance-once`, then, if it
 * moved any, gives the space it freed back to the disk (`giveBackFreedSpace`). Audits that an older Rerail sharing the
 * file writes after that step keep theirs inline, where both versions read it. A file that does not exist yet holds
 * nothing to move, and the move does not create it. Resolves to how many audits it moved.
 */
export async function moveRuleProvenances(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  const { perBatch = MOVED_PER_BATCH, signal } = options;
  const pause = options.pause ?? (() => sleep(PAUSE_MS, undefined, { signal }).catch(() => undefined));
  if (!existsSync(path) || !leftToMove(path)) return 0;
  const database = openRerailDatabase(path);
  try {
    // From here SQLite reports another connection's lock at once, and `whenFree` waits for it off the thread.
    database.$client.pragma("busy_timeout = 0");
    const free: Free = step => whenFree(step, pause, signal);
    const upTo = stepAppliedAt(database);
    if (upTo === undefined) return 0;
    let moved = 0;
    let after = Number.MIN_SAFE_INTEGER;
    for (;;) {
      await pause();
      if (signal?.aborted === true) return moved;
      const batch = await free(() => moveBatch(database, { after, upTo, limit: perBatch }));
      if (batch === undefined) break;
      moved += batch.moved;
      after = batch.last;
    }
    // A run stopped before the end skips this, and the run that moves the rest gives the space back. One stopped after
    // its last batch leaves the free pages to new history.
    if (moved > 0) await giveBackFreedSpace(database.$client, path, free);
    return moved;
  } finally {
    database.$client.close();
  }
}

/**
 * What `step` returns, tried again after each `pause` while another connection is in its way (`isBusy`): a writer that
 * holds the lock, or a read that keeps a checkpoint from emptying the WAL. After `BUSY_TRIES` tries, or once `signal`
 * stops the move, it throws what the last try threw.
 */
async function whenFree<T>(step: () => T, pause: () => Promise<void>, signal: AbortSignal | undefined): Promise<T> {
  for (let tries = 1; ; tries += 1) {
    try {
      return step();
    } catch (error) {
      if (!isBusy(error) || tries >= BUSY_TRIES || signal?.aborted === true) throw error;
    }
    await pause();
  }
}

/** A `wal_checkpoint` row: `busy` is 1 when another connection kept it from finishing; frames in the WAL, and written. */
interface Checkpoint {
  readonly busy: number;
  readonly log: number;
  readonly checkpointed: number;
}

/** A checkpoint that another connection's read kept from emptying the WAL, with how far it got. */
class WalInUse extends Error {
  readonly checkpoint: Checkpoint | undefined;

  constructor(checkpoint: Checkpoint | undefined) {
    super("another connection kept the WAL from being emptied");
    this.checkpoint = checkpoint;
  }
}

/** Whether `error` says another connection held what the statement needed, which a later try may find free. */
function isBusy(error: unknown): boolean {
  return error instanceof WalInUse || (error instanceof Database.SqliteError && error.code.startsWith("SQLITE_BUSY"));
}

/** Writes the WAL into the database file and empties it (`wal_checkpoint(TRUNCATE)`); throws `WalInUse` when it could not. */
function emptyWal(client: Client): void {
  const [checkpoint] = client.pragma("wal_checkpoint(TRUNCATE)") as Checkpoint[];
  if (checkpoint?.busy !== 0) throw new WalInUse(checkpoint);
}

/** When the file ran `audit-rule-provenance-once` (epoch ms), or undefined when it has no record of that. */
function stepAppliedAt(database: RerailDatabase): number | undefined {
  return database.$client.prepare<[string], number>("SELECT applied_at FROM schema_migrations WHERE id = ?").pluck()
    .get(RULE_PROVENANCE_STEP);
}

/** The audits the move takes: those holding their provenance inline that started no later than the step ran (`?`). */
const TAKEN = "started_at <= ? AND rule_provenance_id IS NULL AND rule_provenance_json <> '[]'";

/**
 * Whether the file holds an audit the move takes, read on a read-only connection: once the move has ended, each later
 * daemon start finds none and opens no writer, so it takes no write lock. A read-only open runs a pending step through
 * a writable open first, as it does for any reader.
 */
function leftToMove(path: string): boolean {
  const database = openRerailDatabase(path, { readonly: true });
  try {
    const upTo = stepAppliedAt(database);
    return upTo !== undefined
      && database.$client.prepare<[number], number>(`SELECT 1 FROM audits WHERE ${TAKEN} LIMIT 1`).pluck().get(upTo) !== undefined;
  } finally {
    database.$client.close();
  }
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
  return client.transaction(() => {
    const rows = client.prepare<[number, number, number], { row: number; json: string }>(`SELECT rowid AS row, rule_provenance_json AS json
      FROM audits WHERE rowid > ? AND ${TAKEN} ORDER BY rowid LIMIT ?`).all(after, upTo, limit);
    const stored = client.prepare<[string], number>("SELECT provenance_id FROM audit_rule_provenances WHERE digest = ?").pluck();
    const store = client.prepare<[string, string]>("INSERT INTO audit_rule_provenances (digest, rule_provenance_json) VALUES (?, ?)");
    const name = client.prepare<[number, number]>("UPDATE audits SET rule_provenance_id = ?, rule_provenance_json = '[]' WHERE rowid = ?");
    for (const { row, json } of rows) {
      const digest = createHash("sha256").update(json).digest("hex");
      name.run(stored.get(digest) ?? Number(store.run(digest, json).lastInsertRowid), row);
    }
    const last = rows.at(-1);
    return last === undefined ? undefined : { last: last.row, moved: rows.length };
  }).immediate();
}

/**
 * Once the move has freed much of the file, `VACUUM` once so the space goes back to the disk: without it the file keeps
 * its size, mostly free pages (1.3 GB of 1.4 GB for 4,000 audits). It runs `eraseFreedPages`' three steps (`database.ts`)
 * one by one, each through `free`: empty the WAL into the file, `VACUUM`, which copies the pages in use to a temporary
 * file and writes them back through the WAL while it holds the write lock, and empty the WAL again. `VACUUM` is skipped,
 * and stderr says why, when another connection keeps the WAL from being emptied first, when the database's folder or the
 * temporary folder has less free disk space than the file's size, or when `VACUUM` fails (another writer holding the
 * lock through every try, a full disk), which leaves the file as it was. Stderr also says when a read keeps the WAL
 * full after `VACUUM` (`sayWalInUse`). Throws only what a checkpoint throws other than a busy WAL; the move has
 * committed.
 */
async function giveBackFreedSpace(client: Client, path: string, free: Free): Promise<void> {
  const skipped = await vacuumSkipped(client, path, free);
  if (skipped !== undefined) {
    sayKeepsItsSize(client, path, skipped);
    return;
  }
  try {
    await free(() => emptyWal(client));
  } catch (error) {
    if (!(error instanceof WalInUse)) throw error;
    sayWalInUse(client, path, error.checkpoint);
  }
}

/** Empties the WAL into the file, then runs `VACUUM`; why it did not, or undefined once it ran. */
async function vacuumSkipped(client: Client, path: string, free: Free): Promise<string | undefined> {
  try {
    await free(() => emptyWal(client));
  } catch (error) {
    if (error instanceof WalInUse) return "another connection was using it";
    throw error;
  }
  const short = shortOfDisk(path);
  if (short !== undefined) return short;
  try {
    await free(() => client.exec("VACUUM"));
    return undefined;
  } catch (error) {
    return `VACUUM failed (${errorText(error)})`;
  }
}

/** The line for a move whose `VACUUM` did not run, with the reason. */
function sayKeepsItsSize(client: Client, path: string, skipped: string): void {
  const sizes = sizesOr(() => `, ${megabytes(pragmaNumber(client, "freelist_count") * pragmaNumber(client, "page_size"))} of its `
    + `${megabytes(statSync(path).size)} free for new history,`);
  process.stderr.write(`rerail: ${path} keeps its size${sizes} after this Rerail's upgrade: ${skipped}.\n`);
}

/**
 * The line for a WAL that another connection's read kept full after `VACUUM`. A read begun before `VACUUM` ended keeps
 * the checkpoint from writing the rewritten file into the database file, which keeps its size until the first
 * checkpoint after that read ends. A read begun after it lets the checkpoint write every frame (`checkpointed` reaches
 * `log`) and cut the file to its pages in use; only the WAL keeps the rewritten file's size, which SQLite keeps on disk
 * until the last connection to the file closes.
 */
function sayWalInUse(client: Client, path: string, checkpoint: Checkpoint | undefined): void {
  const size = sizesOr(() => ` to ${megabytes(pragmaNumber(client, "page_count") * pragmaNumber(client, "page_size"))}`);
  if (checkpoint !== undefined && checkpoint.log > 0 && checkpoint.checkpointed === checkpoint.log) {
    const wal = sizesOr(() => ` at ${megabytes(statSync(`${path}-wal`).size)}`);
    process.stderr.write(`rerail: ${path} shrank${size} after this Rerail's upgrade, but its WAL stays${wal} until the last `
      + "connection to the file closes: another connection was reading it as the WAL was emptied.\n");
    return;
  }
  process.stderr.write(`rerail: ${path} shrinks${size} at a later checkpoint, after this Rerail's upgrade: another `
    + "connection was reading it as VACUUM ended, so the rewritten file waits in its WAL.\n");
}

/** What `describe` returns, or nothing when it cannot read the sizes: the reason matters more than the sizes. */
function sizesOr(describe: () => string): string {
  try {
    return describe();
  } catch {
    return "";
  }
}

function pragmaNumber(client: Client, name: string): number {
  // `simple` returns the first column of the first row: a number for these pragmas.
  return client.pragma(name, { simple: true }) as number;
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
