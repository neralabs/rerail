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

describe("a steered hook's receipt", () => {
  it("goes to the daemon that answered the event, on the event's connection, while another connection holds the database's write lock", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, "take");
    const output = await steeredAnswer(root, daemon);
    expect(JSON.stringify(output)).toContain("Split the function.");
    // A writer holds the database: a hook that recorded the receipt itself waited out SQLite's 5 s busy timeout, then lost it.
    const writer = new Database(join(root, "rerail.sqlite"));
    writer.exec("BEGIN IMMEDIATE");
    // The engine socket here answers in this process; held still, its bound cannot run out on a slow machine first.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      await emitHookOutput(output, sink());
    } finally {
      writer.exec("ROLLBACK");
      writer.close();
    }
    expect(daemon.receipts).toEqual([{
      receipt: { session: { host: "claude-code", sessionId: "session-1", cwd: root }, receiptId: expect.any(String), ruleIds: ["rule:a"] },
      configuration: expect.objectContaining({ databasePath: join(root, "rerail.sqlite"), workspaceRoot: root }),
    }]);
    expect(daemon.connections).toBe(1);
    // The daemon records it: the hook wrote nothing.
    expect(receiptRows(root)).toEqual([]);
  });

  it.each(["refuse", "drop"] as const)("is recorded by the hook itself when the daemon does not take it (%s)", { timeout: 20_000 }, async answer => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, answer);
    await emitHookOutput(await steeredAnswer(root, daemon), sink());
    expect(daemon.receipts).toHaveLength(1);
    const [row] = receiptRows(root);
    expect(row?.eventId).toBe(`steering-delivered:${(daemon.receipts[0] as { receipt: { receiptId: string } }).receipt.receiptId}`);
    expect(JSON.parse(row!.payloadJson)).toEqual({ ruleIds: ["rule:a"] });
  });

  it("is recorded by the hook itself when the daemon reads nothing within the bound a receipt has", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const daemon = await fakeDaemon(root, "hold");
    const output = await steeredAnswer(root, daemon);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const emitted = emitHookOutput(output, sink());
    // Whichever comes first: the daemon reads the receipt, or the hook ends without handing it over.
    await Promise.race([daemon.receiptRead, emitted]);
    expect(daemon.receipts).toHaveLength(1);
    expect(receiptRows(root)).toEqual([]);
    // Within what the hook's own record takes (two SQLite opens: 96 ms at the 90th percentile on a busy Windows 11
    // machine), the hook gives up on a daemon too busy to read the receipt: no timer of the hand-over is left.
    await vi.advanceTimersByTimeAsync(100);
    expect(vi.getTimerCount()).toBe(0);
    await emitted;
    expect(receiptRows(root)).toHaveLength(1);
  });
});
