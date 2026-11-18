import { createHash } from "node:crypto";
import { subscribe, unsubscribe } from "node:diagnostics_channel";
import { once } from "node:events";
import { mkdtemp, readFile } from "node:fs/promises";
import { ServerResponse } from "node:http";
import { connect, Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFinished } from "vitest";
import { ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
import { EVENT_ANSWER_HEADER, EVENT_ANSWER_WITH_FACTS } from "../../../src/runtime/event-protocol.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

/** The steer the turn's edit checks find once the test lets them end. */
const LATE = "late steer";
const SESSION = "hook-gave-up";

/**
 * An engine service in a folder of its own with a Claude Code session started, whose turn's edit checks end only once
 * the test calls `release`, then steer `LATE` once. With `facts`, it tells a hook that asks the facts of each answer, as
 * the daemon's does.
 */
async function heldStopService(facts = false) {
  const root = await mkdtemp(join(tmpdir(), "rerail-hook-gave-up-"));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  const socketPath = testEngineEndpoint(root);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const opened = Promise.withResolvers<void>();
  const settling = Promise.withResolvers<void>();
  const checks = Promise.withResolvers<void>();
  const steered = Promise.withResolvers<void>();
  let steers = 0;
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    ...(facts ? { answerFacts: () => ({ dashboard: null }) } : {}),
    createAuditor: ({ steerDelivery }) => ({
      openWorkspace: async () => {
        opened.resolve();
      },
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
      settleGeneratedEdits: async () => {
        settling.resolve();
        await checks.promise;
        if (steers++ > 0) return;
        await steerDelivery.requestSteer(SESSION, "steer", LATE, { revision: 0, scope: [] });
        steered.resolve();
      },
      close: async () => undefined,
    }),
  });
  // Vitest runs clean-ups in reverse order, so the service closes before its folder goes; the checks end so it can.
  onTestFinished(async () => {
    checks.resolve();
    await service.close();
  });
  const session = { host: "claude-code" as const, sessionId: SESSION, cwd: root };
  await service.ingest({ kind: "session-start", eventId: "start", session });
  // The evaluator opens in the background; the stop waits for its checks only once the runtime has taken it in.
  await opened.promise;
  await new Promise<void>(done => setImmediate(done));
  const engine = { socketPath };
  const next = async (eventId: string) => (await ingestEngineEvent(engine, { kind: "user-prompt", eventId, sessionId: SESSION, text: "next" }))
    .map(delivery => delivery.content);
  return { root, engine, settling: settling.promise, release: () => checks.resolve(), steered: steered.promise, next };
}

/** The server's side of the next connection the engine takes in. */
function nextServerSocket(): Promise<Socket> {
  const taken = Promise.withResolvers<Socket>();
  const take = (message: unknown): void => {
    const socket: unknown = typeof message === "object" && message !== null ? Reflect.get(message, "socket") : undefined;
    if (!(socket instanceof Socket)) return;
    unsubscribe("net.server.socket", take);
    taken.resolve(socket);
  };
  subscribe("net.server.socket", take);
  onTestFinished(() => {
    unsubscribe("net.server.socket", take);
  });
  return taken.promise;
}

/** The engine's answer to the next event request it takes in, so a test can see whether it has been written. */
function nextEventAnswer(): Promise<ServerResponse> {
  const taken = Promise.withResolvers<ServerResponse>();
  const take = (message: unknown): void => {
    const response: unknown = typeof message === "object" && message !== null ? Reflect.get(message, "response") : undefined;
    if (!(response instanceof ServerResponse) || response.req.url !== "/events") return;
    unsubscribe("http.server.request.start", take);
    taken.resolve(response);
  };
  subscribe("http.server.request.start", take);
  onTestFinished(() => {
    unsubscribe("http.server.request.start", take);
  });
  return taken.promise;
}

/**
 * Sends the turn's end as a hook does, with `headers`, on a connection the test closes as the hook gives up: `giveUp`
 * resolves once the engine's side of it has closed.
 */
async function stopHook(socketPath: string, root: string, headers: string): Promise<{ giveUp: () => Promise<void> }> {
  const accepted = nextServerSocket();
  const client = connect(socketPath);
  client.on("error", () => undefined);
  onTestFinished(() => {
    client.destroy();
  });
  await once(client, "connect");
  const body = JSON.stringify({ event: { kind: "turn-end", eventId: "stop", sessionId: SESSION, session: { host: "claude-code", sessionId: SESSION, cwd: root }, text: "" } });
  client.write(`POST /events HTTP/1.1\r\nHost: localhost\r\nconnection: close\r\n${headers}content-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
  const server = await accepted;
  const closed = once(server, "close");
  return {
    giveUp: async () => {
      client.destroy();
      await closed;
    },
  };
}

/** How many times the session's ledger records `content` as delivered to its agent. */
async function recordedDeliveries(root: string, content: string): Promise<number> {
  const ledger = await readFile(join(root, "state", createHash("sha256").update(SESSION).digest("hex"), "events.jsonl"), "utf8");
  return ledger.split("\n").filter(line => line.includes("\"type\":\"delivery\"") && line.includes(content)).length;
}

/** The hooks a stop comes from: one older than the answer's facts, and one that reads them, as every hook now does. */
const STOP_HOOKS = [
  { hook: "a hook that reads no facts of the answer", facts: false, headers: "" },
  { hook: "a hook that reads the answer's facts", facts: true, headers: `${EVENT_ANSWER_HEADER}: ${EVENT_ANSWER_WITH_FACTS}\r\n` },
];

describe("an engine service whose hook gave up on a stop", () => {
  it.each(STOP_HOOKS)("hands the steers the stop's checks find to the session's next event, once, and records them once, from $hook", async ({ facts, headers }) => {
    const held = await heldStopService(facts);
    const { giveUp } = await stopHook(held.engine.socketPath, held.root, headers);
    await held.settling;
    await giveUp();
    held.release();
    await held.steered;
    // Before, the stop took the steer into the answer its closed connection no longer carried, and recorded it as
    // delivered; the next prompt got nothing.
    expect(await held.next("next")).toEqual([LATE]);
    expect(await held.next("after")).toEqual([]);
    expect(await recordedDeliveries(held.root, LATE)).toBe(1);
  });

  it("answers a stop whose hook still waits once its checks end, with the steers they find", async () => {
    const held = await heldStopService();
    const answer = nextEventAnswer();
    const stop = ingestEngineEvent(held.engine, { kind: "turn-end", eventId: "stop", sessionId: SESSION, text: "" });
    const response = await answer;
    await held.settling;
    // From the checks' start to the written answer only promise jobs run when nothing is pending, so a stop that stopped
    // waiting for its checks while its hook still waits would have written its answer by now, with none of their steers.
    await new Promise<void>(done => setImmediate(done));
    expect(response.writableEnded).toBe(false);
    held.release();
    expect((await stop).map(delivery => delivery.content)).toEqual([LATE]);
    expect(await held.next("next")).toEqual([]);
  });
});
