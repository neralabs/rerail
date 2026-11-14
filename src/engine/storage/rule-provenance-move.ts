/**
 * Moves the rule provenance that audits recorded before `audit-rule-provenance-once` hold inline
 * (`rule_provenance_json`) to the one copy `audit_rule_provenances` keeps. It runs after the open, never inside it:
 * the daemon starts it once it serves (`daemon.ts`), on a worker thread of its own (`moveRuleProvenancesInWorker`), so
 * none of its SQLite work holds the thread that answers requests. Each batch of audits is one write transaction sized
 * to hold the lock about 100 ms (`moveInBatches`), with a pause before it, so other writers keep recording while it
 * runs; a process stopped partway loses at most one batch, and the next run goes on from what is left. Until an audit
 * moves, the audit store reads its provenance inline. The move never waits inside SQLite for another connection: it
 * awaits a pause and tries again (`whenFree`), so a stop reaches it between two tries.
 *
 * The pages a batch frees stay in the file, zeroed as every delete's are (`secure_delete`, `database.ts`), and new
 * history reuses them before the file grows again. No `VACUUM` runs here: one holds the write lock until it ends, and
 * another process's write fails once it has waited 5 s. `rerail scrub` gives the space back when a person runs it.
 *
 * A Rerail older than the step reads an audit's provenance only from the audit's own row, so it reads `[]` for each
 * audit the move took it from. The move therefore waits while such a Rerail may still be using the file: until
 * `OLDER_WRITE_WINDOW_MS` has passed since the last audit that names no stored copy began, with rules or without, those
 * recorded before the step included (`lastOlderWrite`), checked before the move starts and again before each batch.
 * This Rerail's audit store names a stored copy for every audit it writes, `[]` included, so only an older Rerail
 * writes an audit that names none. With only this Rerail running, the move starts a week after the last audit
 * recorded before the step.
 */
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { Worker } from "node:worker_threads";
import Database from "better-sqlite3";
import { openRerailDatabase, type RerailDatabase } from "./database.js";
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
 * How many times the move tries a step while another connection is in its way: with the daemon's pause, the 5 s every
 * Rerail connection waits (`busy_timeout`, `database.ts`). Past that it gives up as such a connection does.
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
 * longer than 2^31 - 1 ms (about 24.9 days) after 1 ms, which an audit dated weeks past the clock would ask for. A timer
 * counts only the time the machine is awake on Linux and macOS, so after a sleep the move starts at most this late.
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
  /** Stops the move before its next batch; what it moved stays moved. */
  readonly signal?: AbortSignal;
  /** The time in epoch milliseconds, to which an older Rerail's last audit (`lastOlderWrite`) is compared; for tests. */
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
 * What a file asks of the move: nothing; to move audits; or to wait until `until`, since an audit that names no stored
 * copy of its provenance, as an older Rerail writes each audit, began at `at`, less than the window before.
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

/** Runs a step on the move's connection, trying it again after a pause while another connection is in its way. */
type Free = <T>(step: () => T) => Promise<T>;

/**
 * Moves the provenance of every audit that held it inline when the file ran `audit-rule-provenance-once`, then empties
 * the WAL its batches filled into the file (`emptyWalAfterMove`). Audits that an older Rerail sharing the file writes
 * after that step keep theirs inline, where both versions read it. While an audit that names no stored copy, as an
 * older Rerail writes each audit, has begun within the window, one recorded before the step included, the move takes
 * nothing, or stops before its next batch (`lastOlderWrite`). A file that does not exist yet holds nothing to move,
 * and the move does not create it. Resolves to how many audits it moved.
 */
export async function moveRuleProvenances(path: string, options: RuleProvenanceMoveOptions = {}): Promise<number> {
  const older = olderWritesOf(options);
  if (!existsSync(path) || moveState(path, older).kind !== "move") return 0;
  const database = openRerailDatabase(path);
  try {
    // From here SQLite reports another connection's lock at once, and `whenFree` waits for it between tries.
    database.$client.pragma("busy_timeout = 0");
    const upTo = stepAppliedAt(database);
    if (upTo === undefined) return 0;
    const client = database.$client;
    const batching = batchingOf(options, () => takes(client, upTo) && lastOlderWrite(client, older) !== undefined);
    const free: Free = step => whenFree(step, batching.pause, options.signal);
    const done = await moveInBatches(database, upTo, free, batching);
    if (done.ended && done.moved > 0) await emptyWalAfterMove(client, free);
    return done.moved;
  } finally {
    database.$client.close();
  }
}

/** The clock and window `options` give for an older Rerail's last audit, else the time now and `OLDER_WRITE_WINDOW_MS`. */
function olderWritesOf(options: Pick<RuleProvenanceMoveOptions, "clock" | "olderWindowMs">): OlderWrites {
  return { clock: options.clock ?? (() => Date.now()), windowMs: options.olderWindowMs ?? OLDER_WRITE_WINDOW_MS };
}

/** How the batches of a move run, from `options` and `postponed`, which says when an older Rerail's audit stops them. */
function batchingOf(options: RuleProvenanceMoveOptions, postponed: () => boolean): Batching {
  const { signal } = options;
  const pause = options.pause ?? (() => sleep(PAUSE_MS, undefined, { signal }).catch(() => undefined));
  const now = options.now ?? (() => performance.now());
  return { most: options.perBatch ?? MOVED_PER_BATCH, pause, now, signal, postponed };
}

/**
 * Moves the audits that started no later than `upTo`, a batch per write transaction with a pause before each, until
 * none is left, or `signal` or an older Rerail's audit (`postponed`) stops it: how many it moved, and whether none is
 * left. The first batch takes `FIRST_BATCH` audits, and each later one as many as the batch before moved in `BATCH_MS`
 * (`nextBatchSize`), so a batch holds the write lock about that long however loaded the machine and however large each
 * audit's provenance.
 */
