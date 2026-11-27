/**
 * Record writes while another process holds the Rerail database's write lock, as `rerail reset`, `rerail scrub` or a
 * newer Rerail's migration does for seconds: the stores' writes return at once on the calling thread, and are made in
 * the order they were asked for once the lock is given back. Before, each slept in SQLite's busy handler for 5 s on the
 * daemon's thread that answers hooks, then failed and the record was lost. A read of what was recorded makes the
 * waiting writes first, and no thread waits for a write longer than the 5 s bound from when it was asked.
 */
import { fork } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditControlStore, type PolicyOwnerClaim } from "../../../src/engine/audit/audit-control.js";
import type { AuditInput } from "../../../src/engine/audit/audit.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { BUSY_TIMEOUT_MS, failWhileWriteLocked, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { hookFailureLines } from "../../../src/harness/index.js";
import { lastObservedEventAt, SqliteSessionStore } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const session = { sessionId: "held-session", host: "claude-code" as const, cwd: "/work" };
const auditInput: AuditInput = {
  sessionId: session.sessionId, workspaceRoot: "/work", scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "r1", ruleRevisions: [], paths: ["a.py"],
};
const roots: string[] = [];
const closers: Array<() => Promise<void> | void> = [];

/** The word the next write lock wait's thread holds its try with (`holdNextLockWaitTry`), while one is set. */
const heldTries = vi.hoisted(() => ({ next: undefined as Int32Array | undefined }));
vi.mock("node:worker_threads", async importOriginal => {
  const actual = await importOriginal<typeof import("node:worker_threads")>();
  const preload = new URL("../../support/engine/held-lock-wait-try.ts", import.meta.url).href;
  class Worker extends actual.Worker {
    constructor(script: string | URL, options: import("node:worker_threads").WorkerOptions = {}) {
      const heldTry = heldTries.next;
      heldTries.next = undefined;
      super(script, heldTry === undefined ? options : {
        ...options,
        execArgv: [...(options.execArgv ?? []), "--import", preload],
        workerData: { ...(options.workerData as object), heldTry },
      });
    }
  }
  return { ...actual, Worker };
});

/**
 * Holds the first try of the next write lock wait started here that takes the lock (`held-lock-wait-try.ts`), as a
 * thread the system stops between its two statements holds it: `held` settles once that try holds the lock, and
 * `release` lets it go.
 */
function holdNextLockWaitTry(): { held: Promise<void>; release: () => void } {
  const word = new Int32Array(new SharedArrayBuffer(8));
  heldTries.next = word;
  const taken = Atomics.waitAsync(word, 0, 0);
  const release = (): void => {
    heldTries.next = undefined;
    Atomics.store(word, 1, 1);
    Atomics.notify(word, 1);
  };
  closers.push(release);
  return { held: taken.async ? taken.value.then(() => undefined) : Promise.resolve(), release };
}

afterEach(async () => {
  for (const close of closers.splice(0).reverse()) await close();
  vi.restoreAllMocks();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** A process of its own that takes the write lock of `path` (`BEGIN IMMEDIATE`) and gives it back on request. */
async function lockHolder(path: string): Promise<{ hold(): Promise<void>; release(): Promise<void> }> {
  const script = join(dirname(path), "holder.cjs");
  await writeFile(script, [
    "const Database = require(process.argv[2]);",
    "const database = new Database(process.argv[3]);",
    "process.on('message', (command) => {",
    "  if (command === 'hold') database.exec('BEGIN IMMEDIATE');",
    "  if (command === 'release' && database.inTransaction) database.exec('COMMIT');",
    "  process.send(command);",
    "});",
    "process.send('ready');",
  ].join("\n"));
  const child = fork(script, [createRequire(import.meta.url).resolve("better-sqlite3"), path], { stdio: ["ignore", "inherit", "inherit", "ipc"] });
  closers.push(async () => {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, "exit");
    child.kill();
    await exited;
  });
  const reply = async (command: string): Promise<void> => {
    for (;;) if ((await once(child, "message"))[0] === command) return;
  };
  await reply("ready");
  const ask = async (command: string): Promise<void> => {
    const replied = reply(command);
    child.send(command);
    await replied;
  };
  return { hold: () => ask("hold"), release: () => ask("release") };
}

/**
 * A connection of this process holding the write lock: it gives the lock back between two calls of one turn, before
 * any lock wait's answer can arrive, or keeps it past any wait on this thread, since this thread is the one waiting.
 */
function holderHere(path: string): Database.Database {
  const holder = new Database(path);
  closers.push(() => { holder.close(); });
  holder.exec("BEGIN IMMEDIATE");
  return holder;
}

async function stores(): Promise<{ path: string; sessions: SqliteSessionStore; audits: SqliteAuditStore }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-record-writes-"));
  roots.push(root);
  const path = join(root, "rerail.sqlite");
  const sessions = new SqliteSessionStore(path);
  const audits = new SqliteAuditStore(path);
  closers.push(() => audits.close(), () => sessions.close());
  return { path, sessions, audits };
}

/** The record writer of `path`, imported here so the file's other cases run on a tree without the module. */
async function recordWrites(): Promise<typeof import("../../../src/engine/storage/record-writes.js")> {
  return import("../../../src/engine/storage/record-writes.js");
}

