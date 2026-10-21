import { mkdir, opendir, realpath, writeFile } from "node:fs/promises";
import type * as FsPromises from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, RerailHost } from "../../../src/engine/adapter-contract.js";
import type { CodeRuleAuditor, RuleSteerDelivery } from "../../../src/engine/code-rule/code-rule.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { RerailRuntime, SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../../support/hook-events.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";
import { gateStorageAttachments } from "../support/storage-attach-gate.js";

vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FsPromises>();
  return { ...actual, opendir: vi.fn(actual.opendir) };
});

const temporaryDirectory = temporaryDirectories("rerail-policy-runtime-");

interface FixtureOptions {
  deliverBackground?(delivery: AdapterDelivery): Promise<void>;
  host?: RerailHost;
  /** Runs before the session starts, e.g. to seed sources the baseline walk must observe. */
  prepare?(root: string): Promise<void>;
  auditGeneratedEdits?(steerDelivery: RuleSteerDelivery): CodeRuleAuditor["auditGeneratedEdits"];
  /** Start the workspace's repository and gate the runtime on it, as resident runtimes are. */
  startGate?: boolean;
  timingLog?(line: string): void;
}
async function fixture({ deliverBackground, host = "claude-code", prepare, auditGeneratedEdits, startGate, timingLog }: FixtureOptions = {}) {
  const root = await temporaryDirectory();
  await prepare?.(root);
  const databasePath = join(root, "rerail.sqlite");
  if (startGate === true) startTestRepository(root, databasePath);
  let steerDelivery: RuleSteerDelivery | undefined;
  const opened = Promise.withResolvers<void>();
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root,
    embedder: testEmbedder, startGate: startGate === true,
    ...(deliverBackground === undefined ? {} : { deliverBackground }),
    ...(timingLog === undefined ? {} : { timingLog }),
    createAuditor: options => {
      steerDelivery = options.steerDelivery;
      return {
        openWorkspace: async () => { opened.resolve(); },
        acceptsPath: () => true,
        auditGeneratedEdits: auditGeneratedEdits?.(options.steerDelivery)
          ?? (async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] })),
        close: async () => undefined,
      };
    },
  });
  const session = { host, sessionId: "session-1", cwd: root };
  await runtime.ingest({ kind: "session-start", eventId: "start", session });
  // The evaluator opens behind the session start; what is left of that after its openWorkspace is microtasks.
  await opened.promise;
  await new Promise<void>(resolve => setImmediate(resolve));
  if (steerDelivery === undefined) throw new Error("Policy delivery was not initialized");
  let event = 0;
  return {
    runtime, steerDelivery: steerDelivery, session, root, databasePath,
    poll: () => runtime.ingest({ kind: "user-prompt", eventId: `poll-${++event}`, sessionId: session.sessionId, text: "continue" }),
    close: () => runtime.dispose(),
  };
}

/** One steer rule flagging `NEEDLE` in any Python source. */
function needleCatalog() {
  return testCodeRuleCatalog([codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{
      id: "needle",
      violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
      forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
    }],
  })]);
}

