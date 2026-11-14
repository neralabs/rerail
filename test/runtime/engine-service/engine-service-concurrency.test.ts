import { subscribe, unsubscribe } from "node:diagnostics_channel";
import { once } from "node:events";
import { mkdtemp } from "node:fs/promises";
import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { connect, Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import type { AdapterDelivery } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, trackConnections, type EngineService } from "../../../src/runtime/engine-service.js";
import { decodeRerailEventResult } from "../../../src/runtime/event-protocol.js";
import { SqliteSessionReader } from "../../../src/runtime/session-store.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";


describe("engine service session concurrency", () => {
  it("does not let one opening agent session block another", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const socketPath = testEngineEndpoint(root);
    const engine = { socketPath };
    startTestRepository(root, join(root, "rerail.sqlite"));
    const blockedOpened = Promise.withResolvers<void>();
    const gate = Promise.withResolvers<void>();
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: testEmbedder,
      createAuditor: ({ steerDelivery }) => ({
        openWorkspace: async sessionId => {
          if (sessionId === "blocked") {
            blockedOpened.resolve();
            await gate.promise;
            await steerDelivery.requestSteer(sessionId, "steer", "blocked-ready", { revision: 0, scope: [], background: true });
          } else {
            await steerDelivery.requestSteer(sessionId, "steer", "independent-ready", { revision: 0, scope: [], background: true });
          }
        },
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({
          revision: batch.revision,
          disposition: "continue" as const,
          violations: [],
          failures: [],
        }),
        close: async () => undefined,
      }),
    });
    const requests: Array<Promise<readonly AdapterDelivery[]>> = [];
    try {
      const blockedStart = ingestEngineEvent(engine, {
        kind: "session-start",
        eventId: "blocked-start",
        session: { host: "claude-code", sessionId: "blocked", cwd: root },
      });
      requests.push(blockedStart);
      await blockedOpened.promise;
      const independentStart = ingestEngineEvent(engine, {
        kind: "session-start",
        eventId: "independent-start",
        session: { host: "claude-code", sessionId: "independent", cwd: root },
      });
      requests.push(independentStart);
      await expect(independentStart).resolves.toEqual([]);
      await expect(ingestEngineEvent(engine, {
        kind: "user-prompt",
        eventId: "independent-prompt",
        sessionId: "independent",
        text: "continue",
      })).resolves.toEqual([
        expect.objectContaining({ sessionId: "independent", content: "independent-ready" }),
      ]);
      gate.resolve();
      await expect(blockedStart).resolves.toEqual([]);
      await expect(ingestEngineEvent(engine, {
        kind: "user-prompt",
        eventId: "blocked-prompt",
        sessionId: "blocked",
        text: "continue",
      })).resolves.toEqual([
        expect.objectContaining({ sessionId: "blocked", content: "blocked-ready" }),
      ]);
    } finally {
      gate.resolve();
      await Promise.allSettled(requests);
      await service.close();
      await removeFolder(root);
    }
  });

  it("answers the hook of the event it is running when it closes", async () => {
    const held = await holdingService();
    const end = outcome(ingestEngineEvent(held.engine, { kind: "session-end", eventId: "end", sessionId: "busy" }));
    await held.ending;
    // A stop or a restart that arrives while the end runs, which holds on past the close's look 1 s in, the first that
    // can cut a request, so the hook waits through it with its request complete.
    const closing = held.close();
    await sleep(1_500);
    held.release();
    await closing;
    // The block or steer in that answer reaches the agent only if the hook gets it.
    expect(await end).toBe("answered");
    expect(recorded(held.databasePath)).toContain("end");
  }, 20_000);

  it("runs, records and answers the events waiting behind the running one when it closes", async () => {
    const held = await holdingService();
    const end = outcome(ingestEngineEvent(held.engine, { kind: "session-end", eventId: "end", sessionId: "busy" }));
    await held.ending;
    const arrived = nextRequest();
    const restart = outcome(ingestEngineEvent(held.engine, { kind: "session-start", eventId: "restart", session: busySession(held.root) }));
    // The server has the waiting hook's whole request before the close begins.
    const request = await arrived;
    await vi.waitFor(() => expect(request.complete).toBe(true), { timeout: 5_000, interval: 10 });
    const closing = held.close();
    // The end holds on past the close's look 1 s in, the first that can cut a request, so both hooks wait through it
    // with their requests complete.
    await sleep(1_500);
    held.release();
    await closing;
    expect({ end: await end, restart: await restart }).toEqual({ end: "answered", restart: "answered" });
    expect(recorded(held.databasePath)).toEqual(expect.arrayContaining(["start", "end", "restart"]));
  }, 20_000);

  it("cuts off a request that stalls before it has all arrived when it closes, and still runs the events waiting", async () => {
    const held = await holdingService();
    // An event still running, two waiting behind it in the session's lane, and a request that sent part of its body
    // and then nothing: what a busy daemon can hold when a stop asks it to close.
    const end = outcome(held.service.ingest({ kind: "session-end", eventId: "end", sessionId: "busy" }));
    await held.ending;
    const waiting = [
      outcome(held.service.ingest({ kind: "session-start", eventId: "restart", session: busySession(held.root) })),
      outcome(held.service.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: "busy", text: "continue" })),
    ];
    const accepted = nextConnection();
    const partial = await connected(held.engine.socketPath);
    const sent = "POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n\r\n{";
    partial.write(sent);
    const cutOff = once(partial, "close").then(() => performance.now());
    // The server has read all that was sent before the close begins.
    await serverRead(await accepted, Buffer.byteLength(sent));
    const began = performance.now();
    const closing = held.close();
    // The close cuts the stalled request off 1 s in, while the running event still waits. Before, it waited for the
    // request with no limit, since `server.close()` stops the check behind the server's request timeout: past the
    // 10 s `rerail daemon stop` gives before its kill.
    expect(await whenCut(cutOff, began)).toBe("cut off 1 s in");
    held.release();
    await closing;
    expect([await end, ...await Promise.all(waiting)]).toEqual(["answered", "answered", "answered"]);
    expect(recorded(held.databasePath)).toEqual(expect.arrayContaining(["start", "end", "restart", "prompt"]));
  }, 20_000);

  it("cuts off a connection that has sent no request when it closes", async () => {
    const held = await holdingService();
    // A hook that connected and then wrote nothing.
    const accepted = nextConnection();
    const silent = await connected(held.engine.socketPath);
    const cutOff = new Promise<number>(done => silent.once("close", () => done(performance.now())));
    // The server has taken the connection in before the close begins.
    await accepted;
    const began = performance.now();
    const closing = held.close();
    expect(await whenCut(cutOff, began)).toBe("cut off 1 s in");
    held.release();
    await closing;
  }, 20_000);

  it("cuts off a next request that stalls in its headers on a connection its client keeps open when it closes", async () => {
    const held = await holdingService();
    // A client that keeps its connection open after an answer (HTTP/1.1's default), then sends part of its next
    // request's headers and nothing more.
    const kept = await keptConnection(held.engine.socketPath);
    const answered = nextAnswerWritten();
    await kept.send(keptRequest({ kind: "user-prompt", eventId: "first", sessionId: "busy", text: "continue" }));
    await answered;
    await kept.send("POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-ty");
    const began = performance.now();
    const closing = held.close();
    // Before, the close left it to Node's keep-alive timeout, which ends it about 6 s after its last byte: past the
    // 10 s a stop gives when that byte comes 4 s or more into the close.
    expect(await whenCut(kept.closed, began)).toBe("cut off 1 s in");
    held.release();
    await closing;
  }, 20_000);

  it("answers a whole request sent seconds after an answer on a connection its client keeps open when it closes", async () => {
    const held = await holdingService();
    const kept = await keptConnection(held.engine.socketPath);
    const answered = nextAnswerWritten();
    await kept.send(keptRequest({ kind: "session-end", eventId: "end", sessionId: "busy" }));
    await held.ending;
    const closing = held.close();
    held.release();
    await answered;
    // The client sends its next request whole once two looks for stalled requests have found nothing sent since
    // the answer, as Node's keep-alive timeout lets it.
    await sleep(2_500);
    await kept.send(keptRequest({ kind: "session-start", eventId: "restart", session: busySession(held.root) }));
    await vi.waitFor(() => expect(kept.answers()).toEqual(["200", "200"]), { timeout: 5_000, interval: 20 });
    // The connection stays open after its answers, and the close waits for it.
    kept.destroy();
    await closing;
    expect(recorded(held.databasePath)).toEqual(expect.arrayContaining(["end", "restart"]));
  }, 20_000);

  it("answers a whole request sent seconds after the body of a request answered on its headers alone, on a connection its client keeps open, when it closes", async () => {
    const held = await holdingService();
    // The service answers a wrong path on its headers alone, before its body has come. The client keeps the
    // connection open and sends that body once the close has begun.
    const kept = await keptConnection(held.engine.socketPath);
    const answered = nextAnswerWritten();
    const body = "{}";
    await kept.send(`POST /nope HTTP/1.1\r\nHost: localhost\r\ncontent-length: ${body.length}\r\n\r\n`);
    await answered;
    const began = performance.now();
    const closing = held.close();
    await sleep(300);
    await kept.send(body);
    // Two looks for stalled requests find nothing sent since that body, then the client sends its next request
    // whole, as Node's keep-alive timeout lets it: the rest of an answered request is not a next request.
    await sleep(Math.max(0, began + 3_000 - performance.now()));
    expect(await whenCut(kept.closed, began, 0)).toBe("still open");
    await kept.send(keptRequest({ kind: "user-prompt", eventId: "late", sessionId: "busy", text: "continue" }));
    await vi.waitFor(() => expect(kept.answers()).toEqual(["404", "200"]), { timeout: 5_000, interval: 20 });
    // The connection stays open after its answers, and the close waits for it.
    kept.destroy();
    held.release();
    await closing;
    expect(recorded(held.databasePath)).toContain("late");
  }, 20_000);

  it("cuts off a next request that came in before the answer ahead of it and stalls in its body, on a connection its client keeps open, when it closes", async () => {
    const held = await holdingService();
    // A client that sends its next request before it has the answer to the one ahead of it (pipelining): that next
    // request's headers arrive whole while the first one's event runs, then part of its body and nothing more.
    const kept = await keptConnection(held.engine.socketPath);
    const answered = nextAnswerWritten();
    await kept.send(keptRequest({ kind: "session-end", eventId: "end", sessionId: "busy" }));
    await held.ending;
    const next = nextRequest();
    await kept.send("POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n\r\n{");
    await next;
    held.release();
    await answered;
    const began = performance.now();
    const closing = held.close();
    // The answer ahead of it leaves the connection to that next request, which the close cuts off as any request
    // that stalls before it has all arrived.
    expect(await whenCut(kept.closed, began)).toBe("cut off 1 s in");
    await closing;
    expect(kept.answers()).toEqual(["200"]);
  }, 20_000);

  it("lets an answer larger than a socket holds finish arriving when it closes", async () => {
    const held = await holdingService();
    const end = outcome(ingestEngineEvent(held.engine, { kind: "session-end", eventId: "end", sessionId: "busy" }));
    await held.ending;
    // A hook waiting behind the end whose answer repeats its event's id, and so is 600 KB: more than a Windows named
    // pipe (64 KB) or a Unix socket holds, so it is still being written while the hook reads none of it.
    const eventId = `restart-${"x".repeat(600_000)}`;
    const big = await handHook(held.engine.socketPath, JSON.stringify({ event: { kind: "session-start", eventId, session: busySession(held.root) } }));
    const closing = held.close();
    held.release();
    // Once the answer begins to arrive the server has ended it; the close looks for stalled requests at least once
    // more before the hook reads the rest.
    await vi.waitFor(() => expect(big.waiting()).toBeGreaterThan(0), { timeout: 10_000, interval: 20 });
    await sleep(1_200);
    const answer = wholeAnswer(await big.answer(), eventId);
    await closing;
    expect({ end: await end, big: answer }).toEqual({ end: "answered", big: "whole" });
  }, 30_000);

  it("keeps a request whose rest came in time while its event loop was busy past a look for stalled requests", async () => {
    const held = await holdingService();
    const body = JSON.stringify({ event: { kind: "user-prompt", eventId: "late", sessionId: "busy", text: "continue" } });
    const late = await handHook(held.engine.socketPath, body, Math.floor(body.length / 2));
    const began = performance.now();
    const closing = held.close();
    // 0.7 s in, a check phase writes the rest and holds the loop to 1.3 s, past the look due at 1 s: no poll phase
    // has read the rest when that look's timer fires, as when a daemon's loop is held while a hook's bytes arrive.
    await sleep(700);
    const wroteAt = await busyFromCheckPhase(() => late.finish(), began + 1_300) - began;
    held.release();
    const answer = wholeAnswer(await late.answer(), "late");
    await closing;
    expect({ wroteBeforeTheLook: wroteAt < 1_000, late: answer }).toEqual({ wroteBeforeTheLook: true, late: "whole" });
    expect(recorded(held.databasePath)).toContain("late");
  }, 20_000);

  it("keeps only open connections in the list the close reads", async () => {
    const socketPath = testEngineEndpoint(await testFolder());
    // It answers once the whole request has arrived, as the engine service does. Answered on its headers alone, the
    // connection left the list on Windows before the hook sent its body.
    const server = createServer((incoming, response) => {
      incoming.resume();
      incoming.once("end", () => response.end("{}"));
    });
    onTestFinished(async () => {
      server.closeAllConnections();
      await new Promise<void>(done => server.close(() => done()));
    });
    const connections = trackConnections(server);
    const size = async (expected: number, when: string): Promise<void> => {
      await vi.waitFor(() => expect(connections.size, when).toBe(expected), { timeout: 5_000, interval: 20 });
    };
    await new Promise<void>(done => server.listen(socketPath, done));
    // A hook that gets its answer, then one that connects and goes away without a request: a daemon serves
    // thousands of each, and the list must not keep their sockets. The first holds its body back until the list
    // shows it.
    const answered = await handHook(socketPath, "{}", 0);
    await size(1, "a hook whose body has not come");
    answered.finish();
    expect(await answered.answer()).toContain("HTTP/1.1 200");
    await size(0, "after that hook's answer");
    const silent = await connected(socketPath);
    await size(1, "a connection with no request");
    silent.destroy();
    await size(0, "after that connection closed");
  }, 20_000);
});

