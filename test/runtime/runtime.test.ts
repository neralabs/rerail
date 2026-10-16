import { createHash } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type AdapterDelivery,
  type RuleSteerDelivery,
  CodeRuleBridge,
  openRerailDatabase,
  SqliteAuditControlStore,
  setRuleSetting,
} from "../../src/engine/index.js";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import { SqliteSessionStore } from "../../src/runtime/session-store.js";
import { setUpTestRepository } from "../support/repository-rules.js";

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

  it("delivers what an evaluator observes while it opens, and discards a failed or retired evaluator's observations", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-observations-"));
    directories.push(root);
    const callbacks: RuleSteerDelivery[] = [];
    const deliveries: AdapterDelivery[] = [];
    const delivered = Promise.withResolvers<void>();
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
            if (attempt === 1) throw new Error("Workspace attachment failed");
          },
          acceptsPath: () => true,
          auditGeneratedEdits: async (_sessionId, batch) => ({
            revision: batch.revision, disposition: "continue", violations: [], failures: [],
          }),
          close: async () => undefined,
        };
      },
    });
    const failed = { host: "omp" as const, sessionId: "failed-observation", cwd: root };
    const opened = { host: "omp" as const, sessionId: "opened-observation", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "failed", session: failed });
      // A turn boundary waits for the evaluator to have opened or failed.
      expect(await runtime.ingest({ kind: "turn-end", eventId: "failed-turn", sessionId: failed.sessionId, text: "" })).toEqual([]);
      expect(await callbacks[0]!.requestSteer(failed.sessionId, "steer", "Late observation", {
        revision: 0, scope: [], background: true,
      })).toBe("discarded");
      await runtime.ingest({ kind: "session-start", eventId: "opened", session: opened });
      await delivered.promise;
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: opened.sessionId });
      expect(await callbacks[1]!.requestSteer(opened.sessionId, "steer", "Retired observation", {
        revision: 0, scope: [], background: true,
      })).toBe("discarded");
      expect(deliveries).toEqual([
        expect.objectContaining({ sessionId: opened.sessionId, content: "Observation 2" }),
      ]);
    } finally {
      await runtime.dispose();
    }
  });

  it("hands a polling host nothing an evaluator observed while opening until it has opened", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-opening-observation-"));
    directories.push(root);
    const opening = Promise.withResolvers<void>();
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
      createAuditor: ({ steerDelivery }) => ({
        openWorkspace: async sessionId => {
          await steerDelivery.requestSteer(sessionId, "steer", "Observed while opening", { revision: 0, scope: [] });
          await opening.promise;
          throw new Error("Workspace attachment failed");
        },
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    const session = { host: "omp" as const, sessionId: "opening-observation", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: session.sessionId, text: "go" })).toEqual([]);
      expect(await runtime.collectDeliveries(session.sessionId)).toEqual([]);
      opening.resolve();
      expect(await runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "" })).toEqual([]);
    } finally {
      opening.resolve();
      await runtime.dispose();
    }
  });

  it("closes a call left open by a previous runtime even when the evaluator cannot open, so it is not begun again later", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-unopened-call-"));
    directories.push(root);
    const session = { host: "omp" as const, sessionId: "unopened-call", cwd: root };
    const begun = vi.spyOn(CodeRuleBridge.prototype, "begin");
    const runtime = (opens: boolean) => new RerailRuntime({
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
      createAuditor: () => ({
        openWorkspace: async () => { if (!opens) throw new Error("Workspace unavailable"); },
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    const call = { kind: "call" as const, eventId: "call", sessionId: session.sessionId, toolCallId: "write", toolName: "write", input: { path: "a.py" } };
    try {
      const first = runtime(true);
      await first.ingest({ kind: "session-start", eventId: "start-1", session });
      await first.ingest(call);
      await first.dispose();

      const unopened = runtime(false);
      await unopened.ingest({ kind: "session-start", eventId: "start-2", session });
      await unopened.ingest({ kind: "result", eventId: "result", sessionId: session.sessionId, toolCallId: "write", toolName: "write", output: "written" });
      await unopened.dispose();

      begun.mockClear();
      const reopened = runtime(true);
      await reopened.ingest({ kind: "session-start", eventId: "start-3", session });
      await reopened.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "" });
      await reopened.dispose();
      expect(begun).not.toHaveBeenCalled();
    } finally {
      begun.mockRestore();
    }
  });

  it("treats a steer for a source the auditor revised without an edit of its own as stale", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-revised-"));
    directories.push(root);
    let delivery: RuleSteerDelivery | undefined;
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
      createAuditor: ({ steerDelivery }) => {
        delivery = steerDelivery;
        return {
          openWorkspace: async () => undefined,
          acceptsPath: () => true,
          auditGeneratedEdits: async (_sessionId, batch) => ({
            revision: batch.revision, disposition: "continue", violations: [], failures: [],
          }),
          close: async () => undefined,
        };
      },
    });
    const session = { host: "omp" as const, sessionId: "revised-source", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      const steer = { revision: 1, scope: ["lib/a.js"] };
      expect(await delivery!.requestSteer(session.sessionId, "steer", "CommonJS require", steer)).toBe("queued");
      // A package.json edit at revision 2 gave the source new facts.
      delivery!.revised!(session.sessionId, 2, ["lib/a.js"]);
      expect(await delivery!.requestSteer(session.sessionId, "steer", "CommonJS require", steer)).toBe("discarded");
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Refreshed", { revision: 2, scope: ["lib/a.js"] })).toBe("queued");
      // A steer for several sources is dropped whole when one is revised; the others, not revised since, are handed
      // back for the auditor to steer again, and sources revised with it are not.
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Both", { revision: 3, scope: ["lib/b.js", "lib/c.js"] })).toBe("queued");
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Other", { revision: 3, scope: ["lib/d.js"] })).toBe("queued");
      expect(delivery!.revised!(session.sessionId, 4, ["lib/c.js"])).toEqual(["lib/b.js"]);
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Wide", { revision: 4, scope: ["lib/b.js", "lib/e.js"] })).toBe("queued");
      expect(delivery!.revised!(session.sessionId, 5, ["lib/b.js", "lib/e.js"])).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });
});

