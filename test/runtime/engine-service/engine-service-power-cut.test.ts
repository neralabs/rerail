import { spawn, type ChildProcess } from "node:child_process";
import { appendFile, copyFile, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../../src/api/index.js";
import { resetRepositoryHistory, type AdapterEvent, type CodeRuleAuditor } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { addRemote } from "../../../src/runtime/remote-config.js";
import { RemoteOutbox } from "../../../src/runtime/remote-outbox.js";
import { sessionDirectory } from "../../../src/runtime/session-paths.js";
import { lastObservedEventAt, SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/session-store.js";
import { withExperiments } from "../../support/environment/experiments.js";
import { startFakeSink, storedRecords, type FakeSink } from "../../support/fake-sink.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { cutPower, latestCommitWhere, stateAt, walCommits } from "../../support/engine/power-cut.js";
import { startTestRepository } from "../../support/started-repository.js";

// The store's writers commit with `synchronous = NORMAL`: a power cut or an operating system crash can take the last
// commits before it, never a commit the daemon's own end follows, and keeps the earliest ones whole. What it leaves
// (`power-cut.ts`) must read right once the next daemon has started: nothing shows as running, unsent or missing for
// good because the commits after the cut never reached the disk.

const CHILD = fileURLToPath(new URL("../../support/processes/engine-service-process.ts", import.meta.url));
const IDLE_MS = 60_000;
/** Ledger lines past the session's own in the case of an open after a reset. */
const LONG_LEDGER = 2_000;

const roots: string[] = [];
const services: EngineService[] = [];
const children: ChildProcess[] = [];
const sinks: FakeSink[] = [];

function ended(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  child.kill("SIGKILL");
  return exited;
}

afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(children.splice(0).map(ended));
  await Promise.allSettled(services.splice(0).map(service => service.close()));
  await Promise.all(sinks.splice(0).map(sink => sink.close()));
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

const quietAuditor = (): CodeRuleAuditor => ({
  openWorkspace: async () => undefined,
  acceptsPath: () => true,
  auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
  close: async () => undefined,
});

let sockets = 0;
function socketIn(root: string): string {
  sockets += 1;
  return process.platform === "win32" ? `\\\\.\\pipe\\rerail-power-cut-${process.pid}-${Date.now()}-${sockets}` : join(root, `s${sockets}.sock`);
}

/** A started checkout, the database its sessions are recorded in and the folder of their ledgers. */
async function records(): Promise<{ root: string; databasePath: string; checkout: string; storageRoot: string }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-power-cut-")));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const checkout = join(root, "repo");
  await mkdir(checkout);
  startTestRepository(checkout, databasePath);
  return { root, databasePath, checkout, storageRoot: join(root, "state") };
}

/** The next daemon's engine service, in this process, over the records the power cut left. */
async function nextDaemon(root: string, databasePath: string, now: () => number = Date.now): Promise<EngineService> {
  const started = await startEngineService({
    socketPath: socketIn(root), storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: IDLE_MS,
    idleRuntimeMs: 0, createAuditor: quietAuditor, now,
  });
  services.push(started);
  return started;
}

function answer(child: ChildProcess, wanted: (message: Record<string, unknown>) => boolean): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("the daemon process did not answer within 60 s")), 60_000);
    const listen = (message: Record<string, unknown>) => {
      if (!wanted(message)) return;
      clearTimeout(timer);
      child.off("message", listen);
      resolve(message);
    };
    child.on("message", listen);
  });
}

/**
 * A daemon's engine service in a process of its own (`engine-service-process.ts`), which records a check for each
 * batch of edits; `cut` ends it as a power cut does, none of its own clean-up run.
 */