// Each helper below that opens something (a folder, an engine service, a client connection, a wait on a diagnostics
// channel) closes it when the test ends, whether it passed, failed or timed out (`onTestFinished`), so a step that
// fails, or a test that never returns, leaves nothing open for the tests after it. Vitest runs those clean-ups in the
// reverse of the order they were registered: the connections and waits first, then the service, then its folder.

/** A temporary folder for one test, removed once the test has ended and what it opened in the folder has closed. */
async function testFolder(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  return root;
}

/**
 * A client connection to `socketPath`, once it has connected. Its errors are left to the test, which reads them from
 * its close: the close under test cuts some connections off. It is destroyed when the test ends.
 */
async function connected(socketPath: string): Promise<Socket> {
  const socket = connect(socketPath);
  socket.on("error", () => undefined);
  onTestFinished(() => {
    socket.destroy();
  });
  await once(socket, "connect");
  return socket;
}

/** A hook's request written by hand, so a test chooses when the rest of its body goes and when it reads the answer. */
interface HandHook {
  /** Writes the rest of the body. */
  finish(): void;
  /** How many bytes of the answer have arrived and wait unread. */
  waiting(): number;
  /** Reads the answer until the connection closes. */
  answer(): Promise<string>;
}

/**
 * Connects to the engine at `socketPath` and writes a request for `body`, its first `sent` characters only, then
 * reads nothing until `answer`: the socket stops reading at its high-water mark, so the server's write of a larger
 * answer stays unfinished. Resolves once the server has read all it wrote.
 */