async function moveInBatches(
  database: RerailDatabase,
  upTo: number,
  free: Free,
  { most, pause, now, signal, postponed }: Batching,
): Promise<{ moved: number; ended: boolean }> {
  let moved = 0;
  let after = Number.MIN_SAFE_INTEGER;
  let size = Math.min(FIRST_BATCH, most);
  for (;;) {
    await pause();
    if (signal?.aborted === true || postponed()) return { moved, ended: false };
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
 * once that thread ends. Its batches and checkpoints then never run on the calling thread, which in the daemon answers
 * the requests hooks send: on a loaded Windows machine one batch of 100 audits took 3.6 s, past the 2 s a hook's health
 * probe waits. `signal` stops the move before its next step. A file with nothing to move, read on the calling thread,
 * starts no thread.
 *
 * While an audit that names no stored copy of its provenance has begun within `olderWindowMs` (`OLDER_WRITE_WINDOW_MS`
 * unless a test passes its own), recorded before the step or by an older Rerail since, this thread starts no move:
 * stderr says why and until when, and it waits on a timer that holds nothing open, reading the file and the clock again
 * at least once in `OLDER_WAIT_PASS_MS` (`olderPass`). A move such an audit stopped before its next batch waits the
 * same way; one that ended with work left for any other reason goes on at the next start, as before.
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
 * One pass of the wait for an older Rerail: says why the move waits and until when (`sayWaitsForOlder`) when the audit
 * it waits for is not the one it named last, so a wait says it once, then sleeps on `timer` (Node's, unreferenced,
 * unless a test passes its own) until `wait.until` by `older.clock`, or `OLDER_WAIT_PASS_MS`, whichever comes first.
 * False when `signal` ends the pass.
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
    } catch {
      return false;
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
 * Writes the WAL into the database file and empties it (`wal_checkpoint(TRUNCATE)`); throws `WalInUse` when it could not.
 * A TRUNCATE checkpoint holds the write lock while it copies the WAL into the file, so a PASSIVE one, which takes no
 * write lock, copies what it can first.
 */
function emptyWal(client: Client): void {
  client.pragma("wal_checkpoint(PASSIVE)");
  const [checkpoint] = client.pragma("wal_checkpoint(TRUNCATE)") as { busy: number }[];
  if (checkpoint?.busy !== 0) throw new WalInUse();
}

/**
 * Empties the WAL the move's batches filled (36 MB for 4,000 audits) into the file, so the file holds the zeroed pages
 * and the WAL takes no more disk than the daemon's own writes need. A read another connection holds through every try
 * leaves the WAL as it is: SQLite's later checkpoints write it once that read ends.
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

/** The audits the move takes: those holding their provenance inline that started no later than the step ran (`?`). */
const TAKEN = "started_at <= ? AND rule_provenance_id IS NULL AND rule_provenance_json <> '[]'";

/**
 * What the file asks of the move (`MoveState`), read on a read-only connection: once the move has ended, each later
 * daemon start finds nothing and opens no writer, so it takes no write lock. A read-only open runs a pending step through
 * a writable open first, as it does for any reader.
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
 * Nothing when no audit is left to take; a wait while audits are left to take and an audit that names no stored copy
 * began within the window (`lastOlderWrite`); else the move.
 */
function stateAt(client: Client, upTo: number, older: OlderWrites): MoveState {
  if (!takes(client, upTo)) return NONE;
  const at = lastOlderWrite(client, older);
  return at === undefined ? MOVE : { kind: "wait", at, until: at + older.windowMs };
}

/** Whether an audit the move takes is left. */
function takes(client: Client, upTo: number): boolean {
  return client.prepare<[number], number>(`SELECT 1 FROM audits WHERE ${TAKEN} LIMIT 1`).pluck().get(upTo) !== undefined;
}

/**
 * When the last audit began that names no stored copy of its provenance, of those that began within `older.windowMs`
 * of `older.clock()`; undefined when none did. Only a Rerail older than the step writes an audit so, with its
 * provenance in the audit's own row or `[]` there when it ran no rules, and it reads each audit's provenance only from
 * that row. This Rerail's audit store names a stored copy for every audit, `[]` included, so its own audits never
 * count. The audits recorded before the step count as well: a Rerail that ran then may still run beside this one, and
 * record its next audit days later. The audits that began in the window come from `audits_session_started`, which
 * holds each one's start, and only their rows are read, the latest first: a row that holds its provenance runs to
 * hundreds of kilobytes with the start after it, so reading the start of every recorded audit from its row would read
 * the whole history.
 */
function lastOlderWrite(client: Client, { clock, windowMs }: OlderWrites): number | undefined {
  const began = client.prepare<[number], { row: number; at: number }>(
    "SELECT rowid AS row, started_at AS at FROM audits INDEXED BY audits_session_started WHERE started_at > ?",
  ).all(clock() - windowMs);
  const older = client.prepare<[number], number>("SELECT 1 FROM audits WHERE rowid = ? AND rule_provenance_id IS NULL").pluck();
  return began.sort((one, other) => other.at - one.at).find(({ row }) => older.get(row) !== undefined)?.at;
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

/** `ms` in whole days from two days up, in whole minutes from one minute up, else in seconds. */
function duration(ms: number): string {
  if (ms >= 2 * DAY_MS) return `${Math.round(ms / DAY_MS)} days`;
  return ms >= 60_000 ? `${Math.round(ms / 60_000)} minutes` : `${ms / 1_000} s`;
}