async function daemonProcess(root: string, databasePath: string) {
  const child = spawn(process.execPath, ["--import", "tsx", CHILD, socketIn(root), join(root, "state"), databasePath, String(IDLE_MS), "audits"], {
    cwd: process.cwd(), stdio: ["ignore", "ignore", "pipe", "ipc"],
  });
  children.push(child);
  let stderr = "";
  child.stderr!.on("data", chunk => { stderr += String(chunk); });
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  const ready = await answer(child, message => message.ready === true).catch((error: unknown) => {
    throw new Error(`${String(error)}; its stderr: ${stderr.slice(0, 2_000)}`);
  });
  return {
    send: (event: AdapterEvent) => ingestEngineEvent({ socketPath: String(ready.socketPath) }, event),
    stop: async () => {
      child.kill("SIGKILL");
      await exited;
    },
  };
}

function events(cwd: string) {
  const session = (sessionId: string) => ({ host: "claude-code" as const, sessionId, cwd });
  return {
    start: (sessionId: string): AdapterEvent => ({ kind: "session-start", eventId: `${sessionId}:start`, session: session(sessionId) }),
    prompt: (sessionId: string, text: string): AdapterEvent => ({ kind: "user-prompt", eventId: `${sessionId}:${text}`, sessionId, session: session(sessionId), text }),
    /** The agent of `sessionId` writes `text` to `path` with its Write tool, as tool call `toolCallId`. */
    writes: async (send: (event: AdapterEvent) => Promise<unknown>, sessionId: string, toolCallId: string, path: string, text: string) => {
      const tool = { sessionId, session: session(sessionId), toolCallId, toolName: "Write" } as const;
      await send({ kind: "call", eventId: `${sessionId}:${toolCallId}:call`, ...tool, input: { file_path: path, content: text } });
      await writeFile(path, text);
      await send({ kind: "result", eventId: `${sessionId}:${toolCallId}:result`, ...tool, output: "ok" });
    },
  };
}

/** What the dashboard reads of `path` from the daemon's API (`GET <path>`), as a person's request. */
async function dashboard(databasePath: string, path: string): Promise<Record<string, unknown>> {
  const stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }), origin: () => "http://127.0.0.1:4737",
  });
  try {
    const response = await createRerailApi(stores, operations, { version: "0.0.0-test" }).request(path);
    expect(response.status).toBe(200);
    return await response.json() as Record<string, unknown>;
  } finally {
    stores.close();
  }
}

/** Sets the checks of `sessionId` back to running, through a connection of its own. */
function stillRunning(databasePath: string, sessionId: string): void {
  const database = new Database(databasePath);
  try {
    database.prepare("UPDATE audits SET status = 'running', finished_at = NULL WHERE session_id = ?").run(sessionId);
  } finally {
    database.close();
  }
}

/** Every event row the store holds, in order, read through a connection of its own. */
function storedEvents(databasePath: string): Array<{ sequence: number; eventId: string }> {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare("SELECT sequence, event_id AS eventId FROM events ORDER BY sequence").all() as Array<{ sequence: number; eventId: string }>;
  } finally {
    database.close();
  }
}

/** The session's history as the dashboard pages it (`GET /api/sessions/{id}/events`), each event's id, kind and payload. */
async function history(databasePath: string, sessionId: string): Promise<Array<{ eventId: string; kind: string; observedAt: number; payload: unknown }>> {
  const page = await dashboard(databasePath, `/api/sessions/${sessionId}/events`) as { events: Array<{ eventId: string; kind: string; observedAt: number; payload: unknown }> };
  return page.events.map(({ eventId, kind, observedAt, payload }) => ({ eventId, kind, observedAt, payload }));
}

/** The record of lost events the session's history holds: its time and what it says. */
async function lostRecords(databasePath: string, sessionId: string): Promise<Array<{ observedAt: number; payload: unknown }>> {
  return (await history(databasePath, sessionId)).filter(event => event.kind === "events-lost").map(({ observedAt, payload }) => ({ observedAt, payload }));
}

/** How many of the last lines of the ledger of `sessionId` come from the line of `eventId` on. */
async function ledgerLinesFrom(storageRoot: string, sessionId: string, eventId: string): Promise<{ path: string; lines: number }> {
  const path = join(sessionDirectory(storageRoot, sessionId), "events.jsonl");
  const lines = (await readFile(path, "utf8")).split("\n").filter(Boolean);
  const at = lines.findIndex(line => (JSON.parse(line) as { type?: string; eventId?: string }).type === "event" && line.includes(`"eventId":"${eventId}"`));
  expect(at, `the ledger's line of ${eventId}`).toBeGreaterThanOrEqual(0);
  return { path, lines: lines.length - at };
}