async function handHook(socketPath: string, body: string, sent = body.length): Promise<HandHook> {
  const accepted = nextConnection();
  const socket = await connected(socketPath);
  const closed = new Promise<void>(done => socket.once("close", () => done()));
  const head = `POST /events HTTP/1.1\r\nHost: localhost\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body.slice(0, sent)}`;
  socket.write(head);
  await serverRead(await accepted, Buffer.byteLength(head));
  return {
    finish: () => {
      socket.write(body.slice(sent));
    },
    waiting: () => socket.readableLength,
    answer: async () => {
      const chunks: Buffer[] = [];
      socket.on("data", (chunk: Buffer) => chunks.push(chunk));
      await closed;
      return Buffer.concat(chunks).toString("utf8");
    },
  };
}

/** A connection its client keeps open across requests, written by hand. */
interface KeptConnection {
  /** Writes `text` and resolves once the server has read all that was written on the connection. */
  send(text: string): Promise<void>;
  /** The status codes of the answers that have arrived. */
  answers(): string[];
  /** When the connection closed, a `performance.now()` time. */
  closed: Promise<number>;
  destroy(): void;
}

async function keptConnection(socketPath: string): Promise<KeptConnection> {
  const accepted = nextConnection();
  const socket = await connected(socketPath);
  let received = "";
  socket.on("data", (chunk: Buffer) => (received += chunk.toString("utf8")));
  const closed = new Promise<number>(done => socket.once("close", () => done(performance.now())));
  const server = await accepted;
  let written = 0;
  return {
    send: async text => {
      written += Buffer.byteLength(text);
      socket.write(text);
      await serverRead(server, written);
    },
    answers: () => [...received.matchAll(/HTTP\/1\.1 (\d+)/g)].map(match => match[1] ?? ""),
    closed,
    destroy: () => socket.destroy(),
  };
}

