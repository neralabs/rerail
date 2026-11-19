import { mkdtemp } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFinished } from "vitest";
import type { AdapterDelivery, AdapterEvent } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
import { decodeRerailEventResult } from "../../../src/runtime/event-protocol.js";
import { testEmbedder } from "../../support/hook-events.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

const SESSION = "gone";
const STEER = "Rerail found 1 problem in your last edit: a.py calls eval.";

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
});

function turnEnd(eventId: string, cwd: string): AdapterEvent {
  return { kind: "turn-end", eventId, sessionId: SESSION, session: { host: "claude-code", sessionId: SESSION, cwd }, text: "" };
}

function prompt(eventId: string): AdapterEvent {
  return { kind: "user-prompt", eventId, sessionId: SESSION, text: "next" };
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
 * once a boundary waits for them, `steered` once the steer is asked for.
 */
async function checkingService(): Promise<{
  root: string;
  socketPath: string;
  engine: { socketPath: string };
  checking: Promise<void>;
  steered: Promise<void>;
  finishCheck: () => void;
}> {
  const root = await mkdtemp(join(tmpdir(), "rerail-unheard-"));
  const socketPath = testEngineEndpoint(root);
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const opened = Promise.withResolvers<void>();
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
      openWorkspace: async () => {
        opened.resolve();
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
  await service.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: SESSION, cwd: root } });
  // The evaluator opens in the background; a boundary before the runtime has taken that in has no edit checks to wait for.
  await opened.promise;
  await new Promise<void>(done => setImmediate(done));
  return { root, socketPath, engine: { socketPath }, checking: checking.promise, steered: steered.promise, finishCheck: () => check.resolve() };
}
