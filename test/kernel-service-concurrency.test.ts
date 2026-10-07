import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { AdapterDelivery } from "../src/adapter-contract.js";
import { ingestKernelEvent, startKernelService } from "../src/kernel-service.js";
import { testEmbedder } from "./hook-fixtures.js";
import { startTestRepository } from "./support/started-repository.js";


describe("resident kernel session concurrency", () => {
  it("does not let one opening agent session block another", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-sessions-"));
    const socketPath = join(root, "kernel.sock");
    startTestRepository(root, join(root, "rerail.sqlite"));
    const blockedOpened = Promise.withResolvers<void>();
    const gate = Promise.withResolvers<void>();
    const service = await startKernelService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: testEmbedder,
      createPolicyEvaluator: ({ delivery }) => ({
        openWorkspace: async sessionId => {
          if (sessionId === "blocked") {
            blockedOpened.resolve();
            await gate.promise;
            await delivery.requestDelivery(sessionId, "steer", "blocked-ready", { revision: 0, scope: [], background: true });
          } else {
            await delivery.requestDelivery(sessionId, "steer", "independent-ready", { revision: 0, scope: [], background: true });
          }
        },
        acceptsPath: () => true,
        evaluateGeneratedEdits: async (_sessionId, batch) => ({
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
      const blockedStart = ingestKernelEvent(socketPath, {
        kind: "session-start",
        eventId: "blocked-start",
        session: { host: "claude-code", sessionId: "blocked", cwd: root },
      });
      requests.push(blockedStart);
      await blockedOpened.promise;
      const independentStart = ingestKernelEvent(socketPath, {
        kind: "session-start",
        eventId: "independent-start",
        session: { host: "claude-code", sessionId: "independent", cwd: root },
      });
      requests.push(independentStart);
      await expect(independentStart).resolves.toEqual([]);
      await expect(ingestKernelEvent(socketPath, {
        kind: "user-prompt",
        eventId: "independent-prompt",
        sessionId: "independent",
        text: "continue",
      })).resolves.toEqual([
        expect.objectContaining({ sessionId: "independent", content: "independent-ready" }),
      ]);
      gate.resolve();
      await expect(blockedStart).resolves.toEqual([]);
      await expect(ingestKernelEvent(socketPath, {
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
