import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { AdapterDelivery } from "../src/adapter-contract.js";
import type { PolicySteeringDelivery } from "../src/executable-policy.js";
import { RerailRuntime } from "../src/runtime.js";
import { SqliteTranscriptStore } from "../src/transcript-store.js";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("Rerail session startup", () => {
  it("leaves failed workspace attachment inactive and retries descriptor-bearing events without stale router state", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-startup-"));
    directories.push(root);
    const databasePath = join(root, ".rerail", "rerail.sqlite");
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: join(root, "workspace"),
      repositoryContext: { repositoryId: "startup-retry", workspaceRoot: join(root, "workspace") },
      storageRoot: join(root, ".rerail", "sessions"),
      databasePath,
      embedder: {
        fingerprint: () => ({
          modelId: "test", revision: "1", dimensions: 2,
          weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1,
        }),
        embed: async texts => texts.map(() => new Float32Array([1, 0])),
      },
    });
    const transcripts = new SqliteTranscriptStore(databasePath);
    const session = { host: "omp" as const, sessionId: "retry-start", cwd: join(root, "workspace") };
    const prompt = { kind: "user-prompt" as const, eventId: "retry-prompt", sessionId: session.sessionId, session, text: "Recovered prompt" };

    try {
      await expect(runtime.ingest({ kind: "session-start", eventId: "start", session })).rejects.toMatchObject({ code: "ENOENT" });
      expect(transcripts.session(session.sessionId)).toBeUndefined();
      // A hook may retry on its next event rather than sending another session-start.
      await expect(runtime.ingest(prompt)).rejects.toMatchObject({ code: "ENOENT" });
      expect(transcripts.session(session.sessionId)).toBeUndefined();

      await mkdir(session.cwd);
      await runtime.ingest(prompt);
      await runtime.ingest({ kind: "user-prompt", eventId: "next-prompt", sessionId: session.sessionId, text: "Still attached" });
      // Publication records `hud-tracking-start` and one `rule-coverage` event beside the two prompts.
      expect(transcripts.session(session.sessionId)).toMatchObject({ eventCount: 4, endedAt: null });
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });
      expect(transcripts.session(session.sessionId)).toMatchObject({ eventCount: 5, endedAt: expect.any(Number) });
    } finally {
      await runtime.dispose();
      transcripts.close();
    }
  });

  it("delivers observations queued during successful attachment but discards failed and retired attachments", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-observations-"));
    directories.push(root);
    const callbacks: PolicySteeringDelivery[] = [];
    const deliveries: AdapterDelivery[] = [];
    const delivered = Promise.withResolvers<void>();
    const failure = new Error("Workspace attachment failed");
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: {
        fingerprint: () => ({
          modelId: "test", revision: "1", dimensions: 2,
          weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1,
        }),
        embed: async texts => texts.map(() => new Float32Array([1, 0])),
      },
      deliverBackground: delivery => {
        deliveries.push(delivery);
        delivered.resolve();
      },
      createPolicyEvaluator: ({ delivery }) => {
        callbacks.push(delivery);
        const attempt = callbacks.length;
        return {
          openWorkspace: async sessionId => {
            await delivery.requestDelivery(sessionId, "steer", `Observation ${attempt}`, {
              revision: 0, scope: [], background: true,
            });
            if (attempt === 1) throw failure;
          },
          acceptsPath: () => true,
          evaluateGeneratedEdits: async (_sessionId, batch) => ({
            revision: batch.revision, disposition: "continue", findings: [], failures: [],
          }),
          close: async () => undefined,
        };
      },
    });
    const session = { host: "omp" as const, sessionId: "retry-observation", cwd: root };
    try {
      await expect(runtime.ingest({ kind: "session-start", eventId: "failed", session })).rejects.toBe(failure);
      expect(deliveries).toEqual([]);
      await runtime.ingest({ kind: "session-start", eventId: "retry", session });
      await delivered.promise;
      expect(await callbacks[0]!.requestDelivery(session.sessionId, "steer", "Retired observation", {
        revision: 0, scope: [], background: true,
      })).toBe("discarded");
      expect(deliveries).toEqual([
        expect.objectContaining({ sessionId: session.sessionId, content: "Observation 2" }),
      ]);
    } finally {
      await runtime.dispose();
    }
  });
});
