/**
 * Moves the rule provenance that audits recorded before `audit-rule-provenance-once` hold inline
 * (`rule_provenance_json`) to the one copy `audit_rule_provenances` keeps. It runs after the open, never inside it:
 * the daemon starts it once it serves (`daemon.ts`), on a worker thread of its own (`moveRuleProvenancesInWorker`), so
 * its batches and checkpoints never hold the thread that answers requests. That thread only reads, on a read-only
 * connection, whether audits are left to move and whether an older Rerail's audit makes the move wait (`moveState`):
 * at the start, after each pass of the wait, and once the move's thread ends. Each of those reads is one audit row,
 * found through `audits_rule_provenance` (`UNNAMED`), however long the history, and one more, read in full, for each
 * audit whose damaged provenance a move left as it is (`takes`). Each batch of audits is one write transaction sized
 * to hold the lock about 100 ms (`moveInBatches`), with a pause before it, so other writers keep recording while it
 * runs; a process stopped partway loses at most one batch, and the next run goes on from what is left. Until an audit
 * moves, the audit store reads its provenance inline. The move never waits inside SQLite for another connection: it
 * awaits a pause and tries again, for as long as another connection holds the write lock, so a stop reaches it between
 * two tries, and a long hold, such as `rerail scrub`'s on a large file, pauses the move without ending it.
 *
 * The pages a batch frees stay in the file, zeroed as every delete's are (`secure_delete`, `database.ts`), and new
 * history reuses them before the file grows again. No `VACUUM` runs here: one holds the write lock until it ends, and
 * another process's write fails once it has waited 5 s. `rerail scrub` gives the space back when a person runs it.
 *
 * A Rerail older than the step reads an audit's provenance only from the audit's own row, so it reads `[]` for each
 * audit the move took it from. The move therefore waits while such a Rerail may still be using the file: until
 * `OLDER_WRITE_WINDOW_MS` has passed since the last recorded audit that names no stored copy began, with rules or
 * without, those recorded before the step included (`lastOlderWrite`), checked before the move starts and again before
 * each batch. This Rerail's audit store names a stored copy for every audit it writes, `[]` included, and the move
 * names one for every audit it takes, so only an older Rerail writes an audit that names none. With only this Rerail
 * running, the move starts a week after the last audit recorded before the step.
 *
 * Every audit is recorded as it begins, so rowids follow the order audits began in. The move takes the audits that
 * name no stored copy in that order, up to the first that began after the step: from there on each is an older
 * Rerail's, recorded after the step, which keeps its provenance in its row, hundreds of kilobytes with its start after
 * it. Neither the move nor a read of its state reads such a row past that first one. An audit whose provenance in its
 * row is not a JSON array, which only a damaged file or a hand edit leaves, stays as it is (`moveBatch`).
 */
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { Worker } from "node:worker_threads";
import Database from "better-sqlite3";
import { openRerailDatabase, openSqliteDatabase, type RerailDatabase } from "./database.js";
import { RULE_PROVENANCE_STEP } from "./migrations.js";

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
 * How many times the checkpoint after the last batch tries while another connection's read keeps it from emptying the
 * WAL: with the daemon's pause, the 5 s every Rerail connection waits (`busy_timeout`, `database.ts`). Past that it
 * leaves the WAL to SQLite's later checkpoints. A batch has no such bound: it waits as long as another connection holds
 * the write lock, since a batch that gave up would end the move until the daemon's next start.
 */
const BUSY_TRIES = Math.ceil(5_000 / PAUSE_MS);

/**
 * How long the move waits after the last audit that names no stored copy of its provenance began. Rerail before the
 * step wrote every audit so, and a Rerail older than the step that shares the file still does, with its provenance or
 * `[]` in the audit's own row. Such a Rerail reads each audit's provenance only there: an audit moved while it still
 * runs beside this one reads `[]` in its `audits_list`, its support bundle and its remote sink. A week covers an agent
 * host that runs an older Rerail on some days of the week only.
 */
export const OLDER_WRITE_WINDOW_MS = 7 * 24 * 3_600_000;

/**
 * The longest the wait for an older Rerail sleeps before it reads the file and the clock again. Node fires a timer
 * longer than 2^31 - 1 ms (about 24.9 days) after 1 ms, which an audit dated weeks past the clock would ask for. A
 * timer counts only the time the machine is awake on Linux and macOS, so after a sleep the move starts at most this
 * late.
 */
