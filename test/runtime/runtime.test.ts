import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type AdapterDelivery,
  type CodeRuleViolation,
  type RuleSteerDelivery,
  CodeRuleBridge,
  openRerailDatabase,
  SqliteAuditControlStore,
  setRuleSetting,
  renderSteer,
} from "../../src/engine/index.js";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import { SqliteSessionStore } from "../../src/runtime/session-store.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const directories: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
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
      // Queued steers can be collected once the evaluator has opened.
      await runtime.codeRulesOpened(session.sessionId);
      expect(await runtime.ingest({ kind: "turn-end", eventId: "opened", sessionId: session.sessionId, text: "" })).toEqual([]);
      const steer = { revision: 1, scope: ["lib/a.js", "lib/z.js"] };
      expect(await delivery!.requestSteer(session.sessionId, "steer", "CommonJS require", steer)).toBe("queued");
      // A package.json edit at revision 2 gave lib/a.js new facts: the queued steer leaves the queue, and lib/z.js,
      // which it also spoke for, is handed back.
      expect(delivery!.revised!(session.sessionId, 2, ["lib/a.js"])).toEqual(["lib/z.js"]);
      // Gone from the queue: a later revision finds nothing of it to drop or hand back.
      expect(delivery!.revised!(session.sessionId, 2, ["lib/x.js"])).toEqual([]);
      expect(await delivery!.requestSteer(session.sessionId, "steer", "CommonJS require", steer)).toBe("discarded");
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Refreshed", { revision: 2, scope: ["lib/a.js"] })).toBe("queued");
      // A steer for several sources is dropped whole when one is revised; the others, not revised since, are handed
      // back for the auditor to steer again, and sources revised with it are not.
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Both", { revision: 3, scope: ["lib/b.js", "lib/c.js"] })).toBe("queued");
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Other", { revision: 3, scope: ["lib/d.js"] })).toBe("queued");
      expect(delivery!.revised!(session.sessionId, 4, ["lib/c.js"])).toEqual(["lib/b.js"]);
      expect(await delivery!.requestSteer(session.sessionId, "steer", "Wide", { revision: 4, scope: ["lib/b.js", "lib/e.js"] })).toBe("queued");
      expect(delivery!.revised!(session.sessionId, 5, ["lib/b.js", "lib/e.js"])).toEqual([]);
      // Only the steers no revision made stale are delivered.
      expect((await runtime.collectDeliveries(session.sessionId)).map(delivered => delivered.content)).toEqual(["Refreshed", "Other"]);
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