/** Resolves once no record write of this process to `path` waits. */
async function settled(path: string): Promise<void> {
  const writer = (await recordWrites()).openRecordWriter(path);
  try {
    await writer.settled();
  } finally {
    writer.close();
  }
}

/**
 * The clock the record writer counts its 5 s bound on, set by the test: every write asked at 0, and `pastTheBound`
 * moves it 10 s on, so a wait on this thread for those writes is over before it starts.
 */
function boundClock(): { pastTheBound(): void } {
  const now = vi.spyOn(performance, "now").mockReturnValue(0);
  return { pastTheBound: () => { now.mockReturnValue(10_000); } };
}

/** Milliseconds `run` kept this thread, on a clock the test does not set. */
function msOf(run: () => void): number {
  const started = process.hrtime.bigint();
  run();
  return Number(process.hrtime.bigint() - started) / 1e6;
}

function thrown(run: () => void): unknown {
  try {
    run();
  } catch (error) {
    return error;
  }
  return undefined;
}

/**
 * A remote outbox beside the database at `path` whose cursor says a remote holds every event numbered up to `taken`, as
 * a power cut that took the store's last commits leaves it. Only the table a session's first open reads
 * (`remote-cursors.ts`).
 */
function remoteHolds(path: string, taken: number): void {
  const outbox = new Database(join(dirname(path), "remote-outbox.sqlite"));
  try {
    outbox.exec("CREATE TABLE cursors (stream TEXT PRIMARY KEY, value INTEGER NOT NULL)");
    outbox.prepare("INSERT INTO cursors (stream, value) VALUES ('event', ?)").run(taken);
  } finally {
    outbox.close();
  }
}

/** A tool call event of the test's session, observed at `observedAt`, and its line in the session's ledger. */
function callEvent(eventId: string, observedAt: number) {
  return {
    event: { sessionId: session.sessionId, eventId, kind: "call", observedAt, payload: {} },
    line: { eventId, kind: "call", observedAt },
  };
}

/**
 * The ledger's lines after `recorded`, as the daemon walks them for lost events at a session's open (`lostEvents` in
 * `runtime.ts`): none when the ledger does not hold it.
 */
function linesAfter<Line extends { eventId: string }>(ledger: readonly Line[], recorded: string | undefined): Line[] {
  const from = ledger.findIndex(line => line.eventId === recorded);
  return from < 0 ? [] : ledger.slice(from + 1);
}

type CallEvent = ReturnType<typeof callEvent>;

/**
 * The daemon's reopen of the test's session over its ledger `ledger`, oldest first: its open, then a record of the
 * ledger's events after the last recorded one (`lostEvents` in `runtime.ts`).
 */
function reopen(sessions: SqliteSessionStore, ledger: readonly CallEvent[]): void {
  sessions.openSession(session, 9_000);
  const recorded = sessions.lastRecordedEvent(session.sessionId, new Set(["call"]));
  sessions.recordLostEvents(session, linesAfter(ledger.map(({ line }) => line), recorded));
}

/**
 * The test's session, its ledger `ledger`, reopened while another connection holds the write lock past the bound,
 * then every write made once the lock is given back. The row of the ledger's first event is made; those of `waiting`
 * are asked for behind the holder; an event in neither never had its row asked for.
 */
async function reopenedPastTheBound(ledger: readonly CallEvent[], waiting: readonly CallEvent[]): Promise<{ path: string; sessions: SqliteSessionStore }> {
  const { path, sessions } = await stores();
  sessions.openSession(session, 1_000);
  sessions.append(ledger[0]!.event, session);
  const holder = holderHere(path);
  const clock = boundClock();
  for (const { event } of waiting) sessions.append(event, session);
  clock.pastTheBound();
  reopen(sessions, ledger);
  holder.exec("COMMIT");
  await settled(path);
  return { path, sessions };
}

/** The session's records of lost events, oldest first: id, time and what each counts. */
function lostRecords(path: string): unknown[] {
  const records = rows(path, "SELECT event_id, observed_at, payload_json FROM events WHERE kind = 'events-lost' ORDER BY sequence") as Array<[string, number, string]>;
  return records.map(([eventId, observedAt, payload]) => [eventId, observedAt, JSON.parse(payload) as unknown]);
}

/** Counts from now the record writes of this process that commit to `path`, and the write lock takes of any connection. */
async function writesSeen(path: string): Promise<() => { notices: number; lockTakes: number }> {
  let notices = 0;
  closers.push((await recordWrites()).onRecordsWritten(path, () => { notices += 1; }));
  const probe = new Database(":memory:");
  const statement = Object.getPrototypeOf(probe.prepare("SELECT 1")) as Database.Statement;
  probe.close();
  const run = vi.spyOn(statement, "run");
  return () => ({ notices, lockTakes: run.mock.contexts.filter(ran => (ran as Database.Statement).source === "BEGIN IMMEDIATE").length });
}

function rows(path: string, sql: string): unknown[] {
  const database = new Database(path, { readonly: true });
  try {
    return database.prepare(sql).raw().all();
  } finally {
    database.close();
  }
}

