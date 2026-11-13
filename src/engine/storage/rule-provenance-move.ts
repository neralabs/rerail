/**
 * Moves the rule provenance that audits recorded before `audit-rule-provenance-once` hold inline
 * (`rule_provenance_json`) to the one copy `audit_rule_provenances` keeps. It runs after the open, never inside it:
 * the daemon starts it once it serves (`daemon.ts`), on a worker thread of its own (`moveRuleProvenancesInWorker`), so
 * none of its SQLite work holds the thread that answers requests. Each batch of audits is one write transaction sized
 * to hold the lock about 100 ms (`moveInBatches`), with a pause before it, so other writers keep recording while it
 * runs; a process stopped partway loses at most one batch, and the next run goes on from what is left. Until an audit moves, the audit store reads its provenance
 * inline. The move never waits inside SQLite for another connection: it awaits a pause and tries again (`whenFree`),
 * so a stop reaches it between two tries. The `VACUUM` that ends the move holds the write lock for as long as it copies
 * the file, so in the daemon it runs in a process of its own and gives way whenever the daemon's thread waits for that
 * lock (`vacuum-giving-way.ts`). A move whose `VACUUM` a stop or a skip cut short leaves the free space in the file, and
 * a later run gives it back while it is more than half the file and `OWED_FREE_BYTES` (`shrinkOwed`).
 */