/** The session's row as the store holds it, read through a connection of its own. */
function sessionRow(databasePath: string, sessionId: string): { startedAt: number; endedAt: number | null } | undefined {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare("SELECT started_at AS startedAt, ended_at AS endedAt FROM sessions WHERE session_id = ?").get(sessionId) as
      { startedAt: number; endedAt: number | null } | undefined;
  } finally {
    database.close();
  }
}

/** When each of `eventIds` was recorded, in the state the log's last commit leaves. */
function recordedAt(databasePath: string, eventIds: readonly string[]): number[] {
  return stateAt(databasePath, walCommits(databasePath).at(-1)!, database =>
    eventIds.map(eventId => (database.prepare("SELECT observed_at AS at FROM events WHERE event_id = ?").get(eventId) as { at: number }).at));
}

/**
 * Runs `sync` with the outbox's log left as the disk holds it when the power goes: a connection of the test's own stays
 * open meanwhile, so the outbox's close neither copies its log into the file nor removes it, and the file and its log
 * are copied before that connection closes, then put back with no WAL index, as `cutPower` wants them.
 */
async function keepingOutboxLog(root: string, sync: () => Promise<void>): Promise<string> {
  const path = join(root, "remote-outbox.sqlite");
  new RemoteOutbox({ home: root }).status();
  const kept = await mkdtemp(join(tmpdir(), "rerail-outbox-log-"));
  roots.push(kept);
  const holder = new Database(path);
  // A connection counts as open on the log once it has read through it: an idle one would leave the close to the outbox.
  holder.prepare("SELECT count(*) FROM cursors").get();
  try {
    await sync();
    for (const suffix of ["", "-wal"]) await copyFile(`${path}${suffix}`, join(kept, `outbox${suffix}`));
  } finally {
    holder.close();
  }
  for (const suffix of ["", "-wal"]) await copyFile(join(kept, `outbox${suffix}`), `${path}${suffix}`);
  await rm(`${path}-shm`, { force: true });
  return path;
}

/** The highest event number the outbox's cursors say a remote acknowledged. */
const acknowledgedEvents = (state: Database.Database): number =>
  (state.prepare("SELECT max(value) AS value FROM cursors WHERE stream = 'event'").get() as { value: number | null }).value ?? 0;

/** Every id the remote received names one event, and it holds every event the store holds under the store's number. */
function expectOneEventPerId(sink: FakeSink, databasePath: string): void {
  const named = new Map<string, string>();
  for (const record of storedRecords(sink).filter(item => item.type === "event")) {
    expect(named.get(record.id) ?? String(record.eventId), record.id).toBe(String(record.eventId));
    named.set(record.id, String(record.eventId));
  }
  const machine = [...named.keys()][0]!.split(":")[0]!;
  const stored = storedEvents(databasePath);
  expect(stored.map(row => row.eventId)).toContain("s:six");
  expect(stored.map(row => [row.eventId, named.get(`${machine}:${row.sequence}`)])).toEqual(stored.map(row => [row.eventId, row.eventId]));
}

const hasEvent = (database: Database.Database, eventId: string): boolean =>
  database.prepare("SELECT 1 FROM events WHERE event_id = ?").get(eventId) !== undefined;

const runningChecks = (sessionId: string) => (database: Database.Database): boolean =>
  (database.prepare("SELECT count(*) AS n FROM audits WHERE session_id = ? AND status = 'running'").get(sessionId) as { n: number }).n > 0;