describe("record writes while another process holds the write lock", () => {
  it("return at once and are made in the order they were asked for once the lock is given back", async () => {
    const { path, sessions, audits } = await stores();
    const holder = await lockHolder(path);
    await holder.hold();

    const ms = msOf(() => {
      sessions.openSession(session, 1_000);
      sessions.append({ sessionId: session.sessionId, eventId: "edit-call", kind: "call", payload: { tool: "Write" } }, session);
      const auditId = audits.start(auditInput);
      audits.finish(auditId, { status: "completed", violations: [], failures: [] });
    });
    // A write that waited for the lock on this thread would have kept it the 5 s bound.
    expect(ms).toBeLessThan(1_000);
    expect(rows(path, "SELECT session_id FROM sessions")).toEqual([]);

    await holder.release();
    await settled(path);

    // The audit names the session row, which a foreign key demands first: the order held across the two stores.
    expect(rows(path, "SELECT session_id, started_at FROM sessions")).toEqual([[session.sessionId, 1_000]]);
    expect(rows(path, "SELECT event_id FROM events WHERE session_id = 'held-session'")).toEqual([["edit-call"]]);
    expect(rows(path, "SELECT status FROM audits")).toEqual([["completed"]]);
  }, 30_000);

  it("tell who listens for this process's record writes as each commits, at once or once the lock is given back", async () => {
    const { path, sessions } = await stores();
    const { onRecordsWritten } = await recordWrites();
    const told: string[] = [];
    const stop = onRecordsWritten(path, () => { told.push(rows(path, "SELECT session_id FROM sessions").length === 0 ? "before" : "committed"); });
    closers.push(stop);
    sessions.openSession(session, 1_000);
    expect(told).toEqual(["committed"]);

    const holder = await lockHolder(path);
    await holder.hold();
    sessions.append({ sessionId: session.sessionId, eventId: "edit-call", kind: "call", payload: {} }, session);
    expect(told).toEqual(["committed"]);
    await holder.release();
    await settled(path);
    expect(told).toEqual(["committed", "committed"]);

    stop();
    sessions.append({ sessionId: session.sessionId, eventId: "edit-result", kind: "result", payload: {} }, session);
    expect(told).toHaveLength(2);
  }, 30_000);

  it("keep each write's own time, not the time the lock was given back", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = await lockHolder(path);
    await holder.hold();
    const now = vi.spyOn(Date, "now").mockReturnValue(5_000);
    const auditId = audits.start(auditInput);
    sessions.append({ sessionId: session.sessionId, eventId: "edit-result", kind: "result", payload: {} });
    now.mockReturnValue(9_000);

    await holder.release();
    await settled(path);

    expect(rows(path, `SELECT started_at FROM audits WHERE audit_id = '${auditId}'`)).toEqual([[5_000]]);
    expect(rows(path, "SELECT observed_at FROM events WHERE event_id = 'edit-result'")).toEqual([[5_000]]);
  }, 30_000);

  it("interrupt an audit whose start still waits for the lock", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = await lockHolder(path);
    await holder.hold();
    const auditId = audits.start(auditInput);
    audits.interrupt(session.sessionId);

    await holder.release();
    await settled(path);

    expect(rows(path, `SELECT status FROM audits WHERE audit_id = '${auditId}'`)).toEqual([["interrupted"]]);
  }, 30_000);

  it("are made by the last store's close once the lock is free, before the lock wait answers", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = await lockHolder(path);
    await holder.hold();
    sessions.append({ sessionId: session.sessionId, eventId: "last-event", kind: "call", payload: {} });
    await holder.release();

    audits.close();
    sessions.close();

    expect(rows(path, "SELECT event_id FROM events WHERE event_id = 'last-event'")).toEqual([["last-event"]]);
  }, 30_000);

  it("drop and report a write past what may wait, and make the rest", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const holder = await lockHolder(path);
    await holder.hold();
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    writer.write(database => database.$client.prepare("UPDATE sessions SET ended_at = 2000").run(), 64 * 1024 * 1024 + 1);
    sessions.closeSession(session.sessionId, 3_000);

    await holder.release();
    await writer.settled();

    expect(rows(path, "SELECT ended_at FROM sessions")).toEqual([[3_000]]);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("1 records not written"));
  }, 30_000);

  // A steer's receipt and the removal of its kept file after it: the removal, made without the receipt, lost it.
  it("drop every later write once one is dropped behind waiting writes, until those are made", async () => {
    const { path } = await stores();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const { openRecordWriter, flushRecordWrites } = await recordWrites();
    const writer = openRecordWriter(path);
    closers.push(() => writer.close());
    const made: string[] = [];
    const named = (name: string) => () => { made.push(name); };
    const holder = holderHere(path);
    writer.write(named("earlier"), 64 * 1024 * 1024 - 10);
    writer.write(named("receipt"), 100);
    writer.write(named("forget"), 0);
    holder.exec("COMMIT");
    flushRecordWrites(path);
    expect(made).toEqual(["earlier"]);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("2 records not written"));
    // Once what waited is made, a write waits again.
    holder.exec("BEGIN IMMEDIATE");
    writer.write(named("later"), 0);
    holder.exec("COMMIT");
    flushRecordWrites(path);
    expect(made).toEqual(["earlier", "later"]);
  }, 30_000);

  it("take no write lock for a rule coverage that has not changed, so it never waits behind the holder", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const coverage = { repositoryId: "repo", ownerCheckoutPaths: ["/work"], activeRuleCount: 3, ruleSignature: "s1" };
    sessions.recordRuleCoverage(session, coverage);
    const holder = await lockHolder(path);
    await holder.hold();
    const { requireRecordWritesMade } = await recordWrites();
    const clock = boundClock();

    sessions.recordRuleCoverage(session, coverage);

    // A write kept for the lock would make this throw, as the held lock does, without waiting.
    clock.pastTheBound();
    expect(() => requireRecordWritesMade(path)).not.toThrow();
    await holder.release();
  }, 30_000);

  // A holder of a moment in another process gives the lock back between two calls of one turn on this thread, before
  // the lock wait's answer can arrive here: the reads below come in that gap. Before, the claim found no session row
  // and the session failed to open, the verdict found no audit and called the scan unrecorded, and the work-link answer
  // left out the link it had just been asked to add. The claim, a record write itself, now waits behind the row.
  it("are made before a session's owner claim reads the session's row, when the holder gave the lock back", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    sessions.openSession(session, 1_000);
    holder.exec("COMMIT");

    const claim = controls.registerOwner(session.sessionId, "background");
    await settled(path);

    expect(controls.holdsOwner(session.sessionId, claim)).toBe(true);
    expect(rows(path, "SELECT session_id, ended_at FROM sessions")).toEqual([[session.sessionId, null]]);
  }, 30_000);

  it("are made before a scan's verdict reads its audit, when the holder gave the lock back", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = holderHere(path);
    const auditId = audits.start({ ...auditInput, scope: "repository", mode: "log", paths: [] });
    audits.finish(auditId, { status: "completed", violations: [], failures: [] });
    holder.exec("COMMIT");

    expect(audits.get(auditId)?.status).toBe("completed");
  }, 30_000);

  it("are made before the session store reads the session and its work links, when the holder gave the lock back", async () => {
    const { path, sessions } = await stores();
    const holder = holderHere(path);
    sessions.openSession(session, 1_000);
    sessions.linkWork({ sessionId: session.sessionId, kind: "linear-issue", reference: "ABC-1" }, 2_000);
    holder.exec("COMMIT");

    expect(sessions.session(session.sessionId)?.startedAt).toBe(1_000);
    expect(sessions.workLinks(session.sessionId).map(link => link.reference)).toEqual(["ABC-1"]);
  }, 30_000);

  it("keep no thread waiting for writes asked longer ago than the bound: a read answers what is on disk at once", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const holder = holderHere(path);
    const clock = boundClock();
    sessions.linkWork({ sessionId: session.sessionId, kind: "linear-issue", reference: "ABC-1" }, 2_000);
    clock.pastTheBound();

    let links: readonly unknown[] = [];
    expect(msOf(() => { links = sessions.workLinks(session.sessionId); })).toBeLessThan(1_000);
    expect(links).toEqual([]);

    holder.exec("COMMIT");
    await settled(path);
    expect(sessions.workLinks(session.sessionId).map(link => link.reference)).toEqual(["ABC-1"]);
  }, 30_000);

  // A session that starts while another process holds the lock, as `rerail scrub` or `rerail reset` does for seconds:
  // its row waits for the lock. Before, its owner claim made the row on this thread first, waiting up to 5 s, then
  // failed with the lock's error, so the session opened without its start note.
  it("keep a session's owner claim behind its row while the lock is held, without waiting, and make both once it is given back", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    sessions.openSession(session, 1_000);

    let claim: PolicyOwnerClaim | undefined;
    expect(msOf(() => { claim = controls.registerOwner(session.sessionId, "background"); })).toBeLessThan(1_000);

    holder.exec("COMMIT");
    await settled(path);
    expect(controls.holdsOwner(session.sessionId, claim!)).toBe(true);
    expect(rows(path, "SELECT session_id, ended_at FROM sessions")).toEqual([[session.sessionId, null]]);
  }, 30_000);

  // A session that ends while its claim still waits: made first, the release found no row of the claim, and the claim,
  // made after it, left the session held by a runtime that had let it go.
  it("make a session's release behind its claim, when both wait for the lock", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    sessions.openSession(session, 1_000);

    const claim = controls.registerOwner(session.sessionId, "background");
    expect(msOf(() => controls.releaseOwner(session.sessionId, claim))).toBeLessThan(1_000);

    holder.exec("COMMIT");
    await settled(path);
    expect(rows(path, "SELECT session_id, active FROM policy_session_owners")).toEqual([[session.sessionId, 0]]);
  }, 30_000);

  // A session's open reads its last recorded event to find what a power cut took. Before, that read made the session's
  // waiting row on this thread first, waiting up to 5 s for the lock while every other session's hooks waited too.
  it("leave a session's open reading its last recorded event at once while its row waits for the lock", async () => {
    const { path, sessions } = await stores();
    const holder = holderHere(path);
    sessions.openSession(session, 1_000);

    let recorded: string | undefined = "unread";
    expect(msOf(() => { recorded = sessions.lastRecordedEvent(session.sessionId, new Set(["call"])); })).toBeLessThan(1_000);

    expect(recorded).toBeUndefined();
    holder.exec("COMMIT");
  }, 30_000);

  // A session's evaluator reads at its open the places its record holds that its walks could not check, to tell each
  // once. Before, that read made the session's waiting writes on this thread first, up to 5 s while every hook waited.
  it("leave a session's evaluator reading the places its record holds at once while one waits, and keep one record of a place told again", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const gap = { boundary: "turn" as const, path: "src/locked", reason: "EACCES: permission denied, opendir 'src/locked'", key: "src/locked:EACCES" };
    const holder = holderHere(path);
    sessions.recordDiscoveryGaps(session, [gap]);

    let held: readonly unknown[] = ["unread"];
    expect(msOf(() => { held = sessions.discoveryGaps(session.sessionId, 10); })).toBeLessThan(1_000);
    expect(held).toEqual([]);
    // Read as not held, the place is told again by the evaluator that read it, which holds it from then on.
    sessions.recordDiscoveryGaps(session, [gap]);

    holder.exec("COMMIT");
    await settled(path);
    expect(sessions.discoveryGaps(session.sessionId, 10)).toMatchObject([{ path: "src/locked", reason: gap.reason }]);
    expect(rows(path, "SELECT count(*) FROM events WHERE kind = 'source-discovery-gap'")).toEqual([[1]]);
  }, 30_000);

  // A claim kept for the lock is made later, where a refusal reaches no caller: the open is refused at once instead, as
  // the database holds the session's owner now.
  it("refuse at once a claim on a session a live runtime holds, while the lock is held", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    sessions.openSession(session, 1_000);
    controls.registerOwner(session.sessionId, "background");
    holderHere(path);

    expect(thrown(() => controls.registerOwner(session.sessionId, "background"))).toMatchObject({
      message: expect.stringContaining("already owned by a live runtime"),
    });
  }, 30_000);

  // Another runtime claimed the session after this one's claim was kept and before it was made: made then, the claim
  // leaves the live runtime's claim as it is and says so, as the refusal at an open does.
  it("leave a claim on a session another live runtime made while this one's waited, and say so", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    sessions.openSession(session, 1_000);
    const holder = holderHere(path);
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);

    controls.registerOwner(session.sessionId, "background");
    holder.prepare("INSERT INTO policy_session_owners (session_id, delivery, active, process_id, token, updated_at) VALUES (?, 'next-event', 1, ?, 'other', 2000)")
      .run(session.sessionId, process.ppid);
    holder.exec("COMMIT");
    await settled(path);

    expect(rows(path, "SELECT process_id, token, active FROM policy_session_owners")).toEqual([[process.ppid, "other", 1]]);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("already owned by a live runtime"));
  }, 30_000);

  // The lock wait learns that the lock is free by taking it and giving it back on a connection of its thread's own.
  // Before, the owner claim's one try for the session's row, its waiting time spent, met that take as a holder and failed
  // with the lock's error though no other connection held the lock: as the lock was given back, a session's open failed
  // so 1 run in 160 (`engine-service-runtimes.test.ts`). The take held here is one the system stops in its two statements.
  it("are made for a session's owner claim past the bound while the lock wait takes the lock to learn it is free", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    const lockWaitTry = holdNextLockWaitTry();
    const clock = boundClock();
    sessions.openSession(session, 1_000);
    clock.pastTheBound();
    holder.exec("COMMIT");
    await lockWaitTry.held;

    const claimed = thrown(() => controls.registerOwner(session.sessionId, "background"));
    lockWaitTry.release();
    await settled(path);

    expect(claimed).toBeUndefined();
    expect(rows(path, "SELECT session_id FROM policy_session_owners")).toEqual([[session.sessionId]]);
  }, 30_000);

  // A quiet session's open for a scan that met the lock waits for it the same way (`engine-service.ts`). Before, another
  // session's owner claim, a transaction that reads the session's row before it writes, met that wait's take as a holder,
  // and SQLite failed its write at once rather than wait for it.
  it("leave a session's owner claim no lock wait's take to meet", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    sessions.openSession(session, 1_000);
    const holder = holderHere(path);
    const lockWaitTry = holdNextLockWaitTry();
    const stop = new AbortController();
    closers.push(() => stop.abort());
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    const freed = waitForWriteLock(path, stop.signal);
    holder.exec("COMMIT");
    await lockWaitTry.held;

    const claimed = thrown(() => controls.registerOwner(session.sessionId, "background"));
    lockWaitTry.release();

    expect(claimed).toBeUndefined();
    expect(await freed).toBe(true);
    expect(rows(path, "SELECT session_id FROM policy_session_owners")).toEqual([[session.sessionId]]);
  }, 30_000);

  // The daemon asks whether the lock is held before it opens a quiet session again for a scan (`failWhileWriteLocked`).
  // Before, that probe met a lock wait's take as a holder and said the lock was held though no other connection held it,
  // so the scan's open waited for the lock once more.
  it("leave the probe before a quiet session's open for a scan no lock wait's take to meet", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const holder = holderHere(path);
    const lockWaitTry = holdNextLockWaitTry();
    const stop = new AbortController();
    closers.push(() => stop.abort());
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    const freed = waitForWriteLock(path, stop.signal);
    holder.exec("COMMIT");
    await lockWaitTry.held;

    const probed = thrown(() => failWhileWriteLocked(path));
    lockWaitTry.release();

    expect(probed).toBeUndefined();
    expect(await freed).toBe(true);
  }, 30_000);

  // Each of the audit controls' transactions below reads before it writes, as the owner claim's does. Before, each met
  // a lock wait's take as a holder, and SQLite failed its write at once with the lock's error.
  it.each([
    {
      write: "a session's settings change",
      act: (controls: SqliteAuditControlStore): unknown => controls.updateSettings(session.sessionId, { mode: "log" }).settings.mode,
      made: "log",
    },
    {
      write: "a checkout's settings change",
      act: (controls: SqliteAuditControlStore): unknown => controls.updateWorkspaceSettings("/work", { mode: "log" }).mode,
      made: "log",
    },
    {
      write: "a repository audit's claim",
      act: (controls: SqliteAuditControlStore): unknown => controls.claimRepositoryAudit(session.sessionId, "/work"),
      made: true,
    },
    {
      write: "the end of a session whose owner ended",
      arrange: async (controls: SqliteAuditControlStore, path: string): Promise<void> => {
        controls.registerOwner(session.sessionId, "background");
        const ended = fork(join(dirname(path), "ended.cjs"), [], { stdio: "ignore" });
        await once(ended, "exit");
        const database = new Database(path);
        database.prepare("UPDATE policy_session_owners SET process_id = ?").run(ended.pid);
        database.close();
      },
      act: (controls: SqliteAuditControlStore): unknown => controls.endSessionsOfEndedOwners(1_000, 5_000),
      made: 1,
    },
    {
      write: "an audit request's claim",
      arrange: async (controls: SqliteAuditControlStore): Promise<PolicyOwnerClaim> => {
        const claim = controls.registerOwner(session.sessionId, "background");
        await controls.requestAudit(session.sessionId, { scope: "repository" });
        return claim;
      },
      act: (controls: SqliteAuditControlStore, claim?: unknown): unknown => controls.claim(session.sessionId, claim as PolicyOwnerClaim)?.status,
      made: "running",
    },
  ])("leave $write no lock wait's take to meet", async ({ arrange, act, made }) => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    sessions.openSession(session, 1_000);
    await writeFile(join(dirname(path), "ended.cjs"), "");
    const arranged = await arrange?.(controls, path);
    const holder = holderHere(path);
    const lockWaitTry = holdNextLockWaitTry();
    const stop = new AbortController();
    closers.push(() => stop.abort());
    const { waitForWriteLock } = await import("../../../src/engine/storage/write-lock-wait.js");
    const freed = waitForWriteLock(path, stop.signal);
    holder.exec("COMMIT");
    await lockWaitTry.held;

    let result: unknown;
    const failed = thrown(() => { result = act(controls, arranged); });
    lockWaitTry.release();

    expect(failed).toBeUndefined();
    expect(result).toEqual(made);
    expect(await freed).toBe(true);
  }, 30_000);

  it("are reported lost at the last store's close without waiting, once asked longer ago than the bound", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = holderHere(path);
    const clock = boundClock();
    sessions.append({ sessionId: session.sessionId, eventId: "late-event", kind: "call", payload: {} });
    clock.pastTheBound();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);

    expect(msOf(() => { audits.close(); sessions.close(); })).toBeLessThan(1_000);

    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("1 records not written"));
    holder.exec("COMMIT");
  }, 30_000);

  // The audit controls take a use of the record writer at their first claim or release and give it back at their close,
  // so the last store's close still makes what waits, or reports it, and closes the writer's connection. The claim lost
  // there is told to its runtime, which would hold it otherwise.
  it("are reported lost at the last store's close, a session's owner claim among them, once asked longer ago than the bound", async () => {
    const { path, sessions, audits } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    const clock = boundClock();
    sessions.openSession(session, 1_000);
    const lost = vi.fn();
    const claim = controls.registerOwner(session.sessionId, "background", undefined, lost);
    clock.pastTheBound();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);

    expect(msOf(() => { controls.close(); audits.close(); sessions.close(); })).toBeLessThan(1_000);

    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("2 records not written"));
    await vi.waitFor(() => expect(lost).toHaveBeenCalledWith(claim));
    expect(stderr).toHaveBeenCalledWith("rerail: the claim on session held-session was not made, so this process does not own it\n");
    holder.exec("COMMIT");
  }, 30_000);

  // The move of the event numbers past a remote's ran on the store's own connection, on the thread answering hooks: it
  // slept the 5 s bound in SQLite's busy handler, then the session's open failed with the lock's error.
  it("move the event numbers past a remote's at a session's first open without waiting, ahead of its row and events", async () => {
    const { path, sessions } = await stores();
    remoteHolds(path, 40);
    const holder = await lockHolder(path);
    await holder.hold();

    let error: unknown;
    const ms = msOf(() => {
      error = thrown(() => {
        sessions.openSession(session, 1_000);
        sessions.append(callEvent("first-call", 1_000).event, session);
      });
    });
    expect(error).toBeUndefined();
    expect(ms).toBeLessThan(1_000);

    await holder.release();
    await settled(path);
    expect(rows(path, "SELECT session_id FROM sessions")).toEqual([[session.sessionId]]);
    expect(rows(path, "SELECT sequence, event_id FROM events")).toEqual([[41, "first-call"]]);
  }, 30_000);

  // A session reopened while its earlier rows still waited read an older event as the last recorded, and its open
  // counted the rows after it as lost. The read answers what is on disk without waiting for them; the record of lost
  // events, made behind them, counts again.
  it("count no event lost at a session's reopen while its earlier rows wait, when the holder gave the lock back", async () => {
    const [one, two, three] = [callEvent("one", 1), callEvent("two", 2), callEvent("three", 3)];
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    sessions.append(one.event, session);
    const holder = holderHere(path);
    sessions.append(two.event, session);
    sessions.append(three.event, session);
    holder.exec("COMMIT");

    reopen(sessions, [one, two, three]);
    await settled(path);

    expect(lostRecords(path)).toEqual([]);
    expect(sessions.lastRecordedEvent(session.sessionId, new Set(["call"]))).toBe("three");
  }, 30_000);

  // A newer Rerail's migration or `rerail scrub` holds the lock past the read's bound while the session reopens in the
  // same daemon: the read sees the disk without the waiting rows. The record of lost events counted them, and the
  // history then held both the events and a record saying they were lost.
  it("count no event lost at a session's reopen whose row still waits for the lock past the bound", async () => {
    const [one, two, three] = [callEvent("one", 1), callEvent("two", 2), callEvent("three", 3)];
    const { path } = await reopenedPastTheBound([one, two, three], [two, three]);

    expect(rows(path, "SELECT event_id FROM events WHERE kind = 'call' ORDER BY sequence")).toEqual([["one"], ["two"], ["three"]]);
    expect(lostRecords(path)).toEqual([]);
  }, 30_000);

  // The record took the time and id of the last event whose row was missing, and committed after the rows that waited:
  // the newest row read an older time than the newest event's, and the next open walked those rows again.
  it("still count the events whose rows the history lacks once the waiting rows are made, at the stretch's end", async () => {
    const [one, two, gone, three] = [callEvent("one", 1), callEvent("two", 2), callEvent("gone", 3), callEvent("three", 4)];
    const { path, sessions } = await reopenedPastTheBound([one, two, gone, three], [two, three]);

    expect(lastObservedEventAt(path)).toBe(4);
    expect(lostRecords(path)).toEqual([[
      "events-lost:three", 4, { count: 1, kinds: { call: 1 }, firstEventId: "gone", lastEventId: "gone", firstObservedAt: 3, lastObservedAt: 3 },
    ]]);
    expect(sessions.lastRecordedEvent(session.sessionId, new Set(["call"]))).toBe("three");
  }, 30_000);

  // `rerail status` reads the newest row's time (`lastObservedEventAt`) and says nothing was recorded after a hook
  // failure later than it (`hookFailureLines`).
  it("leave `rerail status` counting no hook failure as one after which nothing was recorded, once the waiting rows are made", async () => {
    const [one, two, gone, three] = [callEvent("one", 1_000), callEvent("two", 2_000), callEvent("gone", 3_000), callEvent("three", 4_000)];
    const { path } = await reopenedPastTheBound([one, two, gone, three], [two, three]);
    const failure = { at: 3_500, host: "claude-code", event: "PostToolUse", agent: null, rerail: "0.1.3", reason: "the daemon did not answer" };

    expect(hookFailureLines({ failures: [failure], keptSince: null }, lastObservedEventAt(path))).toEqual([
      "Hooks: 1 hook event could not be recorded in the last 24 hours; the last, PostToolUse from claude-code with Rerail 0.1.3 at 1970-01-01T00:00:03.500Z: the daemon did not answer.",
    ]);
  }, 30_000);

  // The record named the last missing event, so each later open walked the rows after it, which were there, and its
  // write took the lock and told every open dashboard of a commit that changed nothing.
  it("take no write lock and tell no commit at a later reopen, whose walk starts after the counted stretch", async () => {
    const [one, two, gone, three] = [callEvent("one", 1), callEvent("two", 2), callEvent("gone", 3), callEvent("three", 4)];
    const { path, sessions } = await reopenedPastTheBound([one, two, gone, three], [two, three]);
    sessions.openSession(session, 9_000);
    const recorded = sessions.lastRecordedEvent(session.sessionId, new Set(["call"]));
    const seen = await writesSeen(path);

    sessions.recordLostEvents(session, linesAfter([one, two, gone, three].map(({ line }) => line), recorded));

    expect(seen()).toEqual({ notices: 0, lockTakes: 0 });
    expect(recorded).toBe("three");
  }, 30_000);

  it("take no write lock and tell no commit for lost events whose rows are all there", async () => {
    const { path, sessions } = await stores();
    const [one, two] = [callEvent("one", 1), callEvent("two", 2)];
    sessions.openSession(session, 1_000);
    for (const { event } of [one, two]) sessions.append(event, session);
    const seen = await writesSeen(path);

    sessions.recordLostEvents(session, [one.line, two.line]);

    expect(seen()).toEqual({ notices: 0, lockTakes: 0 });
    expect(lostRecords(path)).toEqual([]);
  }, 30_000);

  // Each open that still read the disk without the waiting rows walked from the same older event: a record whose id
  // names the stretch's end would count the missing event again at the second, whose stretch ends later.
  it("count an event whose row never came once, however often the session reopens while rows wait past the bound", async () => {
    const { path, sessions } = await stores();
    const [one, two, gone, three, four] = [callEvent("one", 1), callEvent("two", 2), callEvent("gone", 3), callEvent("three", 4), callEvent("four", 5)];
    sessions.openSession(session, 1_000);
    sessions.append(one.event, session);
    const holder = holderHere(path);
    const clock = boundClock();
    for (const { event } of [two, three]) sessions.append(event, session);
    clock.pastTheBound();
    reopen(sessions, [one, two, gone, three]);
    sessions.append(four.event, session);
    reopen(sessions, [one, two, gone, three, four]);
    holder.exec("COMMIT");
    await settled(path);
    reopen(sessions, [one, two, gone, three, four]);

    expect(lostRecords(path)).toEqual([[
      "events-lost:three", 4, { count: 1, kinds: { call: 1 }, firstEventId: "gone", lastEventId: "gone", firstObservedAt: 3, lastObservedAt: 3 },
    ]]);
    expect(lastObservedEventAt(path)).toBe(5);
  }, 30_000);

  it("throw at once a failure that is not the lock, as a write made at once did before", async () => {
    const { sessions } = await stores();
    expect(() => sessions.append({ sessionId: "no-such-session", eventId: "orphan", kind: "call", payload: {} })).toThrow(/FOREIGN KEY/u);
  });
});

