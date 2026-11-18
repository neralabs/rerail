import { randomUUID } from "node:crypto";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { join } from "node:path";
import { Writable } from "node:stream";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdapterDelivery } from "../../src/engine/index.js";
import { emitHookOutput } from "../../src/harness/hook-adapter.js";
import { runClaudeHook } from "../../src/hosts/claude/index.js";
import { encodeRerailEventResult } from "../../src/runtime/index.js";
import { hookOptions, sessionDatabaseQuery, temporaryDirectories } from "../support/hook-events.js";

const temporaryDirectory = temporaryDirectories("rerail-hook-receipt-");

/** How the fake daemon answers a receipt: takes it, answers it not found (a daemon from before), drops it, or holds it unanswered. */
type ReceiptAnswer = "take" | "refuse" | "drop" | "hold";

interface FakeDaemon {
  socketPath: string;
  /** The receipts it read, as posted. */
  receipts: unknown[];
  /** Settles once it has read its first receipt. */
  receiptRead: Promise<void>;
  /** How many connections hooks opened to it. */
  connections: number;
  close(): Promise<void>;
}

const daemons: FakeDaemon[] = [];
afterEach(async () => {
  vi.useRealTimers();
  await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
});

async function body(request: IncomingMessage): Promise<Record<string, unknown>> {
  let text = "";
  for await (const chunk of request) text += String(chunk);
  return JSON.parse(text) as Record<string, unknown>;
}