describe("source-scoped steers", () => {
  it("starts and accepts edits while repository discovery is pending, then records audit errors and delivers fresh violations", async () => {
    const root = await realpath(await temporaryDirectory());
    await writeFile(join(root, "broken.py"), Buffer.from([0]));
    await writeFile(join(root, "edited.py"), "before\n");
    const databasePath = join(root, "rerail.sqlite");
    const catalog = testCodeRuleCatalog([codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:startup", revision: 1, mode: "block",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{
        id: "needle",
        violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
        forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
      }],
    })]);
    const gate = Promise.withResolvers<void>();
    const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
    vi.mocked(opendir).mockImplementation(async (...args) => {
      if (args[0] === root) await gate.promise;
      return actual.opendir(...args);
    });
    let enforcer: NativeCodeRuleEnforcer | undefined;
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath: databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => {
        enforcer = new NativeCodeRuleEnforcer(catalog, options);
        return enforcer;
      },
    });
    const audits = new SqliteAuditStore(databasePath);
    const session = { host: "omp" as const, sessionId: "startup", cwd: root };
    let ready = false;
    const start = runtime.ingest({ kind: "session-start", eventId: "start", session }).then(result => {
      ready = true;
      return result;
    });
    try {
      await vi.waitFor(() => expect(ready).toBe(true));
      expect(enforcer!.snapshotStatus().pending).toBe(true);
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([
        expect.objectContaining({ scope: "repository", status: "running" }),
      ]);
      await runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "edited.py" },
      });
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      const editDeliveries = await runtime.ingest({
        kind: "result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      });
      await vi.waitFor(() => expect(audits.list({ sessionId: session.sessionId }).audits).toEqual(expect.arrayContaining([
        expect.objectContaining({ scope: "ai-output", status: "completed" }),
        expect.objectContaining({ scope: "repository", status: "running" }),
      ])));
      expect(editDeliveries).toEqual([expect.objectContaining({ content: expect.stringContaining("- edited.py:1:1 — No needle [") })]);
      expect(await runtime.ingest({
        kind: "user-prompt", eventId: "poll", sessionId: session.sessionId, text: "continue",
      })).toEqual([]);
      await writeFile(join(root, "shell.py"), "safe()\n");
      gate.resolve();
      await enforcer!.settle();
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual(expect.arrayContaining([
        expect.objectContaining({
          // A binary file is listed, but files read only in part never fail a repository audit.
          scope: "repository", status: "completed",
          failures: expect.arrayContaining([expect.objectContaining({ path: "broken.py", code: "source-incomplete" })]),
        }),
      ]));
      // A late baseline cannot overwrite the edit observation and replay it at the next checkpoint.
      expect(await runtime.ingest({
        kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "done",
      })).toEqual([]);
      expect(audits.list({ sessionId: session.sessionId }).audits.filter(audit => audit.scope === "ai-output").flatMap(audit => audit.paths).sort())
        .toEqual(["edited.py", "shell.py"]);
    } finally {
      gate.resolve();
      await start;
      await runtime.dispose();
      audits.close();
      vi.mocked(opendir).mockRestore();
    }
  });

  it("evaluates an edit to a source that a rule refreshed at the tool call brings into scope", async () => {
    const root = await realpath(await temporaryDirectory());
    await mkdir(join(root, "web"));
    const databasePath = join(root, "rerail.sqlite");
    const needleIn = (revision: number, include: string) => codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:widened", mode: "steer", revision,
      scope: { include: [include], exclude: [] },
      clauses: [{
        id: "needle",
        violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
        forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
      }],
    });
    let codeRules = [needleIn(1, "api/**/*.py")];
    const catalog = testCodeRuleCatalog(() => codeRules);
    const attachments = gateStorageAttachments();
    attachments.gated = false;
    let result = false;
    let enforcer: NativeCodeRuleEnforcer | undefined;
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => {
        enforcer = new NativeCodeRuleEnforcer(catalog, options);
        const accepts = enforcer.acceptsPath.bind(enforcer);
        // Storage for the refreshed rules attaches only once the tool result's edit has been filtered by scope.
        enforcer.acceptsPath = path => {
          const accepted = accepts(path);
          if (result) for (const workspace of attachments.workspaces) workspace.release();
          return accepted;
        };
        return enforcer;
      },
    });
    const session = { host: "omp" as const, sessionId: "widened", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      // The evaluator opens behind the session start: wait for it and its repository audit before gating attaches.
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(2));
      await enforcer!.settle();
      attachments.gated = true;
      codeRules = [needleIn(2, "**/*.py")];
      await runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "web/new.py" },
      });
      await writeFile(join(root, "web", "new.py"), "NEEDLE\n");
      result = true;
      const deliveries = await runtime.ingest({
        kind: "result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      });
      expect(deliveries).toEqual([expect.objectContaining({ content: expect.stringContaining("web/new.py") })]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await runtime.dispose();
    }
  });

  it("ends a turn without waiting for the session-start repository audit, once the turn's edits are audited", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    // The session's evaluator attaches at once; the repository audit's own workspace waits for the gate.
    const attachments = gateStorageAttachments();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
    });
    const audits = new SqliteAuditStore(databasePath);
    const session = { host: "omp" as const, sessionId: "turn", cwd: root };
    try {
      const start = runtime.ingest({ kind: "session-start", eventId: "start", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(1));
      attachments.workspaces[0]!.release();
      await start;
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(2));
      await runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "edited.py" },
      });
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      expect(await runtime.ingest({
        kind: "result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      })).toEqual([expect.objectContaining({ content: expect.stringContaining("- edited.py:1:1 — No needle [") })]);
      let ended = false;
      const turnEnd = runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "done" })
        .finally(() => { ended = true; });
      await vi.waitFor(() => expect(ended).toBe(true), { timeout: 3_000 });
      await turnEnd;
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual(expect.arrayContaining([
        expect.objectContaining({ scope: "repository", status: "running" }),
        expect.objectContaining({ scope: "ai-output", status: "completed", paths: ["edited.py"] }),
      ]));
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await runtime.dispose();
      audits.close();
    }
  });

  it("closes a session only once its session-start repository audit has stopped", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    const attachments = gateStorageAttachments();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
    });
    const audits = new SqliteAuditStore(databasePath);
    const session = { host: "omp" as const, sessionId: "closing", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(1));
      attachments.workspaces[0]!.release();
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(2));
      let closed = false;
      const closing = runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId })
        .finally(() => { closed = true; });
      // The audit is interrupted as the close begins; its own workspace still waits for the gate.
      await vi.waitFor(() => expect(audits.list({ sessionId: session.sessionId }).audits).toEqual(expect.arrayContaining([
        expect.objectContaining({ scope: "repository", status: "interrupted" }),
      ])));
      await new Promise<void>(resolve => setImmediate(resolve));
      expect(closed).toBe(false);
      attachments.workspaces[1]!.release();
      await closing;
      expect(closed).toBe(true);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await runtime.dispose();
      audits.close();
    }
  });

  it("marks a turn end and a later close in the session's history when its shell-write walk is still running", async () => {
    const root = await realpath(await temporaryDirectory());
    await writeFile(join(root, "a.py"), "before\n");
    const databasePath = join(root, "rerail.sqlite");
    const walk = Promise.withResolvers<void>();
    const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
    vi.mocked(opendir).mockImplementation(async (...args) => {
      if (args[0] === root) await walk.promise;
      return actual.opendir(...args);
    });
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      discoveryWaitMs: 20,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    const reader = new SqliteSessionReader(databasePath);
    const session = { host: "omp" as const, sessionId: "partial", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      expect(await runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "" })).toEqual([]);
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });
      expect(reader.events(session.sessionId).filter(event => event.kind === "source-discovery-pending").map(event => event.payload))
        .toEqual([{ boundary: "turn" }, { boundary: "close" }]);
    } finally {
      walk.resolve();
      await runtime.dispose();
      reader.close();
      vi.mocked(opendir).mockRestore();
    }
  });

  it("passes how long the session's code-rule evaluator took to open to the timing log", async () => {
    const lines: string[] = [];
    const test = await fixture({ timingLog: line => lines.push(line) });
    try {
      expect(lines).toEqual([
        expect.stringMatching(/^rerail: code-rule evaluator of session session-1 opened in \d+ ms\n$/u),
      ]);
    } finally {
      await test.close();
    }
  });

  describe("a turn end's wait for the turn's own work", () => {
    /** The turn end's settle work takes `settleMilliseconds` of the (faked) clock; the rest of the runtime keeps real time. */
    async function turnEndLines(settleMilliseconds: number): Promise<string[]> {
      const root = await realpath(await temporaryDirectory());
      const settling = Promise.withResolvers<void>();
      const release = Promise.withResolvers<void>();
      const lines: string[] = [];
      const runtime = new RerailRuntime({
        storageRoot: join(root, "sessions"), databasePath: join(root, "rerail.sqlite"), workspaceRoot: root, autostart: false, embedder: testEmbedder,
        timingLog: line => lines.push(line),
        createAuditor: () => ({
          openWorkspace: async () => undefined,
          acceptsPath: () => true,
          auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
          settleGeneratedEdits: async () => { settling.resolve(); await release.promise; },
          close: async () => undefined,
        }),
      });
      const session = { host: "omp" as const, sessionId: "slow-turn", cwd: root };
      try {
        await runtime.ingest({ kind: "session-start", eventId: "start", session });
        await runtime.codeRulesOpened(session.sessionId);
        vi.useFakeTimers({ toFake: ["performance"] });
        const turnEnd = runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "" });
        await settling.promise;
        vi.advanceTimersByTime(settleMilliseconds);
        release.resolve();
        await turnEnd;
        return lines.filter(line => line.includes("turn end"));
      } finally {
        release.resolve();
        vi.useRealTimers();
        await runtime.dispose();
      }
    }

    it("is written to the daemon log with how long it took and on what, once it exceeds a second", async () => {
      expect(await turnEndLines(1_100)).toEqual([
        expect.stringMatching(/^rerail: turn end of session slow-turn waited \d+ ms for generated-edit audits\n$/u),
      ]);
    });

    it("writes nothing when it is fast", async () => {
      expect(await turnEndLines(0)).toEqual([]);
    });
  });

  it("answers a turn end and closes the session when recording that its shell-write walk is still running fails", async () => {
    const root = await realpath(await temporaryDirectory());
    await writeFile(join(root, "a.py"), "before\n");
    const databasePath = join(root, "rerail.sqlite");
    const walk = Promise.withResolvers<void>();
    const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
    vi.mocked(opendir).mockImplementation(async (...args) => {
      if (args[0] === root) await walk.promise;
      return actual.opendir(...args);
    });
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const recordDiscoveryPending = vi.spyOn(SqliteSessionStore.prototype, "recordDiscoveryPending").mockImplementation(() => {
      throw new Error("database is locked");
    });
    const close = vi.fn(async () => undefined);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      discoveryWaitMs: 20,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close,
      }),
    });
    const store = new SqliteSessionStore(databasePath);
    const session = { host: "omp" as const, sessionId: "unrecorded-partial", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      await expect(runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "" })).resolves.toEqual([]);
      await expect(runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId })).resolves.toEqual([]);
      expect(recordDiscoveryPending.mock.calls.map(([, boundary]) => boundary)).toEqual(["turn", "close"]);
      expect(close).toHaveBeenCalledTimes(1);
      expect(store.session(session.sessionId)).toMatchObject({ endedAt: expect.any(Number) });
    } finally {
      walk.resolve();
      await runtime.dispose();
      store.close();
      recordDiscoveryPending.mockRestore();
      stderr.mockRestore();
      vi.mocked(opendir).mockRestore();
    }
  });

  it("starts a session before its evaluator opens, and answers a tool call at once without code rules", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    const attachments = gateStorageAttachments();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
    });
    const session = { host: "omp" as const, sessionId: "early", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      // The session has started; its evaluator is still waiting to attach storage.
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(1));
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: session.sessionId, text: "edit it" })).toEqual([]);
      // The call does not wait for the evaluator, so its edit is not checked when its result arrives either.
      expect(await runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "edited.py" },
      })).toEqual([]);
      attachments.workspaces[0]!.release();
      await runtime.codeRulesOpened(session.sessionId);
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      expect(await runtime.ingest({
        kind: "result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      })).toEqual([]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await runtime.dispose();
    }
  });

  it("closes a session without waiting for its evaluator to open, and the evaluator releases itself once it has", async () => {
    const root = await realpath(await temporaryDirectory());
    const opening = Promise.withResolvers<void>();
    const entered = Promise.withResolvers<void>();
    const close = vi.fn(async () => undefined);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath: join(root, "rerail.sqlite"), workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: () => ({
        openWorkspace: async () => { entered.resolve(); await opening.promise; },
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close,
      }),
    });
    const store = new SqliteSessionStore(join(root, "rerail.sqlite"));
    const session = { host: "omp" as const, sessionId: "closing-while-opening", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await entered.promise;
      let closed = false;
      const closing = runtime.closeSession(session.sessionId).then(() => { closed = true; });
      // The evaluator is still held in `openWorkspace`: closing resolves without it.
      await vi.waitFor(() => expect(closed).toBe(true));
      await closing;
      expect(store.session(session.sessionId)).toMatchObject({ endedAt: expect.any(Number) });
      expect(close).not.toHaveBeenCalled();
      opening.resolve();
      await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    } finally {
      opening.resolve();
      await runtime.dispose();
      store.close();
    }
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("leaves the audits of a reopened session alone when the evaluator it replaced is released late", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    const attachments = gateStorageAttachments();
    const evaluators: NativeCodeRuleEnforcer[] = [];
    const firstClosed = Promise.withResolvers<void>();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => {
        const evaluator = new NativeCodeRuleEnforcer(needleCatalog(), options);
        if (evaluators.length === 0) {
          const close = evaluator.close.bind(evaluator);
          evaluator.close = async (...args) => { await close(...args); firstClosed.resolve(); };
        }
        evaluators.push(evaluator);
        return evaluator;
      },
    });
    const audits = new SqliteAuditStore(databasePath);
    const session = { host: "omp" as const, sessionId: "reopened", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start-1", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(1));
      // The first evaluator is still held in its storage attach: the session closes and its id starts again.
      await runtime.closeSession(session.sessionId);
      await runtime.ingest({ kind: "session-start", eventId: "start-2", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(2));
      attachments.workspaces[1]!.release();
      // The second session's evaluator has opened and its repository audit waits for the gate.
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(3));
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([expect.objectContaining({ scope: "repository", status: "running" })]);
      attachments.workspaces[0]!.release();
      await firstClosed.promise;
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([expect.objectContaining({ scope: "repository", status: "running" })]);
      attachments.workspaces[2]!.release();
      await vi.waitFor(() => expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([
        expect.objectContaining({ scope: "repository", status: "completed" }),
      ]));
      await runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "edited.py" },
      });
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      expect(await runtime.ingest({
        kind: "result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      })).toEqual([expect.objectContaining({ content: expect.stringContaining("- edited.py:1:1 — No needle [") })]);
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual(expect.arrayContaining([
        expect.objectContaining({ scope: "ai-output", status: "completed", paths: ["edited.py"] }),
      ]));
      expect(audits.list({ sessionId: session.sessionId }).audits.filter(audit => audit.status === "interrupted")).toEqual([]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await runtime.dispose();
      audits.close();
    }
  });

  it("leaves the audits of a session another runtime reopened alone when the evaluator the closed runtime left opening is released late", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    const attachments = gateStorageAttachments();
    const firstClosed = Promise.withResolvers<void>();
    let created = 0;
    const runtimeOptions = {
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: (options: ConstructorParameters<typeof NativeCodeRuleEnforcer>[1]) => {
        const evaluator = new NativeCodeRuleEnforcer(needleCatalog(), options);
        if (created++ === 0) {
          const close = evaluator.close.bind(evaluator);
          evaluator.close = async (...args) => { await close(...args); firstClosed.resolve(); };
        }
        return evaluator;
      },
    };
    // The daemon makes one runtime per initialize: the second owns the session id once the first has closed it.
    const first = new RerailRuntime(runtimeOptions);
    const second = new RerailRuntime(runtimeOptions);
    const audits = new SqliteAuditStore(databasePath);
    const session = { host: "omp" as const, sessionId: "reopened-elsewhere", cwd: root };
    try {
      await first.ingest({ kind: "session-start", eventId: "start-1", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(1));
      // The first evaluator is still held in its storage attach; its runtime closes the session without waiting for it.
      await first.closeSession(session.sessionId);
      await second.ingest({ kind: "session-start", eventId: "start-2", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(2));
      attachments.workspaces[1]!.release();
      // The second runtime's evaluator has opened and its repository audit waits for the gate.
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(3));
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([expect.objectContaining({ scope: "repository", status: "running" })]);
      attachments.workspaces[0]!.release();
      await firstClosed.promise;
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([expect.objectContaining({ scope: "repository", status: "running" })]);
      attachments.workspaces[2]!.release();
      await vi.waitFor(() => expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([
        expect.objectContaining({ scope: "repository", status: "completed" }),
      ]));
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await first.dispose();
      await second.dispose();
      audits.close();
    }
  });

  describe("a code-rule boundary reached while the evaluator is still opening (indexing)", () => {
    /** A runtime whose first evaluator is held in its storage attach. */
    async function openingFixture() {
      const root = await realpath(await temporaryDirectory());
      const databasePath = join(root, "rerail.sqlite");
      const attachments = gateStorageAttachments();
      const runtime = new RerailRuntime({
        storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
        createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
      });
      const audits = new SqliteAuditStore(databasePath);
      const reader = new SqliteSessionReader(databasePath);
      const session = { host: "omp" as const, sessionId: "slow-open", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await vi.waitFor(() => expect(attachments.workspaces).toHaveLength(1));
      const call = (toolCallId: string) => runtime.ingest({
        kind: "call", eventId: `call-${toolCallId}`, sessionId: session.sessionId, toolCallId, toolName: "write", input: { path: "edited.py" },
      });
      const result = (toolCallId: string) => runtime.ingest({
        kind: "result", eventId: `result-${toolCallId}`, sessionId: session.sessionId, toolCallId, toolName: "write", output: "written",
      });
      /** The steers the next event delivers, waiting for the audit that produces them. */
      const steers = async () => {
        let delivered: readonly AdapterDelivery[] = [];
        await vi.waitFor(async () => {
          delivered = await runtime.ingest({ kind: "user-prompt", eventId: `poll-${Math.random()}`, sessionId: session.sessionId, text: "continue" });
          expect(delivered).not.toEqual([]);
        }, { timeout: 15_000 });
        return delivered;
      };
      const close = async () => {
        for (const workspace of attachments.workspaces) workspace.release();
        attachments.restore();
        await runtime.dispose();
        audits.close();
        reader.close();
      };
      return { root, session, attachments, audits, reader, call, result, steers, runtime, close };
    }

    it("is answered at once without code rules and recorded in the session's history", async () => {
      const test = await openingFixture();
      try {
        // The evaluator never opens in this stretch: each boundary answers on its own, without waiting for it.
        expect(await test.call("write")).toEqual([]);
        await writeFile(join(test.root, "edited.py"), "NEEDLE\n");
        expect(await test.result("write")).toEqual([]);
        expect(await test.runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: test.session.sessionId, text: "done" })).toEqual([]);
        expect(test.reader.events(test.session.sessionId).filter(event => event.kind === "code-rules-pending").map(event => event.payload))
          .toEqual([{ boundary: "call" }, { boundary: "result" }, { boundary: "turn-end" }]);
        expect(test.audits.list({ sessionId: test.session.sessionId }).audits).toEqual([]);
        test.attachments.workspaces[0]!.release();
        // Opened: the call it answered is not begun or finished late. The evaluator's first walk still sees the file
        // changed while it ran, as it sees any change it did not watch being made, and checks it.
        expect(await test.steers()).toEqual([expect.objectContaining({ content: expect.stringContaining("- edited.py:1:1 — No needle [") })]);

      } finally { await test.close(); }
    });

    it("does not check at its result the edit of a call it answered while indexing, even once the evaluator has opened", async () => {
      const test = await openingFixture();
      try {
        expect(await test.call("write")).toEqual([]);
        test.attachments.workspaces[0]!.release();
        await test.runtime.codeRulesOpened(test.session.sessionId);
        await writeFile(join(test.root, "edited.py"), "NEEDLE\n");
        expect(await test.result("write")).toEqual([]);
      } finally { await test.close(); }
    });
  });


  it("keeps a session whose evaluator cannot open, and records why its code rules are off", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockRejectedValueOnce(new Error("Workspace unavailable"));
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
    });
    const audits = new SqliteAuditStore(databasePath);
    const session = { host: "omp" as const, sessionId: "unopened", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "edited.py" },
      });
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      expect(await runtime.ingest({
        kind: "result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      })).toEqual([]);
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual([
        expect.objectContaining({
          status: "failed",
          failures: [expect.objectContaining({ clauseId: "catalog", code: "adapter-error", message: "Workspace unavailable" })],
        }),
      ]);
    } finally {
      await runtime.dispose();
      audits.close();
      vi.restoreAllMocks();
    }
  });

  it("closes a session whose evaluator cannot open even when recording why fails", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const start = vi.spyOn(SqliteAuditStore.prototype, "start").mockImplementation(() => {
      throw new Error("database is locked");
    });
    const unhandled: unknown[] = [];
    const onUnhandled = (reason: unknown): void => { unhandled.push(reason); };
    process.on("unhandledRejection", onUnhandled);
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createAuditor: () => ({
        openWorkspace: async () => { throw new Error("Workspace unavailable"); },
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    const store = new SqliteSessionStore(databasePath);
    const session = { host: "omp" as const, sessionId: "unrecorded-failure", cwd: root };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      // A turn boundary waits for the evaluator to have opened or failed.
      await expect(runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "" })).resolves.toEqual([]);
      await expect(runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId })).resolves.toEqual([]);
      expect(start).toHaveBeenCalled();
      expect(store.session(session.sessionId)).toMatchObject({ endedAt: expect.any(Number) });
      // The session is gone from the runtime: a later event for it no longer finds it.
      await expect(runtime.ingest({ kind: "user-prompt", eventId: "late", sessionId: session.sessionId, text: "late" })).rejects.toThrow();
      // A rejection is reported to the handler only once a macrotask has passed.
      await new Promise<void>(resolve => setImmediate(resolve));
      expect(unhandled).toEqual([]);
      expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recording failed: database is locked"));
    } finally {
      process.off("unhandledRejection", onUnhandled);
      await runtime.dispose();
      store.close();
      start.mockRestore();
      stderr.mockRestore();
    }
  });

  it("keeps a buffered violation across an unchanged turn checkpoint", async () => {
    const test = await fixture();
    try {
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Use the query plan", {
        revision: 0, scope: ["route.py"], background: true,
      });
      const deliveries = await test.runtime.ingest({
        kind: "turn-end", eventId: "turn", sessionId: test.session.sessionId, text: "done",
      });
      expect(deliveries).toEqual([expect.objectContaining({ content: "Use the query plan" })]);
      expect(await test.poll()).toEqual([]);
    } finally { await test.close(); }
  });

  it("hands a polling host no steer while the repository is stopped, and the held steer once restarted", async () => {
    const test = await fixture({ startGate: true });
    try {
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Use the query plan", {
        revision: 0, scope: ["route.py"], background: true,
      });
      stopTestRepository(test.root, test.databasePath);
      expect(await test.runtime.collectDeliveries(test.session.sessionId)).toEqual([]);
      startTestRepository(test.root, test.databasePath);
      expect(await test.runtime.collectDeliveries(test.session.sessionId)).toEqual([
        expect.objectContaining({ content: "Use the query plan" }),
      ]);
    } finally { await test.close(); }
  });

  it("pushes nothing when the repository is stopped while the delivery is being recorded", async () => {
    const pushed: AdapterDelivery[] = [];
    const test = await fixture({ host: "omp", startGate: true, deliverBackground: async delivery => { pushed.push(delivery); } });
    const record = test.runtime.persistDelivery.bind(test.runtime);
    const recorded = Promise.withResolvers<void>();
    test.runtime.persistDelivery = async delivery => {
      await record(delivery);
      stopTestRepository(test.root, test.databasePath);
      recorded.resolve();
    };
    try {
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Use the query plan", {
        revision: 0, scope: ["route.py"], background: true,
      });
      await recorded.promise;
      // The steer follows the ledger write within the same task chain; one macrotask lets it run.
      const handedOff = Promise.withResolvers<void>();
      setImmediate(handedOff.resolve);
      await handedOff.promise;
      expect(pushed).toEqual([]);
    } finally { await test.close(); }
  });

  it("invalidates only results depending on a changed source", async () => {
    const test = await fixture();
    try {
      await test.runtime.ingest({
        kind: "call", eventId: "call", sessionId: test.session.sessionId,
        toolCallId: "write-b", toolName: "write", input: { path: "b.py" },
      });
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Finding in A", { revision: 0, scope: ["a.py"] });
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Old finding in B", { revision: 0, scope: ["b.py"] });
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Old cross-file finding", { revision: 0, scope: ["a.py", "b.py"] });
      await writeFile(join(test.root, "b.py"), "value = 1\n");
      const deliveries = await test.runtime.ingest({
        kind: "result", eventId: "result", sessionId: test.session.sessionId,
        toolCallId: "write-b", toolName: "write", output: "written",
      });
      expect(deliveries).toEqual([expect.objectContaining({ content: "Finding in A" })]);
      await expect(test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Late stale B", {
        revision: 0, scope: ["b.py"],
      })).resolves.toBe("discarded");
      expect(await test.poll()).toEqual([]);
    } finally { await test.close(); }
  });

  it("does not block an event or duplicate delivery while the native host is receiving it", async () => {
    let release: (() => void) | undefined;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const accepted: AdapterDelivery[] = [];
    const test = await fixture({ deliverBackground: async delivery => { await gate; accepted.push(delivery); } });
    try {
      await expect(test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Queued finding", {
        revision: 0, scope: ["route.py"], background: true,
      })).resolves.toBe("queued");
      expect(accepted).toEqual([]);
      expect(await test.poll()).toEqual([]);
      release!();
      await expect.poll(() => accepted).toEqual([expect.objectContaining({ content: "Queued finding" })]);
      expect(await test.poll()).toEqual([]);
    } finally { release?.(); await test.close(); }
  });

  it("holds a native generated-edit boundary until its background steer completes", async () => {
    const steerStarted = Promise.withResolvers<void>();
    const releaseSteer = Promise.withResolvers<void>();
    const accepted: AdapterDelivery[] = [];
    const test = await fixture({
      host: "pi",
      deliverBackground: async delivery => {
        steerStarted.resolve();
        await releaseSteer.promise;
        accepted.push(delivery);
      },
      auditGeneratedEdits: steerDelivery => async (sessionId, batch) => {
        await steerDelivery.requestSteer(sessionId, "steer", `Generated revision ${batch.revision}`, {
          revision: batch.revision,
          scope: batch.edits.map(edit => edit.path),
          background: true,
          generatedEdit: { violations: [], sourceOf: () => undefined },
        });
        return { revision: batch.revision, disposition: "steer", violations: [], failures: [] };
      },
    });
    try {
      await test.runtime.ingest({
        kind: "call", eventId: "write", sessionId: test.session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "route.py" },
      });
      await writeFile(join(test.root, "route.py"), "generated()\n");
      let resultCompleted = false;
      const result = test.runtime.ingest({
        kind: "result", eventId: "result", sessionId: test.session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      }).then(deliveries => {
        resultCompleted = true;
        return deliveries;
      });
      await steerStarted.promise;
      expect(resultCompleted).toBe(false);
      releaseSteer.resolve();
      expect(await result).toEqual([]);
      expect(accepted).toEqual([expect.objectContaining({ content: "Generated revision 1" })]);
    } finally {
      releaseSteer.resolve();
      await test.close();
    }
  });

  it("does not introduce code-rule-specific retries when the native host rejects delivery", async () => {
    const errorOutput = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    let attempts = 0;
    const test = await fixture({ deliverBackground: async () => { attempts += 1; throw new Error("host rejected handoff"); } });
    try {
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Finding", {
        revision: 0, scope: ["route.py"], background: true,
      });
      await expect.poll(() => attempts).toBe(1);
      expect(await test.poll()).toEqual([]);
      expect(await test.poll()).toEqual([]);
      expect(attempts).toBe(1);
    } finally { await test.close(); errorOutput.mockRestore(); }
  });

  it("delivers the code rule violation for a shell-written source at that command's boundary", async () => {
    const batches: unknown[] = [];
    const test = await fixture({
      host: "codex",
      prepare: root => writeFile(join(root, "route.py"), "transaction.fetch_value('SELECT 1')\n"),
      auditGeneratedEdits: steerDelivery => async (sessionId, batch) => {
        batches.push(batch);
        const violation = batch.edits.some(edit => edit.generatedText?.includes("cursor.execute"));
        if (violation) await steerDelivery.requestSteer(sessionId, "steer", `CURSOR_EXECUTE revision ${batch.revision}`, {
          revision: batch.revision, scope: batch.edits.map(edit => edit.path),
        });
        return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
      },
    });
    try {
      const sessionId = test.session.sessionId;
      // The evaluator opens behind the session start, and its baseline walk behind that: a turn checkpoint waits for
      // both, so the walk has read route.py before the shell rewrites it.
      expect(await test.runtime.ingest({ kind: "turn-end", eventId: "baseline", sessionId, text: "" })).toEqual([]);
      // The agent rewrites the file from a shell, not an edit tool, so no preimage is captured.
      await test.runtime.ingest({
        kind: "call", eventId: "shell-write", sessionId, toolCallId: "shell-1", toolName: "shell",
        input: { command: ["bash", "-lc", "python3 -c \"...p.write_text(...)\""] },
      });
      await writeFile(join(test.root, "route.py"), "cursor.execute('SELECT 1')\n");
      const writeBoundary = await test.runtime.ingest({
        kind: "result", eventId: "shell-write-result", sessionId, toolCallId: "shell-1", toolName: "shell", output: "",
      });
      expect(batches).toEqual([{
        revision: 1,
        checkpoints: ["edit", "function"],
        edits: [{
          path: "route.py", language: "python",
          previousReadError: expect.any(String), generatedText: "cursor.execute('SELECT 1')\n",
        }],
      }]);
      expect(writeBoundary).toEqual([expect.objectContaining({ content: "CURSOR_EXECUTE revision 1" })]);

      // The follow-up read-only command sees nothing new and no duplicate delivery.
      await test.runtime.ingest({
        kind: "call", eventId: "shell-read", sessionId, toolCallId: "shell-2", toolName: "shell",
        input: { command: ["bash", "-lc", "sed -n '1,200p' route.py"] },
      });
      const readBoundary = await test.runtime.ingest({
        kind: "result", eventId: "shell-read-result", sessionId, toolCallId: "shell-2", toolName: "shell", output: "",
      });
      expect(batches).toHaveLength(1);
      expect(readBoundary).toEqual([]);
      expect(await test.runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId, text: "done" })).toEqual([]);
      expect(batches).toHaveLength(1);
    } finally { await test.close(); }
  });

  it("discards buffered and late results after session closure", async () => {
    const test = await fixture();
    try {
      await test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Buffered finding", { revision: 0, scope: ["route.py"] });
      await test.runtime.closeSession(test.session.sessionId);
      await expect(test.steerDelivery.requestSteer(test.session.sessionId, "steer", "Late finding", {
        revision: 0, scope: ["route.py"],
      })).resolves.toBe("discarded");
    } finally { await test.close(); }
  });
  it("opens independent agent sessions concurrently", async () => {
    const root = await temporaryDirectory();
    const opened: string[] = [];
    const gate = Promise.withResolvers<void>();
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      workspaceRoot: root,
      autostart: false,
      embedder: testEmbedder,
      createAuditor: () => ({
        openWorkspace: async sessionId => {
          opened.push(sessionId);
          if (sessionId === "blocked") await gate.promise;
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
    const blocked = { host: "claude-code" as const, sessionId: "blocked", cwd: root };
    const independent = { host: "claude-code" as const, sessionId: "independent", cwd: root };

    const blockedStart = runtime.ingest({ kind: "session-start", eventId: "blocked-start", session: blocked });
    const starts = [blockedStart];
    try {
      await vi.waitFor(() => expect(opened).toEqual(["blocked"]));
      const independentStart = runtime.ingest({ kind: "session-start", eventId: "independent-start", session: independent });
      starts.push(independentStart);
      await vi.waitFor(() => expect(opened).toEqual(["blocked", "independent"]));
      await independentStart;
    } finally {
      gate.resolve();
      await Promise.allSettled(starts);
      await runtime.dispose();
    }
  });
});
