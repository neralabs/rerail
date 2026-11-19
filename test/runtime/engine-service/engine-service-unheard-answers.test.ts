import { subscribe, unsubscribe } from "node:diagnostics_channel";
import { once } from "node:events";
import { mkdtemp } from "node:fs/promises";
import { request, ServerResponse } from "node:http";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import type { AdapterDelivery, AdapterEvent, RuleSteerDelivery } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
import { decodeRerailEventResult } from "../../../src/runtime/event-protocol.js";
import { testEmbedder } from "../../support/hook-events.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

const SESSION = "gone";
const STEER = "Rerail found 1 problem in your last edit: a.py calls eval.";
const OTHER = "other";
const OTHER_STEER = "Rerail found 1 problem in your last edit: b.py calls eval.";

// A hook asks the daemon with a time limit of its own: past it, the hook stops waiting and closes its connection, while
// the daemon still runs the event and answers it. A steer such an answer takes reaches no agent.
describe("engine service answers a hook no longer waits for", () => {
  it("keeps a steer for the session's next hook when the hook whose answer took it gave up while its turn's end waited", async () => {
    const service = await checkingService();
    const stop = hook(service.socketPath, turnEnd("stop-1", service.root));
    // The turn's end waits for the session's edit checks; the hook gives up meanwhile, and then the check finds something.
    await service.checking;
    await stop.giveUp();
    service.finishCheck();
    await service.steered;
    expect(contents(await ingestEngineEvent(service.engine, prompt("prompt-2")))).toEqual([STEER]);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-3"))).toEqual([]);
  }, 20_000);

  it("tells a steer once when the hook whose answer took it still waited", async () => {
    const service = await checkingService();
    const stop = hook(service.socketPath, turnEnd("stop-1", service.root));
    await service.checking;
    service.finishCheck();
    expect(contents(await stop.answer)).toEqual([STEER]);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-2"))).toEqual([]);
  }, 20_000);

  it("tells a steer once when its answer waits on its connection behind another session's answer", async () => {
    const service = await checkingService();
    await service.start(OTHER);
    await service.steer(OTHER, OTHER_STEER);
    // One connection carries the turn end of `gone`, held by its check, and then a prompt of the other session, whose
    // answer is ready first and waits for its turn on the connection.
    const responses = serverResponses(2);
    const connection = await pipelined(service.socketPath, [turnEnd("stop-1", service.root), prompt("other-1", OTHER)]);
    await service.checking;
    const answers = await responses;
    await vi.waitFor(() => expect(answers[1]?.writableEnded).toBe(true), { timeout: 5_000, interval: 10 });
    service.finishCheck();
    const received = await connection.received;
    expect({ steer: received.split(STEER).length - 1, other: received.split(OTHER_STEER).length - 1 }).toEqual({ steer: 1, other: 1 });
    expect(await ingestEngineEvent(service.engine, prompt("other-2", OTHER))).toEqual([]);
  }, 20_000);
});

function turnEnd(eventId: string, cwd: string): AdapterEvent {
  return { kind: "turn-end", eventId, sessionId: SESSION, session: { host: "claude-code", sessionId: SESSION, cwd }, text: "" };
}

function prompt(eventId: string, sessionId = SESSION): AdapterEvent {
  return { kind: "user-prompt", eventId, sessionId, text: "next" };
}

/** The responses of the next `count` requests a server in this process reads the headers of. */
function serverResponses(count: number): Promise<ServerResponse[]> {
  const responses: ServerResponse[] = [];
  const all = Promise.withResolvers<ServerResponse[]>();
  const take = (message: unknown): void => {
    const response: unknown = typeof message === "object" && message !== null ? Reflect.get(message, "response") : undefined;
    if (!(response instanceof ServerResponse)) return;
    responses.push(response);
    if (responses.length < count) return;
    unsubscribe("http.server.request.start", take);
    all.resolve(responses);
  };
  subscribe("http.server.request.start", take);
  onTestFinished(() => {
    unsubscribe("http.server.request.start", take);
  });
  return all.promise;
}

/**
 * Sends `events` on one connection, one request after another without waiting for answers, once connected; `received`
 * resolves with all the connection read once the server closes it.
 */
async function pipelined(socketPath: string, events: AdapterEvent[]): Promise<{ received: Promise<string> }> {
  const socket = connect(socketPath);
  socket.on("error", () => undefined);
  onTestFinished(() => {
    socket.destroy();
  });
  await once(socket, "connect");
  let received = "";
  socket.on("data", (chunk: Buffer) => (received += chunk.toString("utf8")));
  const closed = new Promise<string>(done => socket.once("close", () => done(received)));
  socket.write(events.map((event, index) => {
    const body = JSON.stringify({ event });
    const last = index === events.length - 1 ? "connection: close\r\n" : "";
    return `POST /events HTTP/1.1\r\nHost: localhost\r\n${last}content-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body}`;
  }).join(""));
  return { received: closed };
}