/** A hook's whole request for `event`, with no `connection: close`, so its client keeps the connection open. */
function keptRequest(event: unknown): string {
  const body = JSON.stringify({ event });
  return `POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body}`;
}

/**
 * Resolves once a server has written its answer to the next request it reads the headers of. It listens before the
 * server's handler runs, so an answer the handler writes on the headers alone is not missed.
 */
function nextAnswerWritten(): Promise<void> {
  const answer = nextMessage("http.server.request.start", message => {
    const response = field(message, "response", ServerResponse);
    return response === undefined ? undefined : { written: once(response, "finish") };
  });
  return handled(answer.then(async ({ written }) => {
    await written;
  }));
}

/**
 * When a connection the close cut off closed (`closed`, a `performance.now()` time), counted from the close's start
 * (`began`): `cut off 1 s in` at the look 1 s in, which finds it silent since the look the close takes as it begins;
 * how long it took when a later look, or something other than a look, closed it; or `still open` after `wait` ms.
 */
async function whenCut(closed: Promise<number>, began: number, wait = 5_000): Promise<string> {
  const at = await Promise.race([closed, sleep(wait).then(() => undefined)]);
  if (at === undefined) return "still open";
  const after = at - began;
  return after >= 900 && after < 1_950 ? "cut off 1 s in" : `cut off ${Math.round(after)} ms in`;
}