describe("the records a power cut leaves, once the next daemon has started", () => {
  // The check's start reached the disk, its finish did not: today it reads as running until the session's evaluator
  // opens again, and for a session that never opens again, for good.
  it("closes as interrupted a check whose finish the cut took, its daemon gone, and leaves its finished ones", { timeout: 120_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start, writes } = events(checkout);
    const daemon = await daemonProcess(root, databasePath);
    await daemon.send(start("s"));
    await writes(daemon.send, "s", "t1", join(checkout, "a.ts"), "export const a = 1;\n");
    await writes(daemon.send, "s", "t2", join(checkout, "a.ts"), "export const a = 2;\n");
    await daemon.stop();
    cutPower(databasePath, latestCommitWhere(databasePath, database => runningChecks("s")(database)
      && (database.prepare("SELECT count(*) AS n FROM audits WHERE session_id = 's' AND status = 'completed'").get() as { n: number }).n === 1));
    await nextDaemon(root, databasePath);
    const { audits } = await dashboard(databasePath, "/api/audits?sessionId=s") as { audits: Array<{ status: string; finishedAt: number | null }> };
    expect(audits.map(audit => audit.status).sort()).toEqual(["completed", "interrupted"]);
    expect(audits.every(audit => audit.finishedAt !== null)).toBe(true);
  });

  // A check of a session another daemon still runs is that daemon's to finish. Both kinds of check run when the next
  // daemon starts: one of a daemon a kill stopped mid-check, as a power cut stops it, and one of a daemon that runs.
  it("leaves running a check of a daemon that still runs, and closes the gone one's", { timeout: 120_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start, writes } = events(checkout);
    const alive = await daemonProcess(root, databasePath);
    const gone = await daemonProcess(root, databasePath);
    await gone.send(start("s"));
    await writes(gone.send, "s", "t1", join(checkout, "a.ts"), "export const a = 1;\n");
    stillRunning(databasePath, "s");
    await gone.stop();
    await alive.send(start("o"));
    await writes(alive.send, "o", "t1", join(checkout, "b.ts"), "export const b = 1;\n");
    stillRunning(databasePath, "o");
    await nextDaemon(root, databasePath);
    const statuses = async (sessionId: string) =>
      ((await dashboard(databasePath, `/api/audits?sessionId=${sessionId}`)) as { audits: Array<{ status: string }> }).audits.map(audit => audit.status);
    expect(await statuses("o")).toEqual(["running"]);
    expect(await statuses("s")).toEqual(["interrupted"]);
  });

  describe("sent to a remote", () => {
    withExperiments("REMOTE_SINK");

    // The remote took the events, then the cut took their rows: the store's counter went back with them, and the next
    // events took numbers the remote's cursor had passed, so the outbox never sent them. Three rows go: the reopen's
    // write of a row the session already has takes a number too (an AUTOINCREMENT insert that a conflict skips).
    it("sends the events that came after the cut, each under a number no event the remote holds has", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const sink = await startFakeSink();
      sinks.push(sink);
      addRemote({ url: sink.url, token: "test-token", content: true }, root);
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two", "three", "four", "five"]) await daemon.send(prompt("s", text));
      const outbox = new RemoteOutbox({ home: root });
      const sent = () => storedRecords(sink).filter(record => record.type === "event").map(record => ({ id: record.id, eventId: String(record.eventId) }));
      expect((await outbox.sync({ force: true }))[0]?.error).toBeUndefined();
      expect(sent().map(record => record.eventId)).toContain("s:five");
      await daemon.stop();
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:two") && !hasEvent(database, "s:three")));
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "six"));
      expect((await outbox.sync({ force: true }))[0]?.error).toBeUndefined();
      // The lost events the remote holds stay as they were.
      expectOneEventPerId(sink, databasePath);
    });

    // The remote answered 200 and the cut took the outbox's cursor move after it, with the events' rows: the cursor read
    // as if the remote held none of them, and the next events took numbers the remote holds.
    it("sends the events after the cut under new numbers when the cut also took the outbox's acknowledgement", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const sink = await startFakeSink();
      sinks.push(sink);
      addRemote({ url: sink.url, token: "test-token", content: true }, root);
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two", "three", "four", "five"]) await daemon.send(prompt("s", text));
      const outbox = new RemoteOutbox({ home: root });
      const outboxPath = await keepingOutboxLog(root, async () => {
        expect((await outbox.sync({ force: true }))[0]?.error).toBeUndefined();
      });
      await daemon.stop();
      const acknowledged = stateAt(outboxPath, walCommits(outboxPath).at(-1)!, acknowledgedEvents);
      expect(acknowledged).toBeGreaterThan(0);
      cutPower(outboxPath, latestCommitWhere(outboxPath, state => acknowledgedEvents(state) < acknowledged));
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:two") && !hasEvent(database, "s:three")));
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "six"));
      expect((await outbox.sync({ force: true }))[0]?.error).toBeUndefined();
      expectOneEventPerId(sink, databasePath);
    });
  });

  describe("a session's history", () => {
    // The rows went, the ledger kept their lines: the events count as processed, so nothing writes them again, and the
    // session's history missed them without a word.
    it("records, at the session's next open, which events the cut took and when, from its ledger", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two", "three"]) await daemon.send(prompt("s", text));
      await daemon.stop();
      const [two, three] = recordedAt(databasePath, ["s:two", "s:three"]);
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "s:two")));
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "four"));
      const kept = await history(databasePath, "s");
      expect(kept.map(event => event.eventId)).toEqual(expect.arrayContaining(["s:one", "s:four"]));
      expect(await lostRecords(databasePath, "s")).toEqual([{
        observedAt: three,
        payload: { count: 2, kinds: { "user-prompt": 2 }, firstEventId: "s:two", lastEventId: "s:three", firstObservedAt: two, lastObservedAt: three },
      }]);
      const { gaps } = await dashboard(databasePath, "/api/sessions/s") as { gaps: { lostEvents: number } };
      expect(gaps.lostEvents).toBe(2);
    });

    // A second cut took a later event's row and kept the record of the first stretch: the walk back went past that
    // stretch to the newest row, and counted its events again.
    it("counts each lost event once when a second cut takes a later event's row", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const first = await daemonProcess(root, databasePath);
      await first.send(start("s"));
      for (const text of ["one", "two", "three"]) await first.send(prompt("s", text));
      await first.stop();
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "s:two")));
      const second = await daemonProcess(root, databasePath);
      await second.send(prompt("s", "four"));
      await second.stop();
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "events-lost:s:three") && !hasEvent(database, "s:four")));
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "five"));
      expect((await lostRecords(databasePath, "s")).map(record => record.payload)).toEqual([
        expect.objectContaining({ count: 2, firstEventId: "s:two", lastEventId: "s:three" }),
        expect.objectContaining({ count: 1, firstEventId: "s:four", lastEventId: "s:four" }),
      ]);
      const { gaps } = await dashboard(databasePath, "/api/sessions/s") as { gaps: { lostEvents: number } };
      expect(gaps.lostEvents).toBe(3);
    });

    // A reset deleted the session's rows and its ledger stayed: the open asked the store for each ledger line's row in
    // turn, down to the first, at every open until an event's row was written again.
    it("reads the store a few times at an open after a reset, however long the ledger", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout, storageRoot } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      await daemon.send(prompt("s", "one"));
      await daemon.stop();
      const { path } = await ledgerLinesFrom(storageRoot, "s", "s:one");
      const lines = Array.from({ length: LONG_LEDGER }, (_, index) => JSON.stringify({ type: "event", eventId: `s:p${index}`, kind: "user-prompt", observedAt: 1_000 + index }));
      await appendFile(path, lines.map(line => `${line}\n`).join(""));
      const database = new Database(databasePath, { readonly: true });
      const { repositoryId } = database.prepare("SELECT repository_id AS repositoryId FROM sessions WHERE session_id = 's'").get() as { repositoryId: string };
      database.close();
      expect(resetRepositoryHistory(databasePath, repositoryId)).toMatchObject({ openSessions: 1 });
      const next = await nextDaemon(root, databasePath);
      const reads = vi.spyOn(Database.prototype, "prepare");
      await next.ingest(prompt("s", "two"));
      const ofEvents = reads.mock.calls.filter(([text]) => /\bfrom\s+"?events"?/iu.test(String(text))).length;
      reads.mockRestore();
      expect(ofEvents).toBeLessThan(LONG_LEDGER / 10);
      expect(await lostRecords(databasePath, "s")).toEqual([]);
    });

    // The ledger is not synced either: the cut can take its last lines too, and those events count as never processed.
    it("records only the events whose ledger lines the cut left", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout, storageRoot } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two", "three"]) await daemon.send(prompt("s", text));
      await daemon.stop();
      const ledger = await ledgerLinesFrom(storageRoot, "s", "s:three");
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "s:two")), { [ledger.path]: ledger.lines });
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "four"));
      expect((await lostRecords(databasePath, "s")).map(record => record.payload)).toEqual([
        expect.objectContaining({ count: 1, firstEventId: "s:two", lastEventId: "s:two" }),
      ]);
    });

    // A ledger an older build wrote dates none of its events: that build waited for the disk at every commit, so its
    // rows went only by a person's reset or scrub.
    it("takes no event of a ledger an older build wrote for lost", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout, storageRoot } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two"]) await daemon.send(prompt("s", text));
      await daemon.stop();
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "s:two")));
      const { path } = await ledgerLinesFrom(storageRoot, "s", "s:two");
      const lines = (await readFile(path, "utf8")).split("\n").filter(Boolean).map(line => JSON.parse(line) as Record<string, unknown>);
      // An older build's event line names the event alone.
      await writeFile(path, lines.map(line => `${JSON.stringify(line.type === "event" ? { type: "event", eventId: line.eventId } : line)}\n`).join(""));
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "three"));
      expect(await lostRecords(databasePath, "s")).toEqual([]);
    });

    // `rerail reset` deletes a session's history and keeps the row of a session still open: its ledger still lists every
    // event. No row of any is there, as when a power cut took them all, and nothing tells the two apart.
    it("takes no event a reset deleted for lost", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two"]) await daemon.send(prompt("s", text));
      await daemon.stop();
      const database = new Database(databasePath, { readonly: true });
      const { repositoryId } = database.prepare("SELECT repository_id AS repositoryId FROM sessions WHERE session_id = 's'").get() as { repositoryId: string };
      database.close();
      expect(resetRepositoryHistory(databasePath, repositoryId)).toMatchObject({ openSessions: 1 });
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "three"));
      expect(await lostRecords(databasePath, "s")).toEqual([]);
    });

    // The record of lost events takes its times from the ledger, and `rerail status` reads the row's: one clock reading
    // for both, whatever time passes between the row's write and the line's.
    it("gives an event's row and its ledger line one time", async () => {
      const { root, databasePath, checkout, storageRoot } = await records();
      const { start, prompt } = events(checkout);
      const next = await nextDaemon(root, databasePath);
      let tick = Date.now();
      vi.spyOn(Date, "now").mockImplementation(() => (tick += 7));
      await next.ingest(start("s"));
      await next.ingest(prompt("s", "one"));
      vi.restoreAllMocks();
      const database = new Database(databasePath, { readonly: true });
      const row = database.prepare("SELECT observed_at AS at FROM events WHERE event_id = 's:one'").get() as { at: number };
      database.close();
      const { path } = await ledgerLinesFrom(storageRoot, "s", "s:one");
      const line = (await readFile(path, "utf8")).split("\n").filter(Boolean).map(text => JSON.parse(text) as { type?: string; eventId?: string; observedAt?: number })
        .find(entry => entry.type === "event" && entry.eventId === "s:one");
      expect(line?.observedAt).toBe(row.at);
    });

    it("writes one record of a stretch however often the session opens before a later event's row", async () => {
      const { databasePath, checkout } = await records();
      const session = { host: "claude-code" as const, sessionId: "s", cwd: checkout };
      const lost = [{ eventId: "s:two", kind: "user-prompt", observedAt: 2 }, { eventId: "s:three", kind: "call", observedAt: 3 }];
      for (let open = 0; open < 2; open += 1) {
        const store = new SqliteSessionStore(databasePath);
        store.openSession(session);
        store.recordLostEvents(session, lost);
        store.close();
      }
      expect(await lostRecords(databasePath, "s")).toEqual([{
        observedAt: 3, payload: { count: 2, kinds: { "user-prompt": 1, call: 1 }, firstEventId: "s:two", lastEventId: "s:three", firstObservedAt: 2, lastObservedAt: 3 },
      }]);
    });

    // `rerail status` says hooks run but nothing was recorded since a hook failure later than the time of the newest event
    // (`hookFailureLines`, given `lastObservedEventAt`): the cut takes the newest events, so a failure while they were
    // recorded reads so until the next event is.
    it("leaves `rerail status` as main has it: right again once the next event is recorded", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two", "three"]) await daemon.send(prompt("s", text));
      await daemon.stop();
      const commits = walCommits(databasePath);
      const lostAt = stateAt(databasePath, commits.at(-1)!, database =>
        (database.prepare("SELECT observed_at AS at FROM events WHERE event_id = 's:three'").get() as { at: number }).at);
      // A hook of another session that failed while the events the cut took were recorded, at `lostAt`, counts as one
      // after which nothing was recorded.
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "s:two")));
      expect(lastObservedEventAt(databasePath)).toBeLessThan(lostAt);
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "four"));
      expect(lastObservedEventAt(databasePath)).toBeGreaterThanOrEqual(lostAt);
    });
  });

  describe("what the other readers of the store show", () => {
    // The hook process records a steer it showed (`steering-delivered`, its own writer); the footer, the feedback and the
    // evidence count those rows. The cut takes the receipt: what was lost is that record itself, the cost of the setting.
    it("counts one steer fewer in the footer when the cut takes its receipt", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      await daemon.send(prompt("s", "one"));
      const receipts = new SqliteSessionStore(databasePath);
      receipts.recordSteer({ host: "claude-code", sessionId: "s", cwd: checkout }, "r1", ["a-rule"]);
      receipts.close();
      await daemon.send(prompt("s", "two"));
      await daemon.stop();
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "steering-delivered:r1")));
      await nextDaemon(root, databasePath);
      const reader = new SqliteSessionReader(databasePath);
      try {
        expect(reader.hud("s").steeringCount).toBe(0);
      } finally {
        reader.close();
      }
    });

    // The cut took the session's every row, its own included: it began in the seconds before. Its next open wrote its
    // row with the time of that open, which no later write corrects, so the session read as starting then, for good.
    it("gives a session whose row the cut took the time its ledger says it started", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      await daemon.send(prompt("s", "one"));
      await daemon.stop();
      const [startedAt] = recordedAt(databasePath, ["s:start"]);
      cutPower(databasePath, latestCommitWhere(databasePath, database => database.prepare("SELECT 1 FROM sessions WHERE session_id = 's'").get() === undefined));
      const next = await nextDaemon(root, databasePath);
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("s", "two"));
      expect(sessionRow(databasePath, "s")?.startedAt).toBe(startedAt);
    });

    // The next start ends the session its daemon left open at the idle limit after it was last heard from: its newest
    // event the cut left, so it ends that much earlier than after its newest event, by the span of the lost commits.
    it("ends a session the gone daemon left open at the idle limit after its newest event the cut left", { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("s"));
      for (const text of ["one", "two", "three"]) await daemon.send(prompt("s", text));
      await daemon.stop();
      const [one, three] = recordedAt(databasePath, ["s:one", "s:three"]);
      cutPower(databasePath, latestCommitWhere(databasePath, database => hasEvent(database, "s:one") && !hasEvent(database, "s:two")));
      const heardAt = lastObservedEventAt(databasePath)!;
      expect(heardAt).toBeGreaterThanOrEqual(one!);
      expect(heardAt).toBeLessThan(three!);
      await nextDaemon(root, databasePath, () => Date.now() + 24 * 60 * 60_000);
      expect(sessionRow(databasePath, "s")?.endedAt).toBe(heardAt + IDLE_MS);
    });
  });
});
