import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { AdapterDelivery } from "../../src/engine/index.js";
import { ingestEngineEvent, startEngineService } from "../../src/runtime/engine-service.js";
import { testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";


describe("engine service session concurrency", () => {
  it("does not let one opening agent session block another", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const socketPath = join(root, "engine.sock");
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
      const blockedStart = ingestEngineEvent(socketPath, {
        kind: "session-start",
        eventId: "blocked-start",
        session: { host: "claude-code", sessionId: "blocked", cwd: root },
      });
      requests.push(blockedStart);
      await blockedOpened.promise;
      const independentStart = ingestEngineEvent(socketPath, {
        kind: "session-start",
        eventId: "independent-start",
        session: { host: "claude-code", sessionId: "independent", cwd: root },
      });
      requests.push(independentStart);
      await expect(independentStart).resolves.toEqual([]);
      await expect(ingestEngineEvent(socketPath, {
        kind: "user-prompt",
        eventId: "independent-prompt",
        sessionId: "independent",
        text: "continue",
      })).resolves.toEqual([
        expect.objectContaining({ sessionId: "independent", content: "independent-ready" }),
      ]);
      gate.resolve();
      await expect(blockedStart).resolves.toEqual([]);
      await expect(ingestEngineEvent(socketPath, {
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
      await rm(root, { recursive: true, force: true });
    }
  });
});
