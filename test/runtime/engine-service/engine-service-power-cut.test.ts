import { spawn, type ChildProcess } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../../src/api/index.js";
import type { AdapterEvent, CodeRuleAuditor } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { addRemote } from "../../../src/runtime/remote-config.js";
import { RemoteOutbox } from "../../../src/runtime/remote-outbox.js";
import { withExperiments } from "../../support/environment/experiments.js";
import { startFakeSink, storedRecords, type FakeSink } from "../../support/fake-sink.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { cutPower, latestCommitWhere } from "../../support/engine/power-cut.js";
import { startTestRepository } from "../../support/started-repository.js";

// The store's writers commit with `synchronous = NORMAL`: a power cut or an operating system crash can take the last
// commits before it, never a commit the daemon's own end follows, and keeps the earliest ones whole. What it leaves
// (`power-cut.ts`) must read right once the next daemon has started: nothing shows as running, unsent or missing for
// good because the commits after the cut never reached the disk.

const CHILD = fileURLToPath(new URL("../../support/processes/engine-service-process.ts", import.meta.url));
const IDLE_MS = 60_000;

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
async function nextDaemon(root: string, databasePath: string): Promise<EngineService> {
  const started = await startEngineService({
    socketPath: socketIn(root), storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: IDLE_MS,
    idleRuntimeMs: 0, createAuditor: quietAuditor,
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

const hasEvent = (database: Database.Database, eventId: string): boolean =>
  database.prepare("SELECT 1 FROM events WHERE event_id = ?").get(eventId) !== undefined;

const runningChecks = (sessionId: string) => (database: Database.Database): boolean =>
  (database.prepare("SELECT count(*) AS n FROM audits WHERE session_id = ? AND status = 'running'").get(sessionId) as { n: number }).n > 0;

describe("the records a power cut leaves, once the next daemon has started", () => {
  // The check's start reached the disk, its finish did not: today it reads as running until the session's evaluator
  // opens again, and for a session that never opens again, for good.
  it("closes as interrupted a check whose finish the cut took, its daemon gone", { timeout: 120_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start, writes } = events(checkout);
    const daemon = await daemonProcess(root, databasePath);
    await daemon.send(start("s"));
    await writes(daemon.send, "s", "t1", join(checkout, "a.ts"), "export const a = 1;\n");
    await daemon.stop();
    cutPower(databasePath, latestCommitWhere(databasePath, runningChecks("s")));
    await nextDaemon(root, databasePath);
    const { audits } = await dashboard(databasePath, "/api/audits?sessionId=s") as { audits: Array<{ status: string; finishedAt: number | null }> };
    expect(audits.map(audit => audit.status)).toEqual(["interrupted"]);
    expect(audits[0]!.finishedAt).not.toBeNull();
  });

  // A check of a session another daemon still runs is that daemon's to finish.
  it("leaves running a check of a daemon that still runs", { timeout: 120_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start, writes } = events(checkout);
    const daemon = await daemonProcess(root, databasePath);
    await daemon.send(start("s"));
    await writes(daemon.send, "s", "t1", join(checkout, "a.ts"), "export const a = 1;\n");
    await daemon.stop();
    cutPower(databasePath, latestCommitWhere(databasePath, runningChecks("s")));
    const alive = await daemonProcess(root, databasePath);
    await alive.send(start("o"));
    await writes(alive.send, "o", "t1", join(checkout, "b.ts"), "export const b = 1;\n");
    // Its check still runs, as one that takes long does.
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
      // Every id the remote received names one event: the lost ones it holds stay as they were.
      const named = new Map<string, string>();
      for (const record of sent()) {
        expect(named.get(record.id) ?? record.eventId, record.id).toBe(record.eventId);
        named.set(record.id, record.eventId);
      }
      // And it holds every event the store holds, each under the number the store gave it.
      const machine = [...named.keys()][0]!.split(":")[0]!;
      const stored = storedEvents(databasePath);
      expect(stored.map(row => row.eventId)).toContain("s:six");
      expect(stored.map(row => [row.eventId, named.get(`${machine}:${row.sequence}`)])).toEqual(stored.map(row => [row.eventId, row.eventId]));
    });
  });
});
