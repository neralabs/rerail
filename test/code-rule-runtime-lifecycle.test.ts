import { mkdir, opendir, realpath, writeFile } from "node:fs/promises";
import type * as FsPromises from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, RerailHost } from "../src/adapter-contract.js";
import type { CodeRuleAuditor, RuleSteerDelivery } from "../src/code-rule.js";
import { NativeCodeRuleEnforcer } from "../src/code-rule-enforcer.js";
import { SqliteAuditStore } from "../src/audit-store.js";
import { codeRuleSchema } from "../src/code-rule-schema.js";
import { RerailRuntime } from "../src/runtime.js";
import { temporaryDirectories, testEmbedder } from "./hook-fixtures.js";
import { testCodeRuleCatalog } from "./support/repository-rules.js";
import { startTestRepository, stopTestRepository } from "./support/started-repository.js";
import { gateStorageAttachments } from "./storage-attach-gate.js";

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
}
async function fixture({ deliverBackground, host = "claude-code", prepare, auditGeneratedEdits, startGate }: FixtureOptions = {}) {
  const root = await temporaryDirectory();
  await prepare?.(root);
  const databasePath = join(root, "rerail.sqlite");
  if (startGate === true) startTestRepository(root, databasePath);
  let steerDelivery: RuleSteerDelivery | undefined;
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root,
    embedder: testEmbedder, startGate: startGate === true,
    ...(deliverBackground === undefined ? {} : { deliverBackground }),
    createAuditor: options => {
      steerDelivery = options.steerDelivery;
      return {
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: auditGeneratedEdits?.(options.steerDelivery)
          ?? (async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] })),
        close: async () => undefined,
      };
    },
  });
  const session = { host, sessionId: "session-1", cwd: root };
  await runtime.ingest({ kind: "session-start", eventId: "start", session });
  if (steerDelivery === undefined) throw new Error("Policy delivery was not initialized");
  let event = 0;
  return {
    runtime, steerDelivery: steerDelivery, session, root, databasePath,
    poll: () => runtime.ingest({ kind: "user-prompt", eventId: `poll-${++event}`, sessionId: session.sessionId, text: "continue" }),
    close: () => runtime.dispose(),
  };
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
        expect.objectContaining({ scope: "touched-files", status: "completed" }),
        expect.objectContaining({ scope: "repository", status: "running" }),
      ])));
      expect(editDeliveries).toEqual([expect.objectContaining({ content: expect.stringContaining("No needle (edited.py:") })]);
      expect(await runtime.ingest({
        kind: "user-prompt", eventId: "poll", sessionId: session.sessionId, text: "continue",
      })).toEqual([]);
      await writeFile(join(root, "shell.py"), "safe()\n");
      gate.resolve();
      await enforcer!.settle();
      expect(audits.list({ sessionId: session.sessionId }).audits).toEqual(expect.arrayContaining([
        expect.objectContaining({
          scope: "repository", status: "failed",
          failures: expect.arrayContaining([expect.objectContaining({ path: "broken.py", code: "source-incomplete" })]),
        }),
      ]));
      // A late baseline cannot overwrite the edit observation and replay it at the next checkpoint.
      expect(await runtime.ingest({
        kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "done",
      })).toEqual([]);
      expect(audits.list({ sessionId: session.sessionId }).audits.filter(audit => audit.scope === "touched-files").flatMap(audit => audit.paths).sort())
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
          generatedEdit: true,
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
