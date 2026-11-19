import { subscribe, unsubscribe } from "node:diagnostics_channel";
import { once } from "node:events";
import { mkdtemp } from "node:fs/promises";
import { request, ServerResponse } from "node:http";
import { connect, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import type { AdapterDelivery, AdapterEvent, RuleSteerDelivery } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
import { decodeRerailEventResult } from "../../../src/runtime/event-protocol.js";
import { testEmbedder } from "../../support/hook-events.js";
import { deliveryEntries } from "../../support/ledger/deliveries.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

const SESSION = "gone";
const STEER = "Rerail found 1 problem in your last edit: a.py calls eval.";
const OTHER = "other";
const OTHER_STEER = "Rerail found 1 problem in your last edit: b.py calls eval.";
const SECOND_STEER = "Rerail found 1 problem in your last edit: c.py calls eval.";
const NEWER_STEER = "Rerail found 1 problem in your last edit: d.py calls eval.";
// Bigger than a connection holds unread (a Windows named pipe takes 64 KB, a Linux Unix socket about 200 KB), so a
// client that has stopped reading never lets its answer's write complete.
const BIG_STEER = `Rerail found many problems in your last edit: big.py calls eval.\n${"- big.py calls eval [python.eval]\n".repeat(24_000)}`;

// A hook asks the daemon with a time limit of its own: past it, the hook stops waiting and closes its connection, while
// the daemon still runs the event and answers it. A steer such an answer takes reaches no agent.
describe("engine service answers a hook no longer waits for", () => {
  it("keeps a steer for the session's next hook when the hook whose answer took it gave up while its turn's end waited", async () => {
    const service = await checkingService();
    const responses = serverResponses(1);
    const stop = hook(service.socketPath, turnEnd("stop-1", service.root));
    // The turn's end waits for the session's edit checks; the hook gives up meanwhile, the service reads its close, and
    // then the check finds something.
    await service.checking;
    const [response] = await responses;
    const closed = new Promise<void>(done => response?.once("close", () => done()));
    await stop.giveUp();
    await closed;
    service.finishCheck();
    await service.steered;
    expect(contents(await ingestEngineEvent(service.engine, prompt("prompt-2")))).toEqual([STEER]);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-3"))).toEqual([]);
    // Recorded once, by the answer that never reached its hook.
    expect(await service.deliveriesOf(STEER)).toBe(1);
  });

  it("keeps a steer for the session's next hook when the daemon writes the answer before it has read the hook's close", async ({ task }) => {
    const service = await checkingService();
    const responses = serverResponses(2);
    const stop = hook(service.socketPath, turnEnd("stop-1", service.root));
    await service.checking;
    // The session's next hook asks meanwhile and waits behind the turn's end, as a hook does on a busy daemon.
    const next = ingestEngineEvent(service.engine, prompt("prompt-2"));
    const [answer, queued] = await responses;
    await vi.waitFor(() => expect(queued?.req.readableEnded).toBe(true), { timeout: task.timeout, interval: 10 });
    await new Promise<void>(done => setImmediate(done));
    // The hook goes as its answer is written, before the daemon can read its close: the write is refused.
    atItsAnswer(answer, stop.destroy);
    service.finishCheck();
    expect(contents(await next)).toEqual([STEER]);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-3"))).toEqual([]);
  });

  it("tells a steer once when the hook whose answer took it still waited", async () => {
    const service = await checkingService();
    const stop = hook(service.socketPath, turnEnd("stop-1", service.root));
    await service.checking;
    service.finishCheck();
    expect(contents(await stop.answer)).toEqual([STEER]);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-2"))).toEqual([]);
  });

  it("answers the session's next hook while a hook that has stopped reading holds its answer, which it then reads whole", async ({ task }) => {
    const service = await checkingService();
    await service.steer(SESSION, BIG_STEER);
    const stuck = await stuckHook(service.socketPath, prompt("prompt-1"), task.timeout);
    // Answered while the stuck hook reads nothing of its own answer.
    expect(await ingestEngineEvent(service.engine, prompt("prompt-2"))).toEqual([]);
    const received = await stuck.readAll();
    expect(received.split(BIG_STEER.slice(0, 64)).length - 1).toBe(1);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-3"))).toEqual([]);
  });

  it("gives the steers of an answer a hook stopped reading to the session's next hook once that hook gives up, before newer ones", async ({ task }) => {
    const service = await checkingService();
    await service.steer(SESSION, BIG_STEER);
    await service.steer(SESSION, SECOND_STEER);
    const stuck = await stuckHook(service.socketPath, prompt("prompt-1"), task.timeout);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-2"))).toEqual([]);
    await service.steer(SESSION, NEWER_STEER);
    await stuck.giveUp();
    expect(leads(await ingestEngineEvent(service.engine, prompt("prompt-3")))).toEqual(leads([BIG_STEER, SECOND_STEER, NEWER_STEER]));
    expect(await ingestEngineEvent(service.engine, prompt("prompt-4"))).toEqual([]);
  });

  it("keeps no steer of a session's earlier opening when the hook that stopped reading its answer gives up after the session opened again", async ({ task }) => {
    let clock = 1_000;
    const service = await checkingService({ now: () => clock, idleSessionMs: 1_000 });
    await service.steer(SESSION, BIG_STEER);
    const stuck = await stuckHook(service.socketPath, prompt("prompt-1"), task.timeout);
    // The session goes quiet and closes, and its next hook opens it again, while the stuck hook still holds its answer.
    clock += 60_000;
    await service.closeIdleSessions();
    expect(await ingestEngineEvent(service.engine, prompt("prompt-2", SESSION, service.root))).toEqual([]);
    await stuck.giveUp();
    expect(await ingestEngineEvent(service.engine, prompt("prompt-3", SESSION, service.root))).toEqual([]);
  });

  it("keeps the steer a session owed when it opened again for its next hook when the hook whose answer took it gave up", async () => {
    let clock = 1_000;
    const service = await checkingService({ now: () => clock, idleSessionMs: 1_000 });
    await service.steer(SESSION, STEER);
    // Quiet past the limit for a session that owes a steer: it closes, and its ledger keeps the steer owed.
    clock += 2 * 24 * 60 * 60_000;
    await service.closeIdleSessions();
    const responses = serverResponses(1);
    const gone = hook(service.socketPath, prompt("prompt-2", SESSION, service.root));
    atItsAnswer((await responses)[0], gone.destroy);
    await gone.closed;
    expect(contents(await ingestEngineEvent(service.engine, prompt("prompt-3")))).toEqual([STEER]);
    expect(await ingestEngineEvent(service.engine, prompt("prompt-4"))).toEqual([]);
    expect(await service.deliveriesOf(STEER)).toBe(1);
  });

  it("records a steer once when the daemon restarts between the answer no hook read and the session's next hook", async () => {
    const first = await checkingService();
    await first.steer(SESSION, STEER);
    const responses = serverResponses(1);
    const gone = hook(first.socketPath, prompt("prompt-1"));
    atItsAnswer((await responses)[0], gone.destroy);
    await gone.closed;
    await first.close();
    const second = await checkingService({ root: first.root });
    expect(contents(await ingestEngineEvent(second.engine, prompt("prompt-2", SESSION, second.root)))).toEqual([STEER]);
    expect(await ingestEngineEvent(second.engine, prompt("prompt-3"))).toEqual([]);
    expect(await second.deliveriesOf(STEER)).toBe(1);
  });

  it("tells a steer once when its answer waits on its connection behind another session's answer", async ({ task }) => {
    const service = await checkingService();
    await service.start(OTHER);
    await service.steer(OTHER, OTHER_STEER);
    // One connection carries the turn end of `gone`, held by its check, and then a prompt of the other session, whose
    // answer is ready first and waits for its turn on the connection.
    const responses = serverResponses(2);
    const connection = await pipelined(service.socketPath, [turnEnd("stop-1", service.root), prompt("other-1", OTHER)]);
    await service.checking;
    const answers = await responses;
    await vi.waitFor(() => expect(answers[1]?.writableEnded).toBe(true), { timeout: task.timeout, interval: 10 });
    service.finishCheck();
    const received = await connection.received;
    expect({ steer: received.split(STEER).length - 1, other: received.split(OTHER_STEER).length - 1 }).toEqual({ steer: 1, other: 1 });
    expect(await ingestEngineEvent(service.engine, prompt("other-2", OTHER))).toEqual([]);
  });

  it("leaves no listener of its own on a connection for each answer it writes there", async ({ task }) => {
    const service = await checkingService();
    const listeners = async (count: number): Promise<number> => {
      const responses = serverResponses(count);
      await pipelined(service.socketPath, Array.from({ length: count }, (_, index) => prompt(`kept-${count}-${index}`)), false);
      const answers = await responses;
      await vi.waitFor(() => expect(answers.every(answer => answer.writableFinished)).toBe(true), { timeout: task.timeout, interval: 10 });
      return answers[0]?.req.socket.listenerCount("close") ?? -1;
    };
    expect(await listeners(12)).toBe(await listeners(1));
  });

  it("answers a session's end once the service has let go of the session, as before", async () => {
    const service = await checkingService({ idleRuntimeMs: 0 });
    const responses = serverResponses(1);
    const ending = ingestEngineEvent(service.engine, { kind: "session-end", eventId: "end", sessionId: SESSION, session: { host: "claude-code", sessionId: SESSION, cwd: service.root } });
    // What the service holds as it writes the answer: the session's runtime is gone already.
    let openWhenAnswered = -1;
    atItsAnswer((await responses)[0], () => (openWhenAnswered = service.openRuntimes()));
    await ending;
    expect(openWhenAnswered).toBe(0);
  });
});

function turnEnd(eventId: string, cwd: string): AdapterEvent {
  return { kind: "turn-end", eventId, sessionId: SESSION, session: { host: "claude-code", sessionId: SESSION, cwd }, text: "" };
}

/** A prompt of `sessionId`; with `cwd`, it says where the session runs, which a session the service does not hold needs. */
function prompt(eventId: string, sessionId = SESSION, cwd?: string): AdapterEvent {
  const session = cwd === undefined ? {} : { session: { host: "claude-code" as const, sessionId, cwd } };
  return { kind: "user-prompt", eventId, sessionId, text: "next", ...session };
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
 * Has `act` run as the server writes `response`, in the same turn and before the write: a client that leaves then goes
 * before the daemon can read that it has gone, and what `act` reads is what the service holds as it answers.
 */
function atItsAnswer(response: ServerResponse | undefined, act: () => void): void {
  if (response === undefined) throw new Error("the server read no request");
  const end = response.end.bind(response);
  response.end = ((...args: Parameters<typeof end>) => {
    act();
    return end(...args);
  }) as typeof response.end;
}

/**
 * Sends `events` on one connection, one request after another without waiting for answers, once connected; `received`
 * resolves with all the connection read once the server closes it, which the last request asks for unless `closes` is
 * false.
 */
async function pipelined(socketPath: string, events: AdapterEvent[], closes = true): Promise<{ received: Promise<string> }> {
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
    const last = closes && index === events.length - 1 ? "connection: close\r\n" : "";
    return `POST /events HTTP/1.1\r\nHost: localhost\r\n${last}content-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body}`;
  }).join(""));
  return { received: closed };
}