describe("A scan requested while the session's evaluator is still opening", () => {
  /** A runtime whose evaluator's `openWorkspace` is held until `opening` settles, with one scan requested and claimed meanwhile. */
  async function scanFixture() {
    const root = await mkdtemp(join(tmpdir(), "rerail-scan-opening-"));
    directories.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const opening = Promise.withResolvers<void>();
    const scans: string[] = [];
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath,
      embedder: {
        fingerprint: () => ({
          modelId: "test", revision: "1", dimensions: 2,
          weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1,
        }),
        embed: async texts => texts.map(() => new Float32Array([1, 0])),
      },
      createAuditor: ({ audits, workspaceRoot }) => ({
        openWorkspace: async () => { await opening.promise; },
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        audit: async (sessionId, input) => {
          // The repository audit the runtime starts itself once open is not the scan under test.
          if (input.audit !== undefined) return [];
          const auditId = audits.start({
            sessionId, workspaceRoot, scope: input.scope, mode: input.mode,
            revision: 0, catalogRevision: null, ruleRevisions: [], paths: [],
          });
          audits.finish(auditId, { status: "completed", violations: [], failures: [] });
          scans.push(auditId);
          return [auditId];
        },
        close: async () => undefined,
      }),
    });
    const session = { host: "omp" as const, sessionId: "scan-opening", cwd: root };
    await runtime.ingest({ kind: "session-start", eventId: "start", session });
    const controls = new SqliteAuditControlStore(databasePath);
    const request = controls.requestAudit(session.sessionId, { scope: "repository" });
    // The control timer claims it within 250 ms, with the evaluator still held in its open.
    const statusOf = () => controls.listAuditRequests(session.sessionId).find(item => item.id === request.id)?.status;
    await vi.waitFor(() => expect(statusOf()).not.toBe("queued"), { timeout: 4_000 });
    expect(statusOf()).toBe("running");
    const settled = async () => {
      await vi.waitFor(
        () => expect(controls.listAuditRequests(session.sessionId).find(item => item.id === request.id)?.status).not.toBe("running"),
        { timeout: 10_000 },
      );
      return controls.listAuditRequests(session.sessionId).find(item => item.id === request.id)!;
    };
    const close = async () => {
      opening.resolve();
      await runtime.dispose();
      controls.close();
    };
    return { opening, scans, settled, close };
  }

  it("completes with its audit once the evaluator has opened", async () => {
    const test = await scanFixture();
    try {
      expect(test.scans).toEqual([]);
      test.opening.resolve();
      const request = await test.settled();
      expect(request).toMatchObject({ status: "completed", auditIds: test.scans, error: null });
      expect(test.scans).toHaveLength(1);
    } finally { await test.close(); }
  });

  it("reports why the evaluator could not open when it fails, not that a run went unrecorded", async () => {
    const test = await scanFixture();
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      test.opening.reject(new Error("Workspace unavailable"));
      const request = await test.settled();
      expect(request).toMatchObject({ status: "failed", auditIds: [] });
      expect(request.error).toContain("Workspace unavailable");
      expect(request.error).not.toContain("without a recorded run");
    } finally {
      stderr.mockRestore();
      await test.close();
    }
  });
});