const OLDER_WAIT_PASS_MS = 3_600_000;

const DAY_MS = 24 * 3_600_000;

export interface RuleProvenanceMoveOptions {
  /** The most audits per write transaction. */
  readonly perBatch?: number;
  /** Waits before each batch, and between tries while another connection is in the way; tests pass their own. */
  readonly pause?: () => Promise<void>;
  /** The milliseconds that time each batch; tests pass their own. */
  readonly now?: () => number;
  /** Stops the move before its next batch, or before the next try of one another connection is in the way of. */
  readonly signal?: AbortSignal;
  /**
   * The time in epoch milliseconds, to which an older Rerail's last audit (`lastOlderWrite`) is compared; for tests.
   */
  readonly clock?: () => number;
  /** How long after that audit began the move waits (`OLDER_WRITE_WINDOW_MS`); tests pass their own. */
  readonly olderWindowMs?: number;
}

/** What the batches of one move take from its options, and whether an older Rerail's audit stops them. */
interface Batching {
  readonly most: number;
  readonly pause: () => Promise<void>;
  readonly now: () => number;
  readonly signal: AbortSignal | undefined;
  readonly postponed: () => boolean;
}

/** How the move reads whether a Rerail older than the step still uses the file: its clock and its window. */
interface OlderWrites {
  readonly clock: () => number;
  readonly windowMs: number;
}

/**
 * What a file asks of the move: nothing; to move audits; or to wait until `until`, since the last recorded audit that
 * names no stored copy of its provenance, as an older Rerail writes each audit, began at `at`, less than the window
 * before.
 */
type MoveState =
  | { readonly kind: "none" }
  | { readonly kind: "move" }
  | { readonly kind: "wait"; readonly at: number; readonly until: number };

const NONE: MoveState = { kind: "none" };
const MOVE: MoveState = { kind: "move" };

/** A wait for an older Rerail (`MoveState`). */
type OlderWait = Extract<MoveState, { kind: "wait" }>;

/** What `moveRuleProvenancesInWorker` hands the thread it starts (`rule-provenance-move-worker.ts`). */
export interface MoveWorkerData {
  readonly path: string;
  readonly olderWindowMs: number;
}

/** Waits `ms`, holding no process open, and rejects once `signal` aborts. */
export type MoveTimer = (ms: number, signal: AbortSignal | undefined) => Promise<unknown>;

export interface MoveInWorkerOptions {
  /** How long after an older Rerail's last audit the move waits (`OLDER_WRITE_WINDOW_MS`); tests pass their own. */
  readonly olderWindowMs?: number;
  /** The time in epoch milliseconds the calling thread compares that audit with; tests pass their own. */
  readonly clock?: () => number;
  /** The timer each pass of the wait for an older Rerail sleeps on; tests pass their own. */
  readonly timer?: MoveTimer;
}

/** Node's timer from `timers/promises`, unreferenced: a daemon that ends does not wait for it. */
const unreferencedSleep: MoveTimer = (ms, signal) => sleep(ms, undefined, { signal, ref: false });

type Client = RerailDatabase["$client"];

/** What `whenFree` gives back for a step a stop ended while another connection was in its way. */
const STOPPED = Symbol("stopped");

/**
 * Runs a step on the move's connection, trying it again after a pause while another connection is in its way, and
 * gives back `STOPPED` for a step a stop ended between two tries.
 */
type Free = <T>(step: () => T) => Promise<T | typeof STOPPED>;

/**
 * Moves the provenance of every audit that held it inline when the file ran `audit-rule-provenance-once`, `[]`
 * included, then empties the WAL its batches filled into the file (`emptyWalAfterMove`). Audits that an older Rerail
 * sharing the file writes after that step keep theirs inline, where both versions read it. While the last recorded
 * audit that names no stored copy, as an older Rerail writes each audit, began within the window, one recorded before
 * the step included, the move takes nothing, or stops before its next batch (`lastOlderWrite`). A file that does not
 * exist yet holds nothing to move, and the move does not create it. Resolves to how many audits it moved.
 */