describe("Rerail tool results waiting for their edit's check", () => {
  /** What `a.ts` holds once an edit writes `bad`: the finding sits on its second line, seventh column. */
  const BAD = "export const a = 1;\nconst bad = 2;\n";
  function finding(path: string): CodeRuleViolation {
    return {
      id: `no-bad:${path}`, ruleId: "test:no-bad", clauseId: "clause", code: "NO_BAD", mode: "steer", message: "Do not write bad.",
      path, span: { startByte: 26, endByte: 29, startLine: 1, startColumn: 6, endLine: 1, endColumn: 9 }, captures: {}, variables: {},
    };
  }

  /**
   * A session whose edit checks finish only once the test opens `check`, each steering its file's finding when the
   * edit wrote `bad`: Claude Code's unless `host` says otherwise.
   */
  async function hookSession(host: "claude-code" | "omp" = "claude-code") {
    // The real path: macOS's temporary folder is reached through a symlink, which would name the file twice.
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-hook-check-")));
    directories.push(root);
    await writeFile(join(root, "a.ts"), "export const a = 1;\n");
    const check = Promise.withResolvers<void>();
    const checking = Promise.withResolvers<void>();
    const steered = Promise.withResolvers<void>();
    const checks: Promise<void>[] = [];
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: {
        fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
        embed: async texts => texts.map(() => new Float32Array([1, 0])),
      },
      createAuditor: ({ steerDelivery }) => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (sessionId, batch) => {
          const violations = batch.edits.filter(edit => edit.generatedText?.includes("bad")).map(edit => finding(edit.path));
          checks.push((async () => {
            await check.promise;
            if (violations.length === 0) return;
            const steer = renderSteer(violations);
            await steerDelivery.requestSteer(sessionId, "steer", steer, {
              revision: batch.revision, scope: batch.edits.map(edit => edit.path), ruleIds: ["test:no-bad"],
              rules: [{ id: "test:no-bad", state: "steered" }], background: true,
              generatedEdit: { violations, sourceOf: () => undefined },
            });
            steered.resolve();
          })());
          return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
        },
        settleGeneratedEdits: async () => {
          checking.resolve();
          await Promise.all(checks);
        },
        close: async () => undefined,
      }),
    });
    const session = { host, sessionId: "hook-check", cwd: root };
    await runtime.ingest({ kind: "session-start", eventId: "start", session });
    await runtime.codeRulesOpened(session.sessionId);
    /** Writes `text` into `a.ts` with the Write tool; `answering` is the tool result's answer. */
    const edit = async (id: string, text: string) => {
      await runtime.ingest({ kind: "call", eventId: `call-${id}`, sessionId: session.sessionId, toolCallId: id, toolName: "Write", input: { file_path: join(root, "a.ts"), content: text } });
      await writeFile(join(root, "a.ts"), text);
      return { answering: runtime.ingest({ kind: "result", eventId: `result-${id}`, sessionId: session.sessionId, toolCallId: id, toolName: "Write", output: "ok" }) };
    };
    const turnEnd = (eventId: string) => runtime.ingest({ kind: "turn-end", eventId, sessionId: session.sessionId, text: "" });
    return { runtime, check, checking, steered, edit, turnEnd };
  }

  /** `a.ts`'s finding as a steer renders it, and the digest that lists it at the end of the agent's run. */
  const STEER = renderSteer([finding("a.ts")]);
  const DIGEST = renderSteer([finding("a.ts")], { kind: "late" });

  it("waits for the edit's check, so its steer goes back with the tool result", async () => {
    const { runtime, check, edit } = await hookSession();
    try {
      const { answering } = await edit("t1", BAD);
      check.resolve();
      expect((await answering).map(delivery => delivery.content)).toEqual([STEER]);
    } finally {
      await runtime.dispose();
    }
  });

  it("with the experimental digest off, answers after a second when the check is still running, and delivers its steer at the next event", async () => {
    const { runtime, check, checking, steered, edit } = await hookSession();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const { answering } = await edit("t1", BAD);
      await checking.promise;
      await vi.advanceTimersByTimeAsync(1_000);
      expect(await answering).toEqual([]);
      vi.useRealTimers();

      check.resolve();
      await steered.promise;
      expect((await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId: "hook-check", text: "next" })).map(delivery => delivery.content))
        .toEqual([STEER]);
    } finally {
      vi.useRealTimers();
      check.resolve();
      await runtime.dispose();
    }
  });

  it("with the experimental digest on, answers after a second when the check is still running, and lists its steer in the digest at the turn's end, not at the next event", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const { runtime, check, checking, steered, edit, turnEnd } = await hookSession();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      let answered = false;
      const { answering } = await edit("t1", BAD);
      void answering.finally(() => { answered = true; });
      // The hook's one-second wait starts as the check does.
      await checking.promise;
      await vi.advanceTimersByTimeAsync(999);
      expect(answered).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(await answering).toEqual([]);
      vi.useRealTimers();

      check.resolve();
      await steered.promise;
      // The step that made the edit is over: the steer would land on an unrelated one.
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId: "hook-check", text: "next" })).toEqual([]);
      const stop = await turnEnd("turn");
      expect(stop).toEqual([expect.objectContaining({ requested: "steer", ruleIds: ["test:no-bad"], content: DIGEST })]);
      expect(stop[0]!.content).toContain("- a.ts:2:7 — Do not write bad. [test:no-bad]");
      // Listed once.
      expect(await turnEnd("turn-2")).toEqual([]);
    } finally {
      vi.useRealTimers();
      check.resolve();
      await runtime.dispose();
    }
  });

  it("with the experimental digest on, answers an OMP tool result after ten seconds of a check still running, and holds its steer for the digest at the end of the run", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const { runtime, check, checking, steered, edit, turnEnd } = await hookSession("omp");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      let answered = false;
      const { answering } = await edit("t1", BAD);
      void answering.finally(() => { answered = true; });
      await checking.promise;
      await vi.advanceTimersByTimeAsync(9_999);
      expect(answered).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(await answering).toEqual([]);
      // Checks run in order, so the turn end's would wait behind the same one: it answers without spending ten seconds again.
      await expect(turnEnd("turn")).resolves.toEqual([]);
      vi.useRealTimers();

      check.resolve();
      await steered.promise;
      // Neither the host's poll nor a later step carries it, and an OMP turn end is not the end of the run.
      expect(await runtime.collectDeliveries("hook-check")).toEqual([]);
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId: "hook-check", text: "next" })).toEqual([]);
      expect(await turnEnd("turn-2")).toEqual([]);
      const yielded = await runtime.yieldDeliveries("hook-check");
      expect(yielded).toEqual([expect.objectContaining({ requested: "steer", mechanism: "native-queue", ruleIds: ["test:no-bad"], content: DIGEST })]);
      expect(await runtime.yieldDeliveries("hook-check")).toEqual([]);
    } finally {
      vi.useRealTimers();
      check.resolve();
      await runtime.dispose();
    }
  });

  it("with the experimental digest on, drops a held finding once its file is edited again before the run ends", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const { runtime, check, checking, steered, edit } = await hookSession("omp");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const { answering } = await edit("t1", BAD);
      await checking.promise;
      await vi.advanceTimersByTimeAsync(10_000);
      expect(await answering).toEqual([]);
      vi.useRealTimers();
      check.resolve();
      await steered.promise;

      const fixed = await edit("t2", "export const a = 2;\n");
      expect(await fixed.answering).toEqual([]);
      expect(await runtime.yieldDeliveries("hook-check")).toEqual([]);
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

  it("leads a tool call's guidance with the tool, and credits its rule in the delivery's rule ids and rules", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-tool-steer-"));
    directories.push(root);
    const databasePath = join(root, "rerail.sqlite");
    setUpTestRepository(root, {
      library: [{
        kind: "conversation",
        rule: {
          id: "acme:no-force-push",
          mode: "steer",
          match: { field: "input.command", regex: String.raw`\bgit\s+push\b[^\n]*--force\b` },
          sample: { source: "tool:Bash", boundary: "tool-call" },
          action: { content: "Ask before force-pushing." },
          repeat: { mode: "always" },
        },
      }],
    }, databasePath);
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      modelPath: resolve("model"),
      databasePath,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => false,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "tool", cwd: root } });
      const deliveries = await runtime.ingest({
        kind: "call", eventId: "push", sessionId: "tool", toolCallId: "push", toolName: "Bash", input: { command: "git push --force origin main" },
      });
      expect(deliveries).toEqual([expect.objectContaining({
        requested: "steer",
        content: [
          "Rerail has guidance for this Bash call:",
          "- Ask before force-pushing. [acme:no-force-push]",
          "Follow it, or tell the user in one line why you won't.",
        ].join("\n"),
        ruleIds: ["acme:no-force-push"],
        rules: [{ id: "acme:no-force-push", state: "steered" }],
      })]);
    } finally {
      await runtime.dispose();
    }
  });

  it("stores a tool output's secret redacted, while the rule that matches it still sees it and steers", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-secret-output-"));
    directories.push(root);
    const databasePath = join(root, "rerail.sqlite");
    // Assembled from parts so no key-shaped literal sits in the file for secret scanners to refuse.
    const key = ["sk-", "proj-", "Q7f2LmZ9xV4pR8tK1wN6bH3cJ5dS0gYa"].join("");
    setUpTestRepository(root, {
      library: [{
        kind: "conversation",
        rule: {
          id: "acme:no-key-in-output",
          mode: "steer",
          match: { field: "text", regex: "sk-proj-[A-Za-z0-9]{32}" },
          sample: { source: "tool:Bash", boundary: "tool-call" },
          action: { content: "Rotate the key the output showed." },
          repeat: { mode: "always" },
        },
      }],
    }, databasePath);
    const runtime = new RerailRuntime({
      autostart: false,
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      modelPath: resolve("model"),
      databasePath,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => false,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "keys", cwd: root } });
      await runtime.ingest({ kind: "call", eventId: "cat", sessionId: "keys", toolCallId: "cat", toolName: "Bash", input: { command: "cat config.py" } });
      const deliveries = await runtime.ingest({
        kind: "result", eventId: "cat-out", sessionId: "keys", toolCallId: "cat", toolName: "Bash", output: `API_KEY = "${key}"\n`,
      });
      expect(deliveries).toEqual([expect.objectContaining({ requested: "steer", ruleIds: ["acme:no-key-in-output"] })]);
    } finally {
      await runtime.dispose();
    }
    const database = openRerailDatabase(databasePath, { readonly: true });
    let stored: string;
    try {
      stored = database.$client.prepare("SELECT payload_json FROM events WHERE event_id = 'cat-out'").pluck().get() as string;
    } finally {
      database.$client.close();
    }
    expect(stored).not.toContain(key);
    expect(JSON.parse(stored)).toMatchObject({ output: expect.stringMatching(/^API_KEY = "\[redacted:openai-key:[0-9a-f]{6}\]"\n$/u) });
    const ledger = await readFile(join(root, "sessions", createHash("sha256").update("keys").digest("hex"), "events.jsonl"), "utf8");
    expect(ledger).not.toContain(key);
    expect(ledger).toContain("[redacted:openai-key:");
  });
});