describe("a write that is made now or not at all", () => {
  /** A write of one event row into the session `stores()` opened. */
  const eventWrite = (eventId: string) => (database: RerailDatabase): void => {
    database.$client.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json, payload_v8) VALUES (?, ?, 'call', 1, '{}', x'')")
      .run(session.sessionId, eventId);
  };
  const eventIds = (path: string) => rows(path, "SELECT event_id FROM events WHERE kind = 'call' ORDER BY sequence");

  it("says the lock is held, keeping nothing, where a write would wait for it", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    const holder = holderHere(path);

    expect(writer.writeNow(eventWrite("held"))).toBe(false);
    holder.exec("COMMIT");
    await writer.settled();
    expect(writer.writeNow(eventWrite("free"))).toBe(true);

    expect(eventIds(path)).toEqual([["free"]]);
  });

  it("is made once the holder gives the lock back, trying between the thread's turns, never holding it", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    const holder = holderHere(path);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    closers.push(() => { vi.useRealTimers(); });

    const made = writer.writeWhenFree(eventWrite("late"), new AbortController().signal);
    // SQLite's first pauses (1, 2, 5 ms): each try meets the lock, and this thread runs between them.
    await vi.advanceTimersByTimeAsync(8);
    expect(eventIds(path)).toEqual([]);
    holder.exec("COMMIT");
    await vi.advanceTimersByTimeAsync(10);

    await expect(made).resolves.toBe(true);
    expect(eventIds(path)).toEqual([["late"]]);
  });

  it("tries at SQLite's times from the first, however late this thread ends each pause", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    const holder = holderHere(path);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    closers.push(() => { vi.useRealTimers(); });
    // Every pause ends 1 ms late, as Node's timers do by about that: a schedule that added the pauses up would make its
    // 19th try at 1,047 ms, where SQLite's busy handler tries at 1,028 ms.
    const onTime = globalThis.setTimeout;
    vi.stubGlobal("setTimeout", (callback: () => void, ms: number) => onTime(callback, ms + 1));
    closers.push(() => { vi.unstubAllGlobals(); });

    const made = writer.writeWhenFree(eventWrite("on-time"), new AbortController().signal);
    await vi.advanceTimersByTimeAsync(1_028);
    expect(eventIds(path)).toEqual([]);
    holder.exec("COMMIT");
    await vi.advanceTimersByTimeAsync(2);

    expect(eventIds(path)).toEqual([["on-time"]]);
    await expect(made).resolves.toBe(true);
  });

  it("is not made, and keeps nothing, once the tries reach the busy timeout or its caller stops them", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    const holder = holderHere(path);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    closers.push(() => { vi.useRealTimers(); });

    const timedOut = writer.writeWhenFree(eventWrite("timed-out"), new AbortController().signal);
    await vi.advanceTimersByTimeAsync(BUSY_TIMEOUT_MS - 1);
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    await expect(timedOut).resolves.toBe(false);
    const stop = new AbortController();
    const stopped = writer.writeWhenFree(eventWrite("stopped"), stop.signal);
    stop.abort();
    await expect(stopped).resolves.toBe(false);

    expect(vi.getTimerCount()).toBe(0);
    holder.exec("COMMIT");
    await writer.settled();
    expect(eventIds(path)).toEqual([]);
  });
});
