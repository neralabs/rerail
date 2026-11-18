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
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import type { AuditInput } from "../../../src/engine/audit/audit.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { BUSY_TIMEOUT_MS, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { SqliteSessionStore } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const session = { sessionId: "held-session", host: "claude-code" as const, cwd: "/work" };
const auditInput: AuditInput = {
  sessionId: session.sessionId, workspaceRoot: "/work", scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "r1", ruleRevisions: [], paths: ["a.py"],
};
const roots: string[] = [];
const closers: Array<() => Promise<void> | void> = [];

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
  // left out the link it had just been asked to add.
  it("are made before a session's owner claim reads the session's row, when the holder gave the lock back", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    sessions.openSession(session, 1_000);
    holder.exec("COMMIT");

    const claim = controls.registerOwner(session.sessionId, "background");

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

  // Main's open of a session failed with the lock's own error when its row's write waited out the bound, which the
  // engine service answers by trying the open again once the lock is given back.
  it("fail the session's owner claim as a held lock does while its row still waits past the bound, claiming nothing", async () => {
    const { path, sessions } = await stores();
    const controls = new SqliteAuditControlStore(path);
    closers.push(() => controls.close());
    const holder = holderHere(path);
    const clock = boundClock();
    sessions.openSession(session, 1_000);
    clock.pastTheBound();

    expect(thrown(() => controls.registerOwner(session.sessionId, "background"))).toMatchObject({ code: "SQLITE_BUSY" });

    holder.exec("COMMIT");
    await settled(path);
    expect(rows(path, "SELECT session_id FROM sessions")).toEqual([[session.sessionId]]);
    expect(rows(path, "SELECT session_id FROM policy_session_owners")).toEqual([]);
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
  const eventIds = (path: string) => rows(path, "SELECT event_id AS eventId FROM events WHERE kind = 'call' ORDER BY sequence");

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

    expect(eventIds(path)).toEqual([{ eventId: "free" }]);
  });

  it("is made once the holder gives the lock back, trying between the thread's turns, never holding it", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    const holder = holderHere(path);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    closers.push(() => { vi.useRealTimers(); });

    const made = writer.writeWhenFree(eventWrite("late"), new AbortController().signal);
    // SQLite's first pauses (1, 2, 5 ms): each try meets the lock, and this thread runs between them.
    await vi.advanceTimersByTimeAsync(8);
    expect(eventIds(path)).toEqual([]);
    holder.exec("COMMIT");
    await vi.advanceTimersByTimeAsync(10);

    await expect(made).resolves.toBe(true);
    expect(eventIds(path)).toEqual([{ eventId: "late" }]);
  });

  it("is not made, and keeps nothing, once the tries reach the busy timeout or its caller stops them", async () => {
    const { path, sessions } = await stores();
    sessions.openSession(session, 1_000);
    const writer = (await recordWrites()).openRecordWriter(path);
    closers.push(() => writer.close());
    const holder = holderHere(path);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
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