/** How long a test waits for a message on a diagnostics channel: as long as `serverRead` waits for bytes. */
const MESSAGE_WAIT_MS = 5_000;

/**
 * Resolves with the next message Node publishes on its diagnostics channel `name` that `pick` takes, from any server in
 * this process. A test starts the close once the engine's server has what it sent, which this shows without waiting a
 * set time that a slow machine can outlast. The channel is the process's, shared with every later test in the file, so
 * the wait leaves it as it takes its message, as `MESSAGE_WAIT_MS` pass with none (it then rejects, naming the
 * channel), or as the test ends, whichever comes first. It leaves in that same call, so a message published later in
 * the same turn, such as a second request the server parsed from the same chunk, never reaches `pick`. A test that ends
 * has stopped awaiting the wait, so its end leaves it pending rather than reject it.
 */
function nextMessage<T>(name: string, pick: (message: unknown) => T | undefined): Promise<T> {
  const taken = Promise.withResolvers<T>();
  const take = (message: unknown): void => {
    const picked = pick(message);
    if (picked === undefined) return;
    leave();
    taken.resolve(picked);
  };
  const limit = setTimeout(() => {
    leave();
    taken.reject(new Error(`no message came on the diagnostics channel ${name} in ${MESSAGE_WAIT_MS} ms`));
  }, MESSAGE_WAIT_MS);
  const leave = (): void => {
    clearTimeout(limit);
    unsubscribe(name, take);
  };
  subscribe(name, take);
  onTestFinished(leave);
  return handled(taken.promise);
}

/**
 * `promise`, with a handler that keeps it from counting as an unhandled rejection: a wait can give up while its test
 * awaits something else, and Vitest fails a run on an unhandled rejection. A test that awaits it still gets the error.
 */
function handled<T>(promise: Promise<T>): Promise<T> {
  promise.catch(() => undefined);
  return promise;
}

/** The server's side of the next connection a server takes in. */
function nextConnection(): Promise<Socket> {
  return nextMessage("net.server.socket", message => field(message, "socket", Socket));
}