function contents(deliveries: readonly AdapterDelivery[]): string[] {
  return deliveries.map(delivery => delivery.content);
}

/** A hook's request for `event`, sent whole; `giveUp` closes its connection, as the hook does past its own limit. */
function hook(socketPath: string, event: AdapterEvent): { answer: Promise<readonly AdapterDelivery[]>; giveUp: () => Promise<void> } {
  const body = JSON.stringify({ event });
  const outgoing = request({ socketPath, method: "POST", path: "/events", agent: false, headers: { "content-type": "application/json", "content-length": Buffer.byteLength(body) } });
  // Its own close, not `once`, which rejects on the error a destroyed request emits.
  const closed = new Promise<void>(done => outgoing.once("close", () => done()));
  const answer = new Promise<readonly AdapterDelivery[]>((resolve, reject) => {
    outgoing.on("response", response => {
      const chunks: Buffer[] = [];
      response.on("data", (chunk: Buffer) => chunks.push(chunk));
      response.on("end", () => resolve(decodeRerailEventResult(Buffer.concat(chunks).toString("utf8")).deliveries));
    });
    outgoing.on("error", reject);
  });
  answer.catch(() => undefined);
  outgoing.end(body);
  onTestFinished(() => {
    outgoing.destroy();
  });
  return {
    answer,
    giveUp: async () => {
      outgoing.destroy();
      await closed;
    },
  };
}

/**
 * An engine service in a folder of its own with the Claude Code session `gone` started and its evaluator open. The
 * session's edit checks hold until `finishCheck`, and the first that ends then steers `STEER`: `checking` resolves
 * once a boundary waits for them, `steered` once the steer is asked for. `start` starts another session there, and
 * `steer` asks for a steer in a session as its evaluator would.
 */
async function checkingService(): Promise<{
  root: string;
  socketPath: string;
  engine: { socketPath: string };
  checking: Promise<void>;
  steered: Promise<void>;
  finishCheck: () => void;
  start: (sessionId: string) => Promise<void>;
  steer: (sessionId: string, content: string) => Promise<void>;
}> {
  const root = await mkdtemp(join(tmpdir(), "rerail-unheard-"));
  const socketPath = testEngineEndpoint(root);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const opens = new Map<string, PromiseWithResolvers<void>>();
  const openOf = (sessionId: string): PromiseWithResolvers<void> => {
    const open = opens.get(sessionId) ?? Promise.withResolvers<void>();
    opens.set(sessionId, open);
    return open;
  };
  let delivery: RuleSteerDelivery | undefined;
  const checking = Promise.withResolvers<void>();
  const check = Promise.withResolvers<void>();
  const steered = Promise.withResolvers<void>();
  let steers = 1;
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    createAuditor: ({ steerDelivery }) => ({
      openWorkspace: async sessionId => {
        delivery = steerDelivery;
        openOf(sessionId).resolve();
      },
      // No file of the folder is a source, so no walk moves the session's revision past the steer's.
      acceptsPath: () => false,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
      settleGeneratedEdits: async () => {
        checking.resolve();
        await check.promise;
        if (steers-- <= 0) return;
        await steerDelivery.requestSteer(SESSION, "steer", STEER, { revision: 0, scope: [], background: true });
        steered.resolve();
      },
      close: async () => undefined,
    }),
  });
  onTestFinished(async () => {
    check.resolve();
    await service.close();
    await removeFolder(root);
  });
  const start = async (sessionId: string): Promise<void> => {
    await service.ingest({ kind: "session-start", eventId: `start-${sessionId}`, session: { host: "claude-code", sessionId, cwd: root } });
    // The evaluator opens in the background; a boundary before the runtime has taken that in has no edit checks to wait for.
    await openOf(sessionId).promise;
    await new Promise<void>(done => setImmediate(done));
  };
  await start(SESSION);
  return {
    root, socketPath, engine: { socketPath }, checking: checking.promise, steered: steered.promise, finishCheck: () => check.resolve(), start,
    steer: async (sessionId, content) => {
      if (delivery === undefined) throw new Error("no evaluator has opened");
      await delivery.requestSteer(sessionId, "steer", content, { revision: 0, scope: [], background: true });
    },
  };
}