export async function moveRuleProvenances(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  const older = olderWritesOf(options);
  if (!existsSync(path) || moveState(path, older).kind !== "move") return 0;
  // `moveState` opened the file through `openRerailDatabase`, which refuses a file this build cannot read and runs a
  // pending step first. The writer only connects: a writable `openRerailDatabase` begins a write transaction, which
  // waits inside SQLite up to 5 s while another process holds the lock, where a stop cannot reach it.
  const database = openSqliteDatabase(path);
  try {
    // From here SQLite reports another connection's lock at once, and the move waits for it between tries.
    database.$client.pragma("busy_timeout = 0");
    const upTo = stepAppliedAt(database);
    if (upTo === undefined) return 0;
    const client = database.$client;
    const batching = batchingOf(options, () => takes(client, upTo) && lastOlderWrite(client, older) !== undefined);
    const free: Free = step => whenFree(step, batching.pause, options.signal);
    const done = await moveInBatches(database, upTo, batching);
    if (done.ended && done.moved > 0) await emptyWalAfterMove(client, free);
    return done.moved;
  } finally {
    database.$client.close();
  }
}

/**
 * The clock and window `options` give for an older Rerail's last audit, else the time now and `OLDER_WRITE_WINDOW_MS`.
 */
function olderWritesOf(options: Pick<RuleProvenanceMoveOptions, "clock" | "olderWindowMs">): OlderWrites {
  return { clock: options.clock ?? (() => Date.now()), windowMs: options.olderWindowMs ?? OLDER_WRITE_WINDOW_MS };
}

/**
 * How the batches of a move run, from `options` and `postponed`, which says when an older Rerail's audit stops them.
 */
function batchingOf(options: RuleProvenanceMoveOptions, postponed: () => boolean): Batching {
  const { signal } = options;
  const pause = options.pause ?? (() => sleep(PAUSE_MS, undefined, { signal }).catch(() => undefined));
  const now = options.now ?? (() => performance.now());
  return { most: options.perBatch ?? MOVED_PER_BATCH, pause, now, signal, postponed };
}

/**
 * Moves the audits that started no later than `upTo`, a batch per write transaction with a pause before each, until
 * none is left, or `signal` or an older Rerail's audit (`postponed`) stops it: how many it moved, and whether none is
 * left. A batch another connection's lock is in the way of is tried again after the next pause, with no bound, and
 * `signal` and `postponed` are read before each try. The first batch takes `FIRST_BATCH` audits, and each later one as
 * many as the batch before moved in `BATCH_MS` (`nextBatchSize`), so a batch holds the write lock about that long
 * however loaded the machine and however large each audit's provenance.
 */
async function moveInBatches(
  database: RerailDatabase,
  upTo: number,
  { most, pause, now, signal, postponed }: Batching,
): Promise<{ moved: number; ended: boolean }> {
  let moved = 0;
  let after = Number.MIN_SAFE_INTEGER;
  let size = Math.min(FIRST_BATCH, most);
  for (;;) {
    await pause();
    if (signal?.aborted === true || postponed()) return { moved, ended: false };
    const batch = timedBatch(() => moveBatch(database, { after, upTo, limit: size }), now);
    if (batch === BUSY) continue;
    if (batch.done === undefined) return { moved, ended: true };
    sayLeftAsItIs(database.$client.name, batch.done.left);
    moved += batch.done.moved;
    after = batch.done.last;
    size = nextBatchSize(size, batch.took, most);
  }
}

/** What `timedBatch` gives back for a batch another connection's lock was in the way of. */
const BUSY = Symbol("busy");

/**
 * What `batch` did and how long it held the write lock by `now`, or `BUSY` when another connection held what it needed
 * (`isBusy`), which a later try may find free.
 */