/** An engine socket that answers every event with one steer crediting `rule:a`, and receipts as `receiptAnswer` says. */
async function fakeDaemon(root: string, receiptAnswer: ReceiptAnswer): Promise<FakeDaemon> {
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-receipt-engine-${randomUUID()}` : join(root, "engine.sock");
  const read = Promise.withResolvers<void>();
  const daemon: FakeDaemon = { socketPath, receipts: [], receiptRead: read.promise, connections: 0, close: async () => undefined };
  const held: ServerResponse[] = [];
  const server = createServer(async (request, response) => {
    const posted = await body(request);
    if (request.url === "/events") {
      const { eventId, sessionId } = posted.event as { eventId: string; sessionId: string };
      const steer: AdapterDelivery = { sessionId, requested: "steer", mechanism: "additional-context", content: "Split the function.", ruleIds: ["rule:a"], rules: [] };
      response.end(encodeRerailEventResult({ eventId, deliveries: [steer] }));
      return;
    }
    daemon.receipts.push(posted);
    read.resolve();
    if (receiptAnswer === "take") response.end("{}");
    else if (receiptAnswer === "refuse") response.writeHead(404).end();
    else if (receiptAnswer === "drop") request.socket.destroy();
    else held.push(response);
  });
  server.on("connection", () => { daemon.connections += 1; });
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  daemon.close = async () => {
    for (const response of held) response.destroy();
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
  };
  daemons.push(daemon);
  return daemon;
}

/** A steered PreToolUse answered by `daemon`: the hook's answer, its receipt not yet recorded. */
async function steeredAnswer(root: string, daemon: FakeDaemon): Promise<Record<string, unknown>> {
  const { embedder: _embedder, ...options } = hookOptions(root);
  return await runClaudeHook({
    session_id: "session-1", cwd: root, hook_event_name: "PreToolUse",
    tool_name: "Write", tool_use_id: "write-1", tool_input: { file_path: join(root, "a.py"), content: "x = 1\n" },
  }, { ...options, kernelSocketPath: daemon.socketPath });
}

const sink = (): Writable => new Writable({ write(_chunk, _encoding, callback) { callback(); } });

function receiptRows(root: string): Array<{ eventId: string; payloadJson: string }> {
  return sessionDatabaseQuery(root, "SELECT event_id AS eventId, payload_json AS payloadJson FROM events WHERE kind = 'steering-delivered'");
}

/** Another connection holding the database's write lock, as the daemon, a CLI command or another hook does for a while. */
function holdWriteLock(root: string): () => void {
  const writer = new Database(join(root, "rerail.sqlite"));
  writer.exec("BEGIN IMMEDIATE");
  return () => {
    writer.exec("ROLLBACK");
    writer.close();
  };
}

const handed = (root: string) => [{
  receipt: { session: { host: "claude-code", sessionId: "session-1", cwd: root }, receiptId: expect.any(String), ruleIds: ["rule:a"] },
  configuration: expect.objectContaining({ databasePath: join(root, "rerail.sqlite"), workspaceRoot: root }),
}];

describe("a steered hook's receipt", () => {
  it("is recorded by the hook itself at once while the database's write lock is free, handing the daemon nothing", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, "take");
    await emitHookOutput(await steeredAnswer(root, daemon), sink());
    expect(daemon.receipts).toEqual([]);
    const [row] = receiptRows(root);
    expect(row?.eventId).toMatch(/^steering-delivered:/u);
    expect(JSON.parse(row!.payloadJson)).toEqual({ ruleIds: ["rule:a"] });
  });

  it("goes to the daemon that answered the event, on the event's connection, while another connection holds the write lock", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, "take");
    const output = await steeredAnswer(root, daemon);
    expect(JSON.stringify(output)).toContain("Split the function.");
    // Before, the hook waited for the lock up to SQLite's 5 s busy timeout, then lost the receipt.
    const release = holdWriteLock(root);
    // The hook's own tries wait on timers held here, so only the daemon's answer can end it.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      await emitHookOutput(output, sink());
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      release();
    }
    expect(daemon.receipts).toEqual(handed(root));
    expect(daemon.connections).toBe(1);
    // The daemon keeps it: the hook wrote nothing.
    expect(receiptRows(root)).toEqual([]);
  });

  it.each(["refuse", "drop"] as const)("is recorded by the hook itself once the lock is given back when the daemon does not take it (%s)", { timeout: 20_000 }, async answer => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, answer);
    const output = await steeredAnswer(root, daemon);
    const release = holdWriteLock(root);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const emitted = emitHookOutput(output, sink());
    await daemon.receiptRead;
    expect(daemon.receipts).toEqual(handed(root));
    // The refusal reaches the hook in a few turns of this thread's event loop (setImmediate is not faked here).
    for (let turn = 0; turn < 10; turn += 1) await new Promise(resolve => setImmediate(resolve));
    // The hook tries the lock at SQLite's busy pauses (1, 2, 5, 10 ms) as its busy wait did, between its turns: this
    // thread, which holds the lock, runs meanwhile, so it can give the lock back.
    await vi.advanceTimersByTimeAsync(20);
    expect(receiptRows(root)).toEqual([]);
    release();
    await vi.advanceTimersByTimeAsync(20);
    await emitted;
    expect(receiptRows(root)).toHaveLength(1);
  });

  it("is recorded by the hook itself once the lock is given back while the daemon reads it but does not answer", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, "hold");
    const output = await steeredAnswer(root, daemon);
    const release = holdWriteLock(root);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const emitted = emitHookOutput(output, sink());
    await daemon.receiptRead;
    await vi.advanceTimersByTimeAsync(20);
    expect(receiptRows(root)).toEqual([]);
    release();
    // The daemon is busy with other work or stopped: the lock given back ends the hook's wait, as it did before.
    await vi.advanceTimersByTimeAsync(20);
    expect(receiptRows(root)).toHaveLength(1);
    await emitted;
    expect(vi.getTimerCount()).toBe(0);
  });

  it("ends at SQLite's busy timeout, the receipt left with the daemon, while neither the daemon answers nor the lock is given back", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, "hold");
    const output = await steeredAnswer(root, daemon);
    const release = holdWriteLock(root);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const emitted = emitHookOutput(output, sink());
      await daemon.receiptRead;
      // Past SQLite's 5 s busy timeout, the bound the hook's own wait for the lock had.
      await vi.advanceTimersByTimeAsync(5_000);
      await emitted;
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      release();
    }
    // Read by the daemon, which records it whenever it answers; before, the hook lost it at this bound.
    expect(daemon.receipts).toEqual(handed(root));
    expect(receiptRows(root)).toEqual([]);
  });
});
