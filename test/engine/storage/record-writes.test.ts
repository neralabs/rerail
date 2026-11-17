/**
 * Record writes while another process holds the Rerail database's write lock, as `rerail reset`, `rerail scrub` or a
 * newer Rerail's migration does for seconds: the stores' writes return at once on the calling thread, and are made in
 * the order they were asked for once the lock is given back. Before, each slept in SQLite's busy handler for 5 s on the
 * daemon's thread that answers hooks, then failed and the record was lost.
 */
import { fork } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { SqliteSessionStore } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const session = { sessionId: "held-session", host: "claude-code" as const, cwd: "/work" };
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

async function stores(): Promise<{ path: string; sessions: SqliteSessionStore; audits: SqliteAuditStore }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-record-writes-"));
  roots.push(root);
  const path = join(root, "rerail.sqlite");
  const sessions = new SqliteSessionStore(path);
  const audits = new SqliteAuditStore(path);
  closers.push(() => audits.close(), () => sessions.close());
  return { path, sessions, audits };
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

    sessions.openSession(session, 1_000);
    sessions.append({ sessionId: session.sessionId, eventId: "edit-call", kind: "call", payload: { tool: "Write" } }, session);
    const auditId = audits.start({
      sessionId: session.sessionId, workspaceRoot: "/work", scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "r1", ruleRevisions: [], paths: ["a.py"],
    });
    audits.finish(auditId, { status: "completed", violations: [], failures: [] });
    // Nothing is on disk yet, and the stores' own reads do not see it either.
    expect(rows(path, "SELECT session_id FROM sessions")).toEqual([]);
    expect(sessions.session(session.sessionId)).toBeUndefined();

    await holder.release();
    await sessions.settled();

    // The audit names the session row, which a foreign key demands first: the order held across the two stores.
    expect(rows(path, "SELECT session_id, started_at FROM sessions")).toEqual([[session.sessionId, 1_000]]);
    expect(rows(path, "SELECT event_id FROM events WHERE session_id = 'held-session'")).toEqual([["edit-call"]]);
    expect(rows(path, `SELECT status FROM audits WHERE audit_id = '${auditId}'`)).toEqual([["completed"]]);
  }, 30_000);

  it("keep each write's own time, not the time the lock was given back", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = await lockHolder(path);
    await holder.hold();
    const now = vi.spyOn(Date, "now").mockReturnValue(5_000);
    const auditId = audits.start({
      sessionId: session.sessionId, workspaceRoot: "/work", scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "r1", ruleRevisions: [], paths: ["a.py"],
    });
    sessions.append({ sessionId: session.sessionId, eventId: "edit-result", kind: "result", payload: {} });
    now.mockReturnValue(9_000);

    await holder.release();
    await audits.settled();

    expect(rows(path, `SELECT started_at FROM audits WHERE audit_id = '${auditId}'`)).toEqual([[5_000]]);
    expect(rows(path, "SELECT observed_at FROM events WHERE event_id = 'edit-result'")).toEqual([[5_000]]);
  }, 30_000);

  it("interrupt an audit whose start still waits for the lock", async () => {
    const { path, sessions, audits } = await stores();
    sessions.openSession(session, 1_000);
    const holder = await lockHolder(path);
    await holder.hold();
    const auditId = audits.start({
      sessionId: session.sessionId, workspaceRoot: "/work", scope: "touched-files", mode: "steer", revision: 1, catalogRevision: "r1", ruleRevisions: [], paths: ["a.py"],
    });
    audits.interrupt(session.sessionId);

    await holder.release();
    await audits.settled();

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
    // Imported here so the file's other cases run on a tree without the module and fail by what they find.
    const { openRecordWriter } = await import("../../../src/engine/storage/record-writes.js");
    const writer = openRecordWriter(path);
    closers.push(() => writer.close());
    writer.write(database => database.$client.prepare("UPDATE sessions SET ended_at = 2000").run(), 64 * 1024 * 1024 + 1);
    sessions.closeSession(session.sessionId, 3_000);

    await holder.release();
    await writer.settled();

    expect(rows(path, "SELECT ended_at FROM sessions")).toEqual([[3_000]]);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("1 records not written"));
  }, 30_000);

  it("throw at once a failure that is not the lock, as a write made at once did before", async () => {
    const { sessions } = await stores();
    expect(() => sessions.append({ sessionId: "no-such-session", eventId: "orphan", kind: "call", payload: {} })).toThrow(/FOREIGN KEY/u);
  });
});