/**
 * A hook alive that has stopped reading: it sends `event` and reads nothing of the answer, which the server has started
 * to write when this resolves, within `limit` ms (the case's own limit). `readAll` reads it whole at last; `giveUp`
 * closes the connection, once the server has seen it close.
 */
async function stuckHook(socketPath: string, event: AdapterEvent, limit: number): Promise<{ readAll: () => Promise<string>; giveUp: () => Promise<void> }> {
  const responses = serverResponses(1);
  const socket: Socket = connect(socketPath);
  socket.on("error", () => undefined);
  onTestFinished(() => {
    socket.destroy();
  });
  await once(socket, "connect");
  // Paused before anything arrives: it reads only what its own buffer takes, and then nothing.
  socket.pause();
  const body = JSON.stringify({ event });
  socket.write(`POST /events HTTP/1.1\r\nHost: localhost\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
  const [response] = await responses;
  if (response === undefined) throw new Error("the server read no request");
  await vi.waitFor(() => expect(response.writableEnded).toBe(true), { timeout: limit, interval: 10 });
  expect(response.writableFinished).toBe(false);
  return {
    readAll: async () => {
      let received = "";
      socket.on("data", (chunk: Buffer) => (received += chunk.toString("utf8")));
      socket.resume();
      await once(socket, "close");
      return received;
    },
    giveUp: async () => {
      const closed = new Promise<void>(done => response.once("close", () => done()));
      socket.destroy();
      await closed;
    },
  };
}

function contents(deliveries: readonly AdapterDelivery[]): string[] {
  return deliveries.map(delivery => delivery.content);
}

/** The first line of each steer, which tells them apart: a failure then prints no steer of many lines whole. */
function leads(steers: readonly (AdapterDelivery | string)[]): string[] {
  return steers.map(steer => (typeof steer === "string" ? steer : steer.content).split("\n")[0] ?? "");
}

/**
 * A hook's request for `event`, sent whole; `giveUp` closes its connection, as the hook does past its own limit, and
 * resolves once it has; `destroy` closes it in the same turn, and `closed` resolves once it has.
 */
function hook(socketPath: string, event: AdapterEvent): {
  answer: Promise<readonly AdapterDelivery[]>; giveUp: () => Promise<void>; destroy: () => void; closed: Promise<void>;
} {
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
    closed,
    destroy: () => outgoing.destroy(),
    giveUp: async () => {
      outgoing.destroy();
      await closed;
    },
  };
}

interface ServiceOptions {
  /** 0 closes a runtime as its last session ends. */
  idleRuntimeMs?: number;
  idleSessionMs?: number;
  now?: () => number;
  /** A folder an earlier service of the test ran in: this one runs over its records, as a daemon started again does. */
  root?: string;
}

/**
 * An engine service in a folder of its own with the Claude Code session `gone` started and its evaluator open. The
 * session's edit checks hold until `finishCheck`, and the first that ends then steers `STEER`: `checking` resolves
 * once a boundary waits for them, `steered` once the steer is asked for. `start` starts another session there, and
 * `steer` asks for a steer in a session as its evaluator would.
 */
async function checkingService(options: ServiceOptions = {}): Promise<{
  root: string;
  socketPath: string;
  engine: { socketPath: string };
  checking: Promise<void>;
  steered: Promise<void>;
  finishCheck: () => void;
  start: (sessionId: string) => Promise<void>;
  steer: (sessionId: string, content: string) => Promise<void>;
  openRuntimes: () => number;
  closeIdleSessions: () => Promise<void>;
  deliveriesOf: (content: string) => Promise<number>;
  close: () => Promise<void>;
}> {
  const { root: earlier, ...serviceOptions } = options;
  const root = earlier ?? await mkdtemp(join(tmpdir(), "rerail-unheard-"));
  const socketPath = testEngineEndpoint(root);
  const databasePath = join(root, "rerail.sqlite");
  if (earlier === undefined) startTestRepository(root, databasePath);
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
    ...serviceOptions,
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
  let open = true;
  const close = async (): Promise<void> => {
    if (!open) return;
    open = false;
    check.resolve();
    await service.close();
  };
  onTestFinished(async () => {
    await close();
    await removeFolder(root);
  });
  const start = async (sessionId: string): Promise<void> => {
    await service.ingest({ kind: "session-start", eventId: `start-${sessionId}`, session: { host: "claude-code", sessionId, cwd: root } });
    // The evaluator opens in the background; a boundary before the runtime has taken that in has no edit checks to wait for.
    await openOf(sessionId).promise;
    await new Promise<void>(done => setImmediate(done));
  };
  if (earlier === undefined) await start(SESSION);
  return {
    root, socketPath, engine: { socketPath }, checking: checking.promise, steered: steered.promise, finishCheck: () => check.resolve(), start,
    openRuntimes: () => service.openRuntimes(),
    closeIdleSessions: () => service.closeIdleSessions(),
    deliveriesOf: content => deliveryEntries(join(root, "state"), content),
    close,
    steer: async (sessionId, content) => {
      if (delivery === undefined) throw new Error("no evaluator has opened");
      await delivery.requestSteer(sessionId, "steer", content, { revision: 0, scope: [], background: true });
    },
  };
}
