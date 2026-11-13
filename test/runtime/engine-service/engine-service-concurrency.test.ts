import { once } from "node:events";
import { mkdtemp } from "node:fs/promises";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
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
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const held = await holdingService(root);
    try {
      const end = outcome(ingestEngineEvent(held.engine, { kind: "session-end", eventId: "end", sessionId: "busy" }));
      await held.ending;
      // A stop or a restart that arrives while the end runs, which holds on past the close's first look for stalled
      // requests (1 s in), so the hook waits through it with its request complete.
      const closing = held.service.close();
      await sleep(1_500);
      held.release();
      await closing;
      // The block or steer in that answer reaches the agent only if the hook gets it.
      expect(await end).toBe("answered");
      expect(recorded(held.databasePath)).toContain("end");
    } finally {
      held.release();
      await removeFolder(root);
    }
  }, 20_000);

  it("runs, records and answers the events waiting behind the running one when it closes", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const held = await holdingService(root);
    try {
      const end = outcome(ingestEngineEvent(held.engine, { kind: "session-end", eventId: "end", sessionId: "busy" }));
      await held.ending;
      const restart = outcome(ingestEngineEvent(held.engine, { kind: "session-start", eventId: "restart", session: busySession(root) }));
      // The server takes the waiting hook's request in before the close begins.
      await sleep(250);
      const closing = held.service.close();
      // The end holds on past the close's first look for stalled requests (1 s in), so both hooks wait through it with
      // their requests complete.
      await sleep(1_500);
      held.release();
      await closing;
      expect({ end: await end, restart: await restart }).toEqual({ end: "answered", restart: "answered" });
      expect(recorded(held.databasePath)).toEqual(expect.arrayContaining(["start", "end", "restart"]));
    } finally {
      held.release();
      await removeFolder(root);
    }
  }, 20_000);

  it("cuts off a request that stalls before it has all arrived when it closes, and still runs the events waiting", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const held = await holdingService(root);
    try {
      // An event still running, two waiting behind it in the session's lane, and a request that sent part of its body
      // and then nothing: what a busy daemon can hold when a stop asks it to close.
      const end = outcome(held.service.ingest({ kind: "session-end", eventId: "end", sessionId: "busy" }));
      await held.ending;
      const waiting = [
        outcome(held.service.ingest({ kind: "session-start", eventId: "restart", session: busySession(root) })),
        outcome(held.service.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: "busy", text: "continue" })),
      ];
      const partial = connect(held.engine.socketPath);
      partial.on("error", () => undefined);
      await once(partial, "connect");
      partial.write("POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n\r\n{");
      const cutOff = once(partial, "close").then(() => "cut off");
      // The server takes the connection in before the close begins.
      await sleep(250);
      const closing = held.service.close();
      try {
        // The close cuts the stalled request off while the running event still waits; before, it waited out the
        // request's 30 s timeout, past the 10 s `rerail daemon stop` gives before its kill.
        expect(await Promise.race([cutOff, sleep(5_000).then(() => "still open")])).toBe("cut off");
      } finally {
        partial.destroy();
        held.release();
        await closing;
      }
      expect([await end, ...await Promise.all(waiting)]).toEqual(["answered", "answered", "answered"]);
      expect(recorded(held.databasePath)).toEqual(expect.arrayContaining(["start", "end", "restart", "prompt"]));
    } finally {
      held.release();
      await removeFolder(root);
    }
  }, 20_000);

  it("cuts off a connection that has sent no request when it closes", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const held = await holdingService(root);
    try {
      // A hook that connected and then wrote nothing.
      const silent = connect(held.engine.socketPath);
      silent.on("error", () => undefined);
      const cutOff = new Promise<string>(done => silent.once("close", () => done("cut off")));
      await once(silent, "connect");
      // The server takes the connection in before the close begins.
      await sleep(250);
      const closing = held.service.close();
      try {
        expect(await Promise.race([cutOff, sleep(5_000).then(() => "still open")])).toBe("cut off");
      } finally {
        silent.destroy();
        held.release();
        await closing;
      }
    } finally {
      held.release();
      await removeFolder(root);
    }
  }, 20_000);

  it("lets an answer larger than a socket holds finish arriving when it closes", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const held = await holdingService(root);
    try {
      const end = outcome(ingestEngineEvent(held.engine, { kind: "session-end", eventId: "end", sessionId: "busy" }));
      await held.ending;
      // A hook waiting behind the end whose answer repeats its event's id, and so is 600 KB: more than a Windows named
      // pipe (64 KB) or a Unix socket holds, so it is still being written while the hook reads none of it.
      const eventId = `restart-${"x".repeat(600_000)}`;
      const big = await handHook(held.engine.socketPath, JSON.stringify({ event: { kind: "session-start", eventId, session: busySession(root) } }));
      // The server takes the request in before the close begins.
      await sleep(250);
      const closing = held.service.close();
      held.release();
      // Once the answer begins to arrive the server has ended it; the close looks for stalled requests at least once
      // more before the hook reads the rest.
      await vi.waitFor(() => expect(big.waiting()).toBeGreaterThan(0), { timeout: 10_000, interval: 20 });
      await sleep(1_200);
      const answer = wholeAnswer(await big.answer(), eventId);
      await closing;
      expect({ end: await end, big: answer }).toEqual({ end: "answered", big: "whole" });
    } finally {
      held.release();
      await removeFolder(root);
    }
  }, 30_000);

  it("keeps a request whose rest came in time while its event loop was busy past a look for stalled requests", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const held = await holdingService(root);
    try {
      const body = JSON.stringify({ event: { kind: "user-prompt", eventId: "late", sessionId: "busy", text: "continue" } });
      const late = await handHook(held.engine.socketPath, body, Math.floor(body.length / 2));
      // The server takes the first half in before the close begins.
      await sleep(250);
      const began = performance.now();
      const closing = held.service.close();
      // 0.7 s in, a check phase writes the rest and holds the loop to 1.3 s, past the look due at 1 s: no poll phase
      // has read the rest when that look's timer fires, as when a daemon's loop is held while a hook's bytes arrive.
      await sleep(700);
      const wroteAt = await busyFromCheckPhase(() => late.finish(), began + 1_300) - began;
      held.release();
      const answer = wholeAnswer(await late.answer(), "late");
      await closing;
      expect({ wroteBeforeTheLook: wroteAt < 1_000, late: answer }).toEqual({ wroteBeforeTheLook: true, late: "whole" });
      expect(recorded(held.databasePath)).toContain("late");
    } finally {
      held.release();
      await removeFolder(root);
    }
  }, 20_000);
});

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
 * answer stays unfinished.
 */
async function handHook(socketPath: string, body: string, sent = body.length): Promise<HandHook> {
  const socket = connect(socketPath);
  socket.on("error", () => undefined);
  const closed = new Promise<void>(done => socket.once("close", () => done()));
  await once(socket, "connect");
  socket.write(`POST /events HTTP/1.1\r\nHost: localhost\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body.slice(0, sent)}`);
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
 * An engine service with the session `busy` started, whose end waits in the auditor's close until `release`, holding
 * that session's lane: what a busy daemon is running when a stop or a restart closes it. `ending` resolves once the end
 * runs.
 */
async function holdingService(root: string): Promise<{
  service: EngineService;
  engine: { socketPath: string };
  databasePath: string;
  ending: Promise<void>;
  release: () => void;
}> {
  const socketPath = testEngineEndpoint(root);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const ending = Promise.withResolvers<void>();
  const gate = Promise.withResolvers<void>();
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    createAuditor: () => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
      close: async () => {
        ending.resolve();
        await gate.promise;
      },
    }),
  });
  await service.ingest({ kind: "session-start", eventId: "start", session: busySession(root) });
  return { service, engine: { socketPath }, databasePath, ending: ending.promise, release: () => gate.resolve() };
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