function timedBatch<T>(batch: () => T, now: () => number): { done: T; took: number } | typeof BUSY {
  const began = now();
  try {
    const done = batch();
    return { done, took: now() - began };
  } catch (error) {
    if (isBusy(error)) return BUSY;
    throw error;
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
 * once no audit is left to move or `signal` stops it. Its batches and checkpoints then never run on the calling
 * thread, which in the daemon answers the requests hooks send: on a loaded Windows machine one batch of 100 audits took
 * 3.6 s, past the 2 s a hook's health probe waits. `signal` stops the move before its next step. A file with nothing
 * to move, read on the calling thread, starts no thread. What the calling thread reads of the file is one audit row a
 * read (`moveState`), however long the history, and one more for each audit whose damaged provenance a move left as it
 * is.
 *
 * While the last recorded audit that names no stored copy of its provenance began within `olderWindowMs`
 * (`OLDER_WRITE_WINDOW_MS` unless a test passes its own), one recorded before the step or by an older Rerail since,
 * this thread starts no move: stderr says why and until when, and it waits on a timer that holds nothing open, reading
 * the file and the clock again at least once in `OLDER_WAIT_PASS_MS` (`olderPass`). A move such an audit stopped
 * before its next batch waits the same way; one that ended with work left for any other reason goes on at the next
 * start, as before.
 */
export async function moveRuleProvenancesInWorker(
  path: string,
  signal?: AbortSignal,
  options: MoveInWorkerOptions = {},
): Promise<number> {
  const older = olderWritesOf(options);
  const waitedFor = olderPass(path, older, options.timer, signal);
  const stateNow = (): MoveState => (signal?.aborted === true || !existsSync(path) ? NONE : moveState(path, older));
  let moved = 0;
  for (let state = stateNow(); state.kind !== "none";) {
    if (state.kind === "wait") {
      if (!(await waitedFor(state))) return moved;
      state = stateNow();
      continue;
    }
    moved += await movedInWorker({ path, olderWindowMs: older.windowMs }, signal);
    state = waitsOrEnds(stateNow());
  }
  return moved;
}

/** The state after a move's thread ended: a wait for an older Rerail as it is, anything else the end of this run. */
function waitsOrEnds(state: MoveState): MoveState {
  return state.kind === "wait" ? state : NONE;
}

/** How many audits a move on a thread of its own moved, stopped before its next step once `signal` aborts. */
async function movedInWorker(data: MoveWorkerData, signal: AbortSignal | undefined): Promise<number> {
  const worker = moveWorker(data);
  const stop = (): void => worker.postMessage(MOVE_STOP);
  signal?.addEventListener("abort", stop, { once: true });
  try {
    return await movedBy(worker);
  } finally {
    signal?.removeEventListener("abort", stop);
  }
}

/**
 * One pass of the wait for an older Rerail. It says why the move waits and until when (`sayWaitsForOlder`) when the
 * audit it waits for is not the one it named last: on its first pass, and on each later pass that finds another, as
 * while an older Rerail keeps recording. Then it sleeps on `timer` (Node's, unreferenced, unless a test passes its own)
 * until `wait.until` by `older.clock`, or `OLDER_WAIT_PASS_MS`, whichever comes first, so such a Rerail adds at most
 * one line an hour. False when `signal` ends the pass; what else ends the timer is the move's failure.
 */
function olderPass(
  path: string,
  older: OlderWrites,
  timer: MoveTimer | undefined,
  signal: AbortSignal | undefined,
): (wait: OlderWait) => Promise<boolean> {
  const sleepFor = timer ?? unreferencedSleep;
  let named: number | undefined;
  return async wait => {
    if (wait.at !== named) sayWaitsForOlder(path, wait, older.windowMs);
    named = wait.at;
    try {
      await sleepFor(Math.min(Math.max(0, wait.until - older.clock()), OLDER_WAIT_PASS_MS), signal);
      return true;
    } catch (error) {
      if (signal?.aborted === true) return false;
      throw error;
    }
  };
}

/**
 * The line for a move that waits for a Rerail older than the step, with when it starts. The audit it names can be one
 * recorded before the step, by this Rerail before its upgrade.
 */
function sayWaitsForOlder(path: string, { at, until }: { at: number; until: number }, windowMs: number): void {
  process.stderr.write(`rerail: ${path}: moving the rule provenance of audits recorded before this Rerail's upgrade waits `
    + `until ${new Date(until).toISOString()}: the last audit a Rerail older than this one recorded, which reads each `
    + `audit's rule provenance only from the audit's own row, began at ${new Date(at).toISOString()}. The move starts `
    + `once no such audit has begun for ${duration(windowMs)}.\n`);
}

/**
 * The line for each audit a batch left as it is (`moveBatch`), from the batch that reaches it. A later batch of the
 * same move starts after it, and once that move has ended a later start's checks pass over it (`takes`), so a move
 * that runs to its end names it once. A move stopped before its end starts again at the next start from the first
 * audit that names no stored copy, and names it again; a file whose audits left to move are all such audits starts no
 * move, and no line names them. The move's thread writes it to its stderr, which Node pipes to the daemon's,
 * `daemon.log`.
 */
function sayLeftAsItIs(path: string, auditIds: readonly string[]): void {
  for (const auditId of auditIds) {
    process.stderr.write(`rerail: ${path}: moving the rule provenance of audits recorded before this Rerail's upgrade `
      + `leaves audit ${auditId} as it is: the provenance in its row is not a JSON array, so no stored copy can hold it, `
      + "and the audit reads as it did.\n");
  }
}

/**
 * The worker beside this module (`rule-provenance-move-worker.ts`): its JavaScript as the package ships it, or, where
 * this module runs from its TypeScript source as the tests run it, that source through `tsx`.
 */
function moveWorker(data: MoveWorkerData): Worker {
  const fromSource = new URL(import.meta.url).pathname.endsWith(".ts");
  const entry = new URL(`./rule-provenance-move-worker.${fromSource ? "ts" : "js"}`, import.meta.url);
  return new Worker(entry, { workerData: data, ...(fromSource ? { execArgv: ["--import", "tsx"] } : {}) });
}

/** How many audits `worker` moved, once it ends; what it threw, when it fails. */
function movedBy(worker: Worker): Promise<number> {
  return new Promise((resolve, reject) => {
    let moved = 0;
    worker.on("message", (message: unknown) => {
      if (typeof message === "number") moved = message;
    });
    worker.once("error", reject);
    worker.once("exit", code => {
      if (code === 0) resolve(moved);
      else reject(new Error(`the thread moving audit rule provenance exited with code ${code}`));
    });
  });
}

/**
 * What `step`, the checkpoint after the last batch, returns, tried again after each `pause` while another connection is
 * in its way (`isBusy`): a read that keeps it from emptying the WAL, or a writer that holds the lock. After
 * `BUSY_TRIES` tries it throws what the last try threw, as a connection that waits out its busy timeout fails. A stop
 * that comes while it waits ends it before the next try with `STOPPED`, as a stop between two batches ends the move:
 * the step's work is left for the next run, and the stop is not a failure.
 */
async function whenFree<T>(
  step: () => T,
  pause: () => Promise<void>,
  signal: AbortSignal | undefined,
): Promise<T | typeof STOPPED> {
  for (let tries = 1; ; tries += 1) {
    try {
      return step();
    } catch (error) {
      if (!isBusy(error) || tries >= BUSY_TRIES) throw error;
    }
    await pause();
    if (signal?.aborted === true) return STOPPED;
  }
}

/** A checkpoint that another connection's read kept from emptying the WAL. */
class WalInUse extends Error {
  constructor() {
    super("another connection kept the WAL from being emptied");
  }
}

/** Whether `error` says another connection held what the statement needed, which a later try may find free. */
function isBusy(error: unknown): boolean {
  return error instanceof WalInUse || (error instanceof Database.SqliteError && error.code.startsWith("SQLITE_BUSY"));
}

/**
 * Writes the WAL into the database file and empties it (`wal_checkpoint(TRUNCATE)`); throws `WalInUse` when it could
 * not. A TRUNCATE checkpoint holds the write lock while it copies the WAL into the file, so a PASSIVE one, which takes
 * no write lock, copies what it can first.
 */
function emptyWal(client: Client): void {
  client.pragma("wal_checkpoint(PASSIVE)");
  const [checkpoint] = client.pragma("wal_checkpoint(TRUNCATE)") as { busy: number }[];
  if (checkpoint?.busy !== 0) throw new WalInUse();
}

/**
 * Empties the WAL the move's batches filled (36 MB for 4,000 audits) into the file, so the file holds the zeroed pages
 * and the WAL takes no more disk than the daemon's own writes need. A read another connection holds through every try,
 * or a stop between two tries, leaves the WAL as it is: SQLite's later checkpoints write it once that read ends.
 */
async function emptyWalAfterMove(client: Client, free: Free): Promise<void> {
  try {
    await free(() => emptyWal(client));
  } catch (error) {
    if (!isBusy(error)) throw error;
  }
}

/** When the file ran `audit-rule-provenance-once` (epoch ms), or undefined when it has no record of that. */
function stepAppliedAt(database: RerailDatabase): number | undefined {
  return database.$client.prepare<[string], number>("SELECT applied_at FROM schema_migrations WHERE id = ?").pluck()
    .get(RULE_PROVENANCE_STEP);
}

/**
 * The audits that name no stored copy of their provenance, through `audits_rule_provenance`, which lists them by rowid,
 * the order they were recorded in: a statement that takes them in that order reads only the rows it returns. The hint
 * keeps that plan whatever statistics an `ANALYZE` leaves, and makes a file without the index fail the statement
 * rather than read every audit's row; this build's DDL creates the index on every writable open, the daemon's included.
 */
const UNNAMED = "audits INDEXED BY audits_rule_provenance WHERE rule_provenance_id IS NULL";

/**
 * Whether the text `value` names is a JSON array, which a stored copy must be (the CHECK on `audit_rule_provenances`).
 * Only a damaged file or a hand edit leaves other text in an audit's row, since the same CHECK on `audits` refuses it
 * on insert; the move leaves such an audit as it is (`moveBatch`). `json_valid` comes first: `json_type` fails the
 * statement on text that is not JSON.
 */
const holdable = (value: string): string => `CASE WHEN json_valid(${value}) THEN json_type(${value}) = 'array' ELSE 0 END`;

/** Which audits one batch takes: up to `limit` after rowid `after`, of those that began no later than `upTo`. */
interface BatchRange {
  readonly after: number;
  readonly upTo: number;
  readonly limit: number;
}

/** An audit a batch takes (`takenAfter`): its rowid, its id and the text of its provenance. */
interface TakenAudit {
  readonly row: number;
  readonly auditId: string;
  readonly json: string;
}

/**
 * What the file asks of the move (`MoveState`), read on a read-only connection: once the move has ended, each later
 * daemon start finds nothing and opens no writer, so it takes no write lock. A read-only open runs a pending step
 * through a writable open first, as it does for any reader. Each of its reads is one audit row, however long the
 * history (`takes`, `lastOlderWrite`), and `takes` reads one more for each audit whose damaged provenance a move left
 * as it is.
 */
function moveState(path: string, older: OlderWrites): MoveState {
  const database = openRerailDatabase(path, { readonly: true });
  try {
    const upTo = stepAppliedAt(database);
    return upTo === undefined ? NONE : stateAt(database.$client, upTo, older);
  } finally {
    database.$client.close();
  }
}

/**
 * Nothing when no audit is left to take; a wait while audits are left to take and the last recorded audit that names
 * no stored copy began within the window (`lastOlderWrite`); else the move.
 */
function stateAt(client: Client, upTo: number, older: OlderWrites): MoveState {
  if (!takes(client, upTo)) return NONE;
  const at = lastOlderWrite(client, older);
  return at === undefined ? MOVE : { kind: "wait", at, until: at + older.windowMs };
}

/**
 * Whether an audit the move takes is left: the first recorded audit that names no stored copy, past those the move
 * leaves as they are (`holdable`), began no later than the step (`upTo`). The move takes them in the order they were
 * recorded, up to the first that began after the step, so once that one comes first none is left. One row, and one
 * more for each audit left as it is before it: the start of every audit an older Rerail recorded after the step sits
 * after its provenance, and reading the start of each of 2,400 such audits took 1.1 s on a cold cache. An audit left as
 * it is is read in full and its text parsed, at every start: about 0.1 ms, and 0.4 ms on a cold cache, for one of
 * 189 KB on Linux x64. Only a damaged file or a hand edit makes one.
 */
function takes(client: Client, upTo: number): boolean {
  const firsts = client.prepare<[number], number | null>(
    `SELECT CASE WHEN started_at <= ? THEN ${holdable("rule_provenance_json")} END FROM ${UNNAMED} ORDER BY rowid`).pluck();
  for (const taken of firsts.iterate(upTo)) if (taken !== 0) return taken === 1;
  return false;
}

/**
 * When the last recorded audit that names no stored copy of its provenance began, when that was less than
 * `older.windowMs` before `older.clock()` or later; undefined otherwise, or when every audit names one. Only a Rerail
 * older than the step writes an audit so, with its provenance in the audit's own row or `[]` there when it ran no
 * rules, and it reads each audit's provenance only from that row. This Rerail's audit store names a stored copy for
 * every audit, `[]` included, so its own audits never count. The audits recorded before the step count as well, until
 * the move takes them: a Rerail that ran then may still run beside this one, and record its next audit days later.
 * Every audit is recorded as it begins, so the last recorded is the one such a Rerail began last: one row however many
 * audits the week holds, where reading the start of each of 50,000 took 69 ms on Linux and 195 ms on Windows.
 */
function lastOlderWrite(client: Client, { clock, windowMs }: OlderWrites): number | undefined {
  const at = client.prepare<[], number>(`SELECT started_at FROM ${UNNAMED} ORDER BY rowid DESC LIMIT 1`).pluck().get();
  return at !== undefined && at > clock() - windowMs ? at : undefined;
}

/**
 * Moves up to `limit` audits after rowid `after` that name no stored copy, up to the first that began after the step
 * (`takenAfter`), in one write transaction: each names the stored copy of its JSON text instead, stored first when no
 * audit had the same text, and keeps `[]` inline. An audit whose provenance is `[]` names the stored copy of `[]`, as
 * the store's audits that run no rules do, so no audit recorded before the step is left naming none. The text moves as
 * it is, so every audit reads the same provenance after. Its key is the SHA-256 of the text, as the audit store keys
 * the provenance it writes, so the store's later audits of the same rules name the copy moved here. An audit whose text
 * no stored copy can hold (`holdable`) stays as it is, naming none, with its text in its row, where both versions read
 * it as before. The last rowid it read, how many it moved and which it left, or undefined when none was left.
 */
function moveBatch(
  database: RerailDatabase,
  { after, upTo, limit }: BatchRange,
): { last: number; moved: number; left: string[] } | undefined {
  const client = database.$client;
  return client.transaction(() => {
    const rows = takenAfter(client, { after, upTo, limit });
    const copyOf = storedCopyOf(client);
    const name = client.prepare<[number, number]>("UPDATE audits SET rule_provenance_id = ?, rule_provenance_json = '[]' WHERE rowid = ?");
    const left: string[] = [];
    for (const { row, auditId, json } of rows) {
      const copy = copyOf(json);
      if (copy === undefined) left.push(auditId);
      else name.run(copy, row);
    }
    const last = rows.at(-1);
    return last === undefined ? undefined : { last: last.row, moved: rows.length - left.length, left };
  }).immediate();
}

/**
 * The id of the stored copy of a provenance's text, stored first when no audit had the same text, or undefined when no
 * stored copy can hold that text (`holdable`). A text found by its key is one already stored, so only a new one is read
 * as JSON.
 */
function storedCopyOf(client: Client): (json: string) => number | undefined {
  const stored = client.prepare<[string], number>("SELECT provenance_id FROM audit_rule_provenances WHERE digest = ?").pluck();
  const store = client.prepare<{ digest: string; json: string }>(
    `INSERT INTO audit_rule_provenances (digest, rule_provenance_json) SELECT @digest, @json WHERE ${holdable("@json")}`);
  return json => {
    const digest = createHash("sha256").update(json).digest("hex");
    const copy = stored.get(digest);
    if (copy !== undefined) return copy;
    const inserted = store.run({ digest, json });
    return inserted.changes === 0 ? undefined : Number(inserted.lastInsertRowid);
  };
}

/**
 * Up to `limit` audits after rowid `after` that name no stored copy of their provenance, in the order they were
 * recorded, each with its id and its provenance's text, ending before the first that began after the step (`upTo`).
 * From that one on, each such audit is one a Rerail older than the step recorded after it, whose provenance stays in
 * its row: the batch reads none of their rows past the first. Under the batch's write lock, reading the start of each
 * of 2,000 such rows took 125 ms on Linux and 470 ms on Windows.
 */
function takenAfter(client: Client, { after, upTo, limit }: BatchRange): TakenAudit[] {
  const audits = client.prepare<[number, number, number], { row: number; auditId: string; json: string | null }>(`SELECT
    rowid AS row, audit_id AS auditId, CASE WHEN started_at <= ? THEN rule_provenance_json END AS json FROM ${UNNAMED}
    AND rowid > ? ORDER BY rowid LIMIT ?`);
  const taken: TakenAudit[] = [];
  for (const { row, auditId, json } of audits.iterate(upTo, after, limit)) {
    if (json === null) break;
    taken.push({ row, auditId, json });
  }
  return taken;
}

/** `ms` in whole days from two days up, in whole minutes from one minute up, else in seconds. */
function duration(ms: number): string {
  if (ms >= 2 * DAY_MS) return `${Math.round(ms / DAY_MS)} days`;
  return ms >= 60_000 ? `${Math.round(ms / 60_000)} minutes` : `${ms / 1_000} s`;
}