describe("Rerail tool results from a one-shot hook", () => {
  /** A Claude Code session whose edit check finishes, and steers, only once the test opens `check`. */
  async function hookSession() {
    const root = await mkdtemp(join(tmpdir(), "rerail-hook-check-"));
    directories.push(root);
    await writeFile(join(root, "a.ts"), "export const a = 1;\n");
    const check = Promise.withResolvers<void>();
    const checking = Promise.withResolvers<void>();
    const steered = Promise.withResolvers<void>();
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: {
        fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
        embed: async texts => texts.map(() => new Float32Array([1, 0])),
      },
      createAuditor: ({ steerDelivery }) => {
        let revision = 0;
        return {
          openWorkspace: async () => undefined,
          acceptsPath: () => true,
          auditGeneratedEdits: async (_sessionId, batch) => {
            revision = batch.revision;
            return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
          },
          settleGeneratedEdits: async () => {
            checking.resolve();
            await check.promise;
            await steerDelivery.requestSteer("hook-check", "steer", "Checked the edit.", { revision, scope: [], background: true });
            steered.resolve();
          },
          close: async () => undefined,
        };
      },
    });
    const session = { host: "claude-code" as const, sessionId: "hook-check", cwd: root };
    await runtime.ingest({ kind: "session-start", eventId: "start", session });
    await runtime.ingest({ kind: "call", eventId: "call", sessionId: session.sessionId, toolCallId: "t1", toolName: "Write", input: { file_path: join(root, "a.ts"), content: "x" } });
    const result = () => runtime.ingest({ kind: "result", eventId: "result", sessionId: session.sessionId, toolCallId: "t1", toolName: "Write", output: "ok" });
    return { runtime, check, checking, steered, result };
  }

  it("waits for the edit's check, so its steer goes back with the tool result", async () => {
    const { runtime, check, result } = await hookSession();
    try {
      const answering = result();
      check.resolve();
      expect((await answering).map(delivery => delivery.content)).toEqual(["Checked the edit."]);
    } finally {
      await runtime.dispose();
    }
  });

  it("answers after a second when the check is still running, and delivers its steer at the next event", async () => {
    const { runtime, check, checking, steered, result } = await hookSession();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      let answered = false;
      const answering = result().finally(() => { answered = true; });
      // The hook's one-second wait starts as the check does.
      await checking.promise;
      await vi.advanceTimersByTimeAsync(999);
      expect(answered).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(await answering).toEqual([]);
      vi.useRealTimers();

      check.resolve();
      await steered.promise;
      const next = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId: "hook-check", text: "next" });
      expect(next.map(delivery => delivery.content)).toEqual(["Checked the edit."]);
    } finally {
      vi.useRealTimers();
      check.resolve();
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

  it("runs only the repository's configured conversation rules: none before set-up, never one switched off, and never a rule file", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-rule-file-overlay-"));
    directories.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const rule = (id: string, content: string) => ({
      id, mode: "steer" as const, reference: "Guest checkout remains available.",
      match: { operator: "gte" as const, threshold: 0.99 },
      sample: { source: "assistant-text" as const, boundary: "message" as const },
      action: { content },
    });
    const rulePath = join(root, ".codex", "conversation-rules.json");
    await mkdir(dirname(rulePath), { recursive: true });
    await writeFile(rulePath, JSON.stringify({
      version: 1,
      rules: [rule("shared", "From the file."), rule("switched-off", "From the file."), rule("file-only", "From the file.")],
    }));
    const runtime = () => new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      modelPath: resolve("model"),
      databasePath,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => false,
        auditGeneratedEdits: async (_sessionId, batch) => ({
          revision: batch.revision, disposition: "continue", violations: [], failures: [],
        }),
        close: async () => undefined,
      }),
    });
    type Listed = { rules: Array<{ rule: { id: string; action: { content: string } } }> };

    // A checkout whose repository is not set up runs no rules; a rule file in the working tree is never read.
    const before = runtime();
    try {
      await before.ingest({ kind: "session-start", eventId: "before", session: { host: "codex", sessionId: "before", cwd: root } });
      expect((await before.manageRules("before", { action: "list" }) as Listed).rules).toEqual([]);
    } finally {
      await before.dispose();
    }

    const repositoryKey = setUpTestRepository(root, {
      library: [
        { kind: "conversation", rule: rule("shared", "From the library.") },
        { kind: "conversation", rule: rule("switched-off", "From the library.") },
      ],
    }, databasePath);
    const database = openRerailDatabase(databasePath);
    try {
      setRuleSetting(database, {
        owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: "switched-off", on: false, mode: null,
      });
    } finally {
      database.$client.close();
    }
    const after = runtime();
    try {
      await after.ingest({ kind: "session-start", eventId: "after", session: { host: "codex", sessionId: "after", cwd: root } });
      const listed = await after.manageRules("after", { action: "list" }) as Listed;
      expect(listed.rules.map(({ rule: { id, action } }) => [id, action.content])).toEqual([
        ["shared", "From the library."],
      ]);
    } finally {
      await after.dispose();
    }
  });
});