import { createHash } from "node:crypto";
import { accessSync, constants, existsSync, statfsSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { Worker } from "node:worker_threads";
import Database from "better-sqlite3";
import { openRerailDatabase, type RerailDatabase } from "./database.js";
import { RULE_PROVENANCE_STEP } from "./migrations.js";
import { PING, PONG, type ThreadWatch, vacuumGivingWay, VacuumNotRun, WAIT_NOTICE_MS } from "./vacuum-giving-way.js";

/** What `moveRuleProvenancesInWorker` posts to stop the move's thread (`rule-provenance-move-worker.ts`). */
export const MOVE_STOP = "rerail-move-stop";

/** The most audits one transaction moves: it bounds the memory the move holds. */
const MOVED_PER_BATCH = 100;

/** How many audits the first batch moves. Each later one is sized from how long the one before held the write lock. */
const FIRST_BATCH = 10;

/**
 * How long one batch aims to hold the write lock. Every other writer waits for it, the daemon's own stores among them,
 * which wait on the thread that answers hooks: on a loaded Windows machine one batch of 100 audits held it 3.6 s, past
 * the 2 s a hook's health probe waits.
 */
const BATCH_MS = 100;

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

/**
 * The free space a later run gives back, once the move has ended, when it is also more than half the file
 * (`shrinkOwed`): a move whose `VACUUM` was cut short leaves most of the file free (1.3 GB of 1.4 GB for 4,000 audits),
 * and a `VACUUM` that ran leaves none. 64 MB is what the move frees from about 290 audits whose provenance took 221 KB
 * each (767 rules on), a seventh of the smallest file reported; below it, a start leaves the space to new history, which
 * fills it as it comes (39.9 KB per audited edit), and the line the cut wrote names `rerail scrub`.
 */
const OWED_FREE_BYTES = 64_000_000;

export interface RuleProvenanceMoveOptions {
  /** The most audits per write transaction. */
  readonly perBatch?: number;
  /** Waits before each batch, and between tries while another connection is in the way; tests pass their own. */
  readonly pause?: () => Promise<void>;
  /** The milliseconds that time each batch; tests pass their own. */
  readonly now?: () => number;
  /** Stops the move before its next batch, and a `VACUUM` run beside it at once; what it moved stays moved. */
  readonly signal?: AbortSignal;
  /**
   * The thread whose writes the `VACUUM` gives way to: with it, `VACUUM` runs in a process of its own once the file is
   * quiet, and ends whenever that thread stops answering, until one attempt commits (`vacuumGivingWay`). Without it,
   * `VACUUM` runs on the move's connection, holding the write lock until it ends.
   */
  readonly watch?: ThreadWatch;
  /** How long a `VACUUM` that gives way waits for a quiet file before stderr says so, once; tests pass their own. */
  readonly waitNotice?: number;
}

/** What the batches of one move take from its options. */
interface Batching {
  readonly most: number;
  readonly pause: () => Promise<void>;
  readonly now: () => number;
  readonly signal: AbortSignal | undefined;
}

type Client = RerailDatabase["$client"];

/** Runs a step on the move's connection, trying it again after a pause while another connection is in its way. */
type Free = <T>(step: () => T) => Promise<T>;

/**
 * Moves the provenance of every audit that held it inline when the file ran `audit-rule-provenance-once`, then, if it
 * moved any, or an earlier move's `VACUUM` was cut short (`shrinkOwed`), gives the space it freed back to the disk
 * (`giveBackFreedSpace`). Audits that an older Rerail sharing the file writes after that step keep theirs inline, where
 * both versions read it. A file that does not exist yet holds nothing to move, and the move does not create it. Resolves
 * to how many audits it moved.
 */
export async function moveRuleProvenances(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  const { signal } = options;
  const pause = options.pause ?? (() => sleep(PAUSE_MS, undefined, { signal }).catch(() => undefined));
  const now = options.now ?? (() => performance.now());
  const batching: Batching = { most: options.perBatch ?? MOVED_PER_BATCH, pause, now, signal };
  if (!existsSync(path) || !workLeft(path)) return 0;
  const database = openRerailDatabase(path);
  try {
    // From here SQLite reports another connection's lock at once, and `whenFree` waits for it between tries.
    database.$client.pragma("busy_timeout = 0");
    const free: Free = step => whenFree(step, pause, signal);
    const upTo = stepAppliedAt(database);
    if (upTo === undefined) return 0;
    const done = await moveInBatches(database, upTo, free, batching);
    const client = database.$client;
    if (shrinks(client, upTo, done)) await giveBackFreedSpace(client, path, free, vacuumFor(client, path, free, options));
    return done.moved;
  } finally {
    database.$client.close();
  }
}

/**
 * Whether the move gives the free space back now: once it has moved the last audit, if it moved any or a `VACUUM` is
 * owed. A run stopped before the end skips it, and the run that moves the rest gives the space back.
 */
function shrinks(client: Client, upTo: number, { moved, ended }: { moved: number; ended: boolean }): boolean {
  return ended && (moved > 0 || shrinkOwed(client, upTo));
}

/** The move's `VACUUM`: beside the move, giving way to the thread `options.watch` watches, or on its connection. */
function vacuumFor(client: Client, path: string, free: Free, options: RuleProvenanceMoveOptions): () => Promise<void> {
  const { watch, signal } = options;
  if (watch === undefined) {
    return () => free(() => {
      client.exec("VACUUM");
    });
  }
  const noticeMs = options.waitNotice ?? WAIT_NOTICE_MS;
  return () => vacuumGivingWay(client, path, {
    watch, signal, noticeMs,
    // Read again before each attempt: one can start hours after the first, when the disk holds less.
    blocked: () => shortOfDisk(client, path),
    waitedLong: quietMs => sayStillWaiting(client, path, { quietMs, noticeMs }),
  });
}

/**
 * Moves the audits that started no later than `upTo`, a batch per write transaction with a pause before each, until
 * none is left or `signal` stops it: how many it moved, and whether none is left. The first batch takes `FIRST_BATCH`
 * audits, and each later one as many as the batch before moved in `BATCH_MS` (`nextBatchSize`), so a batch holds the
 * write lock about that long however loaded the machine and however large each audit's provenance.
 */
async function moveInBatches(
  database: RerailDatabase,
  upTo: number,
  free: Free,
  { most, pause, now, signal }: Batching,
): Promise<{ moved: number; ended: boolean }> {
  let moved = 0;
  let after = Number.MIN_SAFE_INTEGER;
  let size = Math.min(FIRST_BATCH, most);
  for (;;) {
    await pause();
    if (signal?.aborted === true) return { moved, ended: false };
    let took = 0;
    const batch = await free(() => {
      const began = now();
      try {
        return moveBatch(database, { after, upTo, limit: size });
      } finally {
        took = now() - began;
      }
    });
    if (batch === undefined) return { moved, ended: true };
    moved += batch.moved;
    after = batch.last;
    size = nextBatchSize(size, took, most);
  }
}

/**
 * How many audits the batch after one of `size` that held the write lock `took` ms takes: as many as fit in `BATCH_MS`
 * at that pace, at least one, at most twice `size`, so one quick batch does not make the next one long, and at most
 * `most`.
 */
function nextBatchSize(size: number, took: number, most: number): number {
  return Math.max(1, Math.min(most, 2 * size, Math.floor((size * BATCH_MS) / Math.max(took, 1))));
}

/**
 * Runs `moveRuleProvenances` on a worker thread with a connection of its own, and resolves to how many audits it moved
 * once that thread ends. Its batches, checkpoints and `VACUUM` then never run on the calling thread, which in the daemon
 * answers the requests hooks send: on a loaded Windows machine one batch of 100 audits took 3.6 s, and the `VACUUM` of
 * 4,000 audits 8.9 s, past the 2 s a hook's health probe waits. The calling thread's own writes still wait for the
 * write lock the `VACUUM` holds, so the `VACUUM` gives way to them: the worker pings this thread while it runs, and ends
 * it once this thread stops answering (`watchThread`). `signal` stops the move before its next step, and a running
 * `VACUUM` at once. A file with nothing to move and no `VACUUM` owed, read on the calling thread, starts no thread.
 */
export async function moveRuleProvenancesInWorker(path: string, signal?: AbortSignal): Promise<number> {
  if (signal?.aborted === true || !existsSync(path) || !workLeft(path)) return 0;
  const worker = moveWorker(path);
  const stop = (): void => worker.postMessage(MOVE_STOP);
  signal?.addEventListener("abort", stop, { once: true });
  try {
    return await movedBy(worker);
  } finally {
    signal?.removeEventListener("abort", stop);
  }
}

/**
 * The worker beside this module (`rule-provenance-move-worker.ts`): its JavaScript as the package ships it, or, where
 * this module runs from its TypeScript source as the tests run it, that source through `tsx`.
 */
function moveWorker(path: string): Worker {
  const fromSource = new URL(import.meta.url).pathname.endsWith(".ts");
  const entry = new URL(`./rule-provenance-move-worker.${fromSource ? "ts" : "js"}`, import.meta.url);
  return new Worker(entry, { workerData: path, ...(fromSource ? { execArgv: ["--import", "tsx"] } : {}) });
}

/** How many audits `worker` moved, once it ends; what it threw, when it fails. Answers each of its pings meanwhile. */
function movedBy(worker: Worker): Promise<number> {
  return new Promise((resolve, reject) => {
    let moved = 0;
    worker.on("message", (message: unknown) => {
      if (message === PING) worker.postMessage(PONG);
      else if (typeof message === "number") moved = message;
    });
    worker.once("error", reject);
    worker.once("exit", code => {
      if (code === 0) resolve(moved);
      else reject(new Error(`the thread moving audit rule provenance exited with code ${code}`));
    });
  });
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

/**
 * Writes the WAL into the database file and empties it (`wal_checkpoint(TRUNCATE)`); throws `WalInUse` when it could not.
 * A TRUNCATE checkpoint holds the write lock while it copies the WAL into the file, so a PASSIVE one, which takes no
 * write lock, copies what it can first: after `VACUUM` the WAL can hold the whole rewritten file.
 */
function emptyWal(client: Client): void {
  client.pragma("wal_checkpoint(PASSIVE)");
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
 * Whether the file holds an audit the move takes, or a `VACUUM` an earlier move did not finish (`shrinkOwed`), read on a
 * read-only connection: once the move and its `VACUUM` have ended, each later daemon start finds neither and opens no
 * writer, so it takes no write lock. A read-only open runs a pending step through a writable open first, as it does for
 * any reader.
 */
function workLeft(path: string): boolean {
  const database = openRerailDatabase(path, { readonly: true });
  try {
    const upTo = stepAppliedAt(database);
    const client = database.$client;
    return upTo !== undefined
      && (client.prepare<[number], number>(`SELECT 1 FROM audits WHERE ${TAKEN} LIMIT 1`).pluck().get(upTo) !== undefined
        || shrinkOwed(client, upTo));
  } finally {
    database.$client.close();
  }
}

/**
 * Whether a `VACUUM` the move ran up to is owed: an audit that started before the step names a stored provenance (the
 * move moved it), and the free pages are more than half the file and more than `OWED_FREE_BYTES`. A move whose `VACUUM`
 * was stopped, skipped or failed leaves the file so; a `VACUUM` that ran leaves no free page.
 */
function shrinkOwed(client: Client, upTo: number): boolean {
  const free = pragmaNumber(client, "freelist_count");
  return 2 * free > pragmaNumber(client, "page_count") && free * pragmaNumber(client, "page_size") > OWED_FREE_BYTES
    && client.prepare<[number], number>("SELECT 1 FROM audits WHERE rule_provenance_id IS NOT NULL AND started_at <= ? LIMIT 1")
      .pluck().get(upTo) !== undefined;
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
 * one by one, each through `free` or `vacuum`: empty the WAL into the file, `VACUUM`, which copies the pages in use to a
 * temporary file and writes them back through the WAL while it holds the write lock, and empty the WAL again. `VACUUM`
 * is skipped, and stderr says why, when the WAL cannot be emptied first (another connection keeps it busy through every
 * try, or the checkpoint fails), when a disk has less free space than the copies `VACUUM` writes to it (`shortOfDisk`),
 * when `VACUUM` fails (another writer holding the lock through every try, a full disk), or when a stop ends it, each of
 * which leaves the file as it was. Stderr also says when the WAL stays full after `VACUUM` (`sayWalInUse`,
 * `sayWalNotEmptied`). Never throws: the move has committed, and a step that fails here only leaves the file larger
 * than it could be.
 */
async function giveBackFreedSpace(client: Client, path: string, free: Free, vacuum: () => Promise<void>): Promise<void> {
  const skipped = await vacuumSkipped(client, path, free, vacuum);
  if (skipped !== undefined) {
    sayKeepsItsSize(client, path, skipped);
    return;
  }
  try {
    await free(() => emptyWal(client));
  } catch (error) {
    if (error instanceof WalInUse) sayWalInUse(client, path, error.checkpoint);
    else sayWalNotEmptied(client, path, error);
  }
}

/** Empties the WAL into the file, then runs `vacuum`; why it did not, or undefined once it ran. */
async function vacuumSkipped(client: Client, path: string, free: Free, vacuum: () => Promise<void>): Promise<string | undefined> {
  const before = (await walNotEmptied(client, free)) ?? shortOfDisk(client, path);
  if (before !== undefined) return before;
  try {
    await vacuum();
    return undefined;
  } catch (error) {
    return error instanceof VacuumNotRun ? error.message : `VACUUM failed (${errorText(error)})`;
  }
}

/** Empties the WAL into the file: why it could not, or undefined once it did. */
async function walNotEmptied(client: Client, free: Free): Promise<string | undefined> {
  try {
    await free(() => emptyWal(client));
    return undefined;
  } catch (error) {
    return error instanceof WalInUse ? "another connection was using it" : `its WAL could not be emptied (${errorText(error)})`;
  }
}

/** The line for a move whose `VACUUM` did not run, with the reason and what gives the space back. */
function sayKeepsItsSize(client: Client, path: string, skipped: string): void {
  process.stderr.write(`rerail: ${path} keeps its size${freeOfSize(client, path)} after this Rerail's upgrade: ${skipped}. `
    + "Run `rerail scrub` to give the free space back.\n");
}

/**
 * The line for a `VACUUM` that has waited `noticeMs` for the quiet it needs: what is free, that the daemon keeps waiting,
 * and what gives the space back now.
 */
function sayStillWaiting(client: Client, path: string, { quietMs, noticeMs }: { quietMs: number; noticeMs: number }): void {
  process.stderr.write(`rerail: ${path} has not shrunk yet${freeOfSize(client, path)} after this Rerail's upgrade: in `
    + `${duration(noticeMs)} no other connection left the file alone for the ${duration(quietMs)} its VACUUM waits for. `
    + "The daemon keeps waiting; run `rerail scrub` to give the free space back now.\n");
}

/** ", <free> of its <size> free for new history,", or nothing when the sizes cannot be read. */
function freeOfSize(client: Client, path: string): string {
  return sizesOr(() => `, ${megabytes(pragmaNumber(client, "freelist_count") * pragmaNumber(client, "page_size"))} of its `
    + `${megabytes(statSync(path).size)} free for new history,`);
}

/** `ms` in whole minutes from one minute up, else in seconds. */
function duration(ms: number): string {
  return ms >= 60_000 ? `${Math.round(ms / 60_000)} minutes` : `${ms / 1_000} s`;
}

/**
 * The line for a WAL that another connection's read kept full after `VACUUM`. A read begun before `VACUUM` ended keeps
 * the checkpoint from writing the rewritten file into the database file, which keeps its size until the first
 * checkpoint after that read ends. A read begun after it lets the checkpoint write every frame (`checkpointed` reaches
 * `log`) and cut the file to its pages in use; only the WAL keeps the rewritten file's size, which SQLite keeps on disk
 * until the last connection to the file closes.
 */
function sayWalInUse(client: Client, path: string, checkpoint: Checkpoint | undefined): void {
  const size = rewrittenSize(client);
  if (checkpoint !== undefined && checkpoint.log > 0 && checkpoint.checkpointed === checkpoint.log) {
    const wal = sizesOr(() => ` at ${megabytes(statSync(`${path}-wal`).size)}`);
    process.stderr.write(`rerail: ${path} shrank${size} after this Rerail's upgrade, but its WAL stays${wal} until the last `
      + "connection to the file closes: another connection was reading it as the WAL was emptied.\n");
    return;
  }
  process.stderr.write(`rerail: ${path} shrinks${size} at a later checkpoint, after this Rerail's upgrade: another `
    + "connection was reading it as VACUUM ended, so the rewritten file waits in its WAL.\n");
}

/**
 * The line for a checkpoint after `VACUUM` that failed rather than found the WAL busy: the rewritten file waits in the
 * WAL, and SQLite's next checkpoint writes it into the database file.
 */
function sayWalNotEmptied(client: Client, path: string, error: unknown): void {
  process.stderr.write(`rerail: ${path} shrinks${rewrittenSize(client)} at a later checkpoint, after this Rerail's upgrade: `
    + `emptying its WAL after VACUUM failed (${errorText(error)}).\n`);
}

/** " to <size>", the size of the file `VACUUM` rewrote, or nothing when it cannot be read. */
function rewrittenSize(client: Client): string {
  return sizesOr(() => ` to ${megabytes(pragmaNumber(client, "page_count") * pragmaNumber(client, "page_size"))}`);
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
 * Why `VACUUM` may run out of disk space for the database at `path`, or undefined when it has room. `VACUUM` copies the
 * pages in use, not the free ones, to a temporary file in SQLite's temporary folder (`sqliteTemporaryFolder`), then
 * writes them back through the WAL beside the database: the disk of each folder needs room for one copy of the pages in
 * use, and a disk that holds both folders room for two. Right after the move most of the file is free pages (1.3 GB of
 * 1.4 GB for 4,000 audits), so the file's size would ask several times the room `VACUUM` takes.
 */
function shortOfDisk(client: Client, path: string): string | undefined {
  try {
    const inUse = (pragmaNumber(client, "page_count") - pragmaNumber(client, "freelist_count")) * pragmaNumber(client, "page_size");
    for (const { folder, copies } of disksOf([dirname(path), sqliteTemporaryFolder()])) {
      const { bavail, bsize } = statfsSync(folder);
      if (bavail * bsize < copies * inUse) {
        return `${folder} has ${megabytes(bavail * bsize)} of free disk space, less than the ${megabytes(copies * inUse)} `
          + "VACUUM writes to its disk";
      }
    }
    return undefined;
  } catch (error) {
    return `its free disk space could not be read (${errorText(error)})`;
  }
}

/**
 * The folder SQLite writes `VACUUM`'s temporary copy to. On Windows it asks the system (GetTempPath: TMP, then TEMP,
 * then the user's profile folder). Elsewhere it takes the first of SQLITE_TMPDIR, TMPDIR, /var/tmp, /usr/tmp and /tmp
 * that is a folder it may write in (`os_unix.c`), so with TMPDIR unset it is /var/tmp, where Node's `tmpdir()` is /tmp,
 * which many systems keep in memory.
 */
function sqliteTemporaryFolder(): string {
  const { SQLITE_TMPDIR, TMPDIR, TMP, TEMP, USERPROFILE } = process.env;
  if (process.platform === "win32") return TMP ?? TEMP ?? USERPROFILE ?? tmpdir();
  return [SQLITE_TMPDIR, TMPDIR, "/var/tmp", "/usr/tmp", "/tmp"].find(writableFolder) ?? ".";
}

function writableFolder(folder: string | undefined): folder is string {
  try {
    accessSync(folder ?? "", constants.W_OK | constants.X_OK);
    return folder !== undefined && statSync(folder).isDirectory();
  } catch {
    return false;
  }
}

/** The disks that hold `folders`: each once, named by its first folder, with how many of `folders` it holds. */
function disksOf(folders: readonly string[]): { folder: string; copies: number }[] {
  const disks = new Map<number, { folder: string; copies: number }>();
  for (const folder of folders) {
    const { dev } = statSync(folder);
    const disk = disks.get(dev);
    disks.set(dev, { folder: disk?.folder ?? folder, copies: (disk?.copies ?? 0) + 1 });
  }
  return [...disks.values()];
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function megabytes(bytes: number): string {
  return `${(bytes / 1e6).toFixed(1)} MB`;
}
