import { createHash } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdapterDelivery } from "../src/adapter-contract.js";
import type { RuleSteerDelivery } from "../src/code-rule.js";
import { RerailRuntime } from "../src/runtime.js";
import { SqliteSessionStore } from "../src/session-store.js";

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
    const sessionStore = new SqliteSessionStore(databasePath);
    const session = { host: "omp" as const, sessionId: "retry-start", cwd: join(root, "workspace") };
    const prompt = { kind: "user-prompt" as const, eventId: "retry-prompt", sessionId: session.sessionId, session, text: "Recovered prompt" };

    try {
      await expect(runtime.ingest({ kind: "session-start", eventId: "start", session })).rejects.toMatchObject({ code: "ENOENT" });
      expect(sessionStore.session(session.sessionId)).toBeUndefined();
      // A hook may retry on its next event rather than sending another session-start.
      await expect(runtime.ingest(prompt)).rejects.toMatchObject({ code: "ENOENT" });
      expect(sessionStore.session(session.sessionId)).toBeUndefined();

      await mkdir(session.cwd);
      await runtime.ingest(prompt);
      await runtime.ingest({ kind: "user-prompt", eventId: "next-prompt", sessionId: session.sessionId, text: "Still attached" });
      // Publication records `hud-tracking-start` and one `rule-coverage` event beside the two prompts.
      expect(sessionStore.session(session.sessionId)).toMatchObject({ eventCount: 4, endedAt: null });
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });
      expect(sessionStore.session(session.sessionId)).toMatchObject({ eventCount: 5, endedAt: expect.any(Number) });
    } finally {
      await runtime.dispose();
      sessionStore.close();
    }
  });

  it("delivers observations queued during successful attachment but discards failed and retired attachments", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-observations-"));
    directories.push(root);
    const callbacks: RuleSteerDelivery[] = [];
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
      createAuditor: ({ steerDelivery }) => {
        callbacks.push(steerDelivery);
        const attempt = callbacks.length;
        return {
          openWorkspace: async sessionId => {
            await steerDelivery.requestSteer(sessionId, "steer", `Observation ${attempt}`, {
              revision: 0, scope: [], background: true,
            });
            if (attempt === 1) throw failure;
          },
          acceptsPath: () => true,
          auditGeneratedEdits: async (_sessionId, batch) => ({
            revision: batch.revision, disposition: "continue", violations: [], failures: [],
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
      expect(await callbacks[0]!.requestSteer(session.sessionId, "steer", "Retired observation", {
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

describe("Rerail session restore", () => {
  it("skips a ledger rule mutation that no longer parses and replays the rest", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-restore-"));
    directories.push(root);
    const storageRoot = join(root, "sessions");
    const session = { host: "claude-code" as const, sessionId: "restored", cwd: root };
    const rule = {
      id: "restored-rule", mode: "block", reference: "Guest checkout remains available.",
      match: { operator: "gte", threshold: 0.99 },
      sample: { source: "assistant-text", boundary: "message" },
      action: { content: "Check checkout." },
    };
    const { mode: _mode, ...modeless } = rule;
    const ledgerPath = join(storageRoot, createHash("sha256").update(session.sessionId).digest("hex"), "events.jsonl");
    await mkdir(dirname(ledgerPath), { recursive: true });
    await writeFile(ledgerPath, [
      { operation: "upsert", rule: { ...modeless, id: "stale-rule", action: { delivery: "followUp", content: "Old." } }, replace: false },
      { operation: "upsert", rule, replace: false },
    ].map(mutation => `${JSON.stringify({ type: "rule-mutation", mutation })}\n`).join(""));
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot,
      modelPath: resolve("model"),
      databasePath: join(root, "rerail.sqlite"),
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => false,
        auditGeneratedEdits: async (_sessionId, batch) => ({
          revision: batch.revision, disposition: "continue", violations: [], failures: [],
        }),
        close: async () => undefined,
      }),
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      const listed = await runtime.manageRules(session.sessionId, { action: "list" }) as { rules: Array<{ rule: { id: string } }> };
      const ids = listed.rules.map(inspection => inspection.rule.id);
      expect(ids).toContain("restored-rule");
      expect(ids).not.toContain("stale-rule");
      expect(stderr.mock.calls.map(([line]) => String(line))).toContainEqual(
        expect.stringMatching(/^rerail: skipped unreadable rule mutation: persisted rule\.\S+ .*\n$/u),
      );
    } finally {
      stderr.mockRestore();
      await runtime.dispose();
    }
  });

  it("does not parse a host conversation-rules.json again once the catalog imported it", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-imported-rule-file-"));
    directories.push(root);
    const rulePath = join(root, ".codex", "conversation-rules.json");
    const rule = {
      id: "imported-rule", mode: "steer", reference: "Guest checkout remains available.",
      match: { operator: "gte", threshold: 0.99 },
      sample: { source: "assistant-text", boundary: "message" },
      action: { content: "Check checkout." },
    };
    await mkdir(dirname(rulePath), { recursive: true });
    await writeFile(rulePath, JSON.stringify({ version: 1, rules: [rule] }));
    const runtime = () => new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      modelPath: resolve("model"),
      databasePath: join(root, "rerail.sqlite"),
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => false,
        auditGeneratedEdits: async (_sessionId, batch) => ({
          revision: batch.revision, disposition: "continue", violations: [], failures: [],
        }),
        close: async () => undefined,
      }),
    });
    const first = runtime();
    try {
      await first.ingest({ kind: "session-start", eventId: "first", session: { host: "codex", sessionId: "first", cwd: root } });
    } finally {
      await first.dispose();
    }

    const { mode: _mode, ...modeless } = rule;
    await writeFile(rulePath, JSON.stringify({ version: 1, rules: [{ ...modeless, action: { delivery: "steer", content: "Check checkout." } }] }));
    const second = runtime();
    try {
      await second.ingest({ kind: "session-start", eventId: "second", session: { host: "codex", sessionId: "second", cwd: root } });
      const listed = await second.manageRules("second", { action: "list" }) as { rules: Array<{ rule: { id: string } }> };
      expect(listed.rules.map(inspection => inspection.rule.id)).toContain("imported-rule");
    } finally {
      await second.dispose();
    }
  });
});
