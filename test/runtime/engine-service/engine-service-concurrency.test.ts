import { once } from "node:events";
import { mkdtemp } from "node:fs/promises";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { describe, expect, it } from "vitest";
import type { AdapterDelivery } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
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

  it("cuts off the requests it has not answered when it closes, and runs no event still waiting in a session's lane", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
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
        // The session's end waits here, holding its lane.
        close: async () => {
          ending.resolve();
          await gate.promise;
        },
      }),
    });
    await service.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "busy", cwd: root } });
    // An event still running, two waiting behind it in the session's lane, and a hook whose request has not all
    // arrived: what a busy daemon holds when `rerail daemon stop` asks it to close.
    const end = service.ingest({ kind: "session-end", eventId: "end", sessionId: "busy" });
    await ending.promise;
    const waiting = [
      service.ingest({ kind: "session-start", eventId: "restart", session: { host: "claude-code", sessionId: "busy", cwd: root } }),
      service.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: "busy", text: "continue" }),
    ];
    for (const event of waiting) event.catch(() => undefined);
    const partial = connect(socketPath);
    partial.on("error", () => undefined);
    await once(partial, "connect");
    partial.write("POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n\r\n{");
    const cutOff = once(partial, "close").then(() => "cut off");
    let closing: Promise<void> | undefined;
    try {
      // The server takes the connection in before the close begins.
      await sleep(100);
      closing = service.close();
      // The close ends the connection at once, while the running event still waits; before, it waited for the
      // request's 30 s timeout.
      expect(await Promise.race([cutOff, sleep(5_000).then(() => "still open")])).toBe("cut off");
    } finally {
      partial.destroy();
      gate.resolve();
      await closing;
    }
    await expect(end).resolves.toEqual([]);
    for (const event of waiting) await expect(event).rejects.toThrow(/closing/u);
    const store = new SqliteSessionReader(databasePath);
    try {
      const recorded = store.events("busy").map(event => event.eventId);
      expect(recorded).toEqual(expect.arrayContaining(["start", "end"]));
      expect(recorded).not.toContain("restart");
      expect(recorded).not.toContain("prompt");
    } finally {
      store.close();
      await removeFolder(root);
    }
  }, 20_000);
});