/** The next request a server has read the headers of. */
function nextRequest(): Promise<IncomingMessage> {
  return nextMessage("http.server.request.start", message => field(message, "request", IncomingMessage));
}

/** `message[key]` when it is an instance of `type`. */
function field<T>(message: unknown, key: string, type: abstract new (...args: never[]) => T): T | undefined {
  if (typeof message !== "object" || message === null) return undefined;
  const value: unknown = Reflect.get(message, key);
  return value instanceof type ? value : undefined;
}

/** Resolves once `server`, a server's side of a connection, has read `bytes` bytes. */
async function serverRead(server: Socket, bytes: number): Promise<void> {
  await vi.waitFor(() => expect(server.bytesRead).toBe(bytes), { timeout: 5_000, interval: 10 });
}

/** `whole` when `answer` is a 200 whose body carries the event `eventId`; otherwise its status line and its length. */
function wholeAnswer(answer: string, eventId: string): string {
  const [head = "", body = ""] = answer.split("\r\n\r\n");
  try {
    if (head.startsWith("HTTP/1.1 200") && decodeRerailEventResult(body).eventId === eventId) return "whole";
  } catch {
    // A body cut short is not JSON.
  }
  return `${head.split("\r\n")[0] || "no answer"}, ${answer.length} characters`;
}

/**
 * Runs `first` in the next check phase, then keeps the event loop busy until `until` (a `performance.now()` time), as a
 * synchronous call does in a daemon. Resolves with the time `first` ran.
 */
async function busyFromCheckPhase(first: () => void, until: number): Promise<number> {
  return await new Promise<number>(done => setImmediate(() => {
    const at = performance.now();
    first();
    while (performance.now() < until) {
      // The loop stays busy.
    }
    done(at);
  }));
}

function busySession(root: string): { host: "claude-code"; sessionId: string; cwd: string } {
  return { host: "claude-code", sessionId: "busy", cwd: root };
}

/**
 * An engine service in a folder of its own (`root`) with the session `busy` started, whose end waits in the auditor's
 * close until `release`, holding that session's lane: what a busy daemon is running when a stop or a restart closes
 * it. `ending` resolves once the end runs. `close` closes the service once, however often it is called, and the test's
 * end calls it after `release`, then removes the folder: closing the service closes the session, whose auditor's
 * close waits for `release` too.
 */
async function holdingService(): Promise<{
  service: EngineService;
  engine: { socketPath: string };
  root: string;
  databasePath: string;
  ending: Promise<void>;
  release: () => void;
  close: () => Promise<void>;
}> {
  const root = await testFolder();
  const socketPath = testEngineEndpoint(root);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const ending = Promise.withResolvers<void>();
  const gate = Promise.withResolvers<void>();
  const opened = Promise.withResolvers<void>();
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    createAuditor: () => ({
      openWorkspace: async () => {
        opened.resolve();
      },
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
      close: async () => {
        ending.resolve();
        await gate.promise;
      },
    }),
  });
  let closing: Promise<void> | undefined;
  const close = (): Promise<void> => (closing ??= handled(service.close()));
  onTestFinished(async () => {
    gate.resolve();
    await close();
  });
  await service.ingest({ kind: "session-start", eventId: "start", session: busySession(root) });
  // The session's evaluator opens in the background, and an end that arrives before it has opened does not wait for
  // it: the end would finish at once and `ending` would resolve only later, when the late evaluator is closed. So the
  // service is handed over once the evaluator has opened and the runtime has taken that in.
  await opened.promise;
  await new Promise<void>(done => setImmediate(done));
  return { service, engine: { socketPath }, root, databasePath, ending: ending.promise, release: () => gate.resolve(), close };
}

/** The ids of the events the store at `databasePath` recorded for the session `busy`. */
function recorded(databasePath: string): string[] {
  const store = new SqliteSessionReader(databasePath);
  try {
    return store.events("busy").map(event => event.eventId);
  } finally {
    store.close();
  }
}

/** How a hook's request ended: `answered`, or the error it got instead of an answer. */
async function outcome(request: Promise<readonly AdapterDelivery[]>): Promise<string> {
  return await request.then(() => "answered", (error: unknown) => `error: ${error instanceof Error ? error.message : String(error)}`);
}
