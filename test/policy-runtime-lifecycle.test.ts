import { mkdir, opendir, realpath, writeFile } from "node:fs/promises";
import type * as FsPromises from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, RerailHost } from "../src/adapter-contract.js";
import type { ExecutablePolicyEvaluator, PolicySteeringDelivery } from "../src/executable-policy.js";
import { NativeExecutablePolicyEnforcer } from "../src/executable-policy-enforcer.js";
import { bootstrapPolicyDslCatalog } from "../src/policy-dsl-catalog.js";
import { SqlitePolicyExecutionStore } from "../src/policy-execution-store.js";
import { policyRuleSchema } from "../src/policy-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { RerailRuntime } from "../src/runtime.js";
import { temporaryDirectories, testEmbedder } from "./hook-fixtures.js";
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
  evaluateGeneratedEdits?(delivery: PolicySteeringDelivery): ExecutablePolicyEvaluator["evaluateGeneratedEdits"];
}
async function fixture({ deliverBackground, host = "claude-code", prepare, evaluateGeneratedEdits }: FixtureOptions = {}) {
  const root = await temporaryDirectory();
  await prepare?.(root);
  let delivery: PolicySteeringDelivery | undefined;
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath: join(root, "rerail.sqlite"), workspaceRoot: root,
    embedder: testEmbedder,
    ...(deliverBackground === undefined ? {} : { deliverBackground }),
    createPolicyEvaluator: options => {
      delivery = options.delivery;
      return {
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        evaluateGeneratedEdits: evaluateGeneratedEdits?.(options.delivery)
          ?? (async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", findings: [], failures: [] })),
        close: async () => undefined,
      };
    },
  });
  const session = { host, sessionId: "session-1", cwd: root };
  await runtime.ingest({ kind: "session-start", eventId: "start", session });
  if (delivery === undefined) throw new Error("Policy delivery was not initialized");
  let event = 0;
  return {
    runtime, delivery, session, root,
    poll: () => runtime.ingest({ kind: "user-prompt", eventId: `poll-${++event}`, sessionId: session.sessionId, text: "continue" }),
    close: () => runtime.dispose(),
  };
}

describe("source-scoped policy delivery", () => {
  it("starts and accepts edits while repository discovery is pending, then records scan errors and delivers fresh findings", async () => {
    const root = await realpath(await temporaryDirectory());
    await writeFile(join(root, "broken.py"), Buffer.from([0]));
    await writeFile(join(root, "edited.py"), "before\n");
    const databasePath = join(root, "rerail.sqlite");
    const rules = new SqliteRuleStore(databasePath);
    rules.upsertPolicy(policyRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:startup", revision: 1,
      scope: { include: ["**/*.py"], exclude: [] },
      checks: [{
        id: "needle",
        finding: { code: "NEEDLE", severity: "error", anchor: "witness.span", message: "No needle" },
        forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
      }],
    }));
    rules.close();
    const catalog = bootstrapPolicyDslCatalog({ databasePath });
    const gate = Promise.withResolvers<void>();
    const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
    vi.mocked(opendir).mockImplementation(async (...args) => {
      if (args[0] === root) await gate.promise;
      return actual.opendir(...args);
    });
    let enforcer: NativeExecutablePolicyEnforcer | undefined;
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath: databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createPolicyEvaluator: options => {
        enforcer = new NativeExecutablePolicyEnforcer(catalog, options);
        return enforcer;
      },
    });
    const runs = new SqlitePolicyExecutionStore(databasePath);
    const session = { host: "omp" as const, sessionId: "startup", cwd: root };
    let ready = false;
    const start = runtime.ingest({ kind: "session-start", eventId: "start", session }).then(result => {
      ready = true;
      return result;
    });
    try {
      await vi.waitFor(() => expect(ready).toBe(true));
      expect(enforcer!.snapshotStatus().pending).toBe(true);
      expect(runs.list({ sessionId: session.sessionId }).runs).toEqual([
        expect.objectContaining({ scope: "repository", status: "running" }),
      ]);
      await runtime.ingest({
        kind: "tool-call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "edited.py" },
      });
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      const editDeliveries = await runtime.ingest({
        kind: "tool-result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      });
      await vi.waitFor(() => expect(runs.list({ sessionId: session.sessionId }).runs).toEqual(expect.arrayContaining([
        expect.objectContaining({ scope: "touched-files", status: "completed" }),
        expect.objectContaining({ scope: "repository", status: "running" }),
      ])));
      expect(editDeliveries).toEqual([expect.objectContaining({ content: expect.stringContaining("NEEDLE") })]);
      expect(await runtime.ingest({
        kind: "user-prompt", eventId: "poll", sessionId: session.sessionId, text: "continue",
      })).toEqual([]);
      await writeFile(join(root, "shell.py"), "safe()\n");
      gate.resolve();
      await enforcer!.settle();
      expect(runs.list({ sessionId: session.sessionId }).runs).toEqual(expect.arrayContaining([
        expect.objectContaining({
          scope: "repository", status: "failed",
          failures: expect.arrayContaining([expect.objectContaining({ path: "broken.py", code: "source-incomplete" })]),
        }),
      ]));
      // A late baseline cannot overwrite the edit observation and replay it at the next checkpoint.
      expect(await runtime.ingest({
        kind: "turn-end", eventId: "turn", sessionId: session.sessionId, text: "done",
      })).toEqual([]);
      expect(runs.list({ sessionId: session.sessionId }).runs.filter(run => run.scope === "touched-files").flatMap(run => run.paths).sort())
        .toEqual(["edited.py", "shell.py"]);
    } finally {
      gate.resolve();
      await start;
      await runtime.dispose();
      runs.close();
      catalog.close();
      vi.mocked(opendir).mockRestore();
    }
  });

  it("evaluates an edit to a source that a rule refreshed at the tool call brings into scope", async () => {
    const root = await realpath(await temporaryDirectory());
    await mkdir(join(root, "web"));
    const databasePath = join(root, "rerail.sqlite");
    const needleIn = (revision: number, include: string) => policyRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:widened", revision,
      scope: { include: [include], exclude: [] },
      checks: [{
        id: "needle",
        finding: { code: "NEEDLE", severity: "error", anchor: "witness.span", message: "No needle" },
        forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
      }],
    });
    const rules = new SqliteRuleStore(databasePath);
    rules.upsertPolicy(needleIn(1, "api/**/*.py"));
    rules.close();
    const catalog = bootstrapPolicyDslCatalog({ databasePath });
    const attachments = gateStorageAttachments();
    attachments.gated = false;
    let toolResult = false;
    let enforcer: NativeExecutablePolicyEnforcer | undefined;
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
      createPolicyEvaluator: options => {
        enforcer = new NativeExecutablePolicyEnforcer(catalog, options);
        const accepts = enforcer.acceptsPath.bind(enforcer);
        // Storage for the refreshed rules attaches only once the tool result's edit has been filtered by scope.
        enforcer.acceptsPath = path => {
          const accepted = accepts(path);
          if (toolResult) for (const workspace of attachments.workspaces) workspace.release();
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
      const widened = new SqliteRuleStore(databasePath);
      widened.upsertPolicy(needleIn(2, "**/*.py"));
      widened.close();
      await runtime.ingest({
        kind: "tool-call", eventId: "call", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "web/new.py" },
      });
      await writeFile(join(root, "web", "new.py"), "NEEDLE\n");
      toolResult = true;
      const deliveries = await runtime.ingest({
        kind: "tool-result", eventId: "result", sessionId: session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      });
      expect(deliveries).toEqual([expect.objectContaining({ content: expect.stringContaining("web/new.py") })]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await runtime.dispose();
      catalog.close();
    }
  });

  it("keeps a buffered finding across an unchanged turn checkpoint", async () => {
    const test = await fixture();
    try {
      await test.delivery.requestDelivery(test.session.sessionId, "steer", "Use the query plan", {
        revision: 0, scope: ["route.py"], background: true,
      });
      const deliveries = await test.runtime.ingest({
        kind: "turn-end", eventId: "turn", sessionId: test.session.sessionId, text: "done",
      });
      expect(deliveries).toEqual([expect.objectContaining({ content: "Use the query plan" })]);
      expect(await test.poll()).toEqual([]);
    } finally { await test.close(); }
  });

  it("invalidates only results depending on a changed source", async () => {
    const test = await fixture();
    try {
      await test.runtime.ingest({
        kind: "tool-call", eventId: "call", sessionId: test.session.sessionId,
        toolCallId: "write-b", toolName: "write", input: { path: "b.py" },
      });
      await test.delivery.requestDelivery(test.session.sessionId, "steer", "Finding in A", { revision: 0, scope: ["a.py"] });
      await test.delivery.requestDelivery(test.session.sessionId, "steer", "Old finding in B", { revision: 0, scope: ["b.py"] });
      await test.delivery.requestDelivery(test.session.sessionId, "steer", "Old cross-file finding", { revision: 0, scope: ["a.py", "b.py"] });
      await writeFile(join(test.root, "b.py"), "value = 1\n");
      const deliveries = await test.runtime.ingest({
        kind: "tool-result", eventId: "result", sessionId: test.session.sessionId,
        toolCallId: "write-b", toolName: "write", output: "written",
      });
      expect(deliveries).toEqual([expect.objectContaining({ content: "Finding in A" })]);
      await expect(test.delivery.requestDelivery(test.session.sessionId, "steer", "Late stale B", {
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
      await expect(test.delivery.requestDelivery(test.session.sessionId, "steer", "Queued finding", {
        revision: 0, scope: ["route.py"], background: true,
      })).resolves.toBe("queued");
      expect(accepted).toEqual([]);
      expect(await test.poll()).toEqual([]);
      release!();
      await expect.poll(() => accepted).toEqual([expect.objectContaining({ content: "Queued finding" })]);
      expect(await test.poll()).toEqual([]);
    } finally { release?.(); await test.close(); }
  });

  it("holds a native generated-edit boundary until its background handoff completes", async () => {
    const handoffStarted = Promise.withResolvers<void>();
    const releaseHandoff = Promise.withResolvers<void>();
    const accepted: AdapterDelivery[] = [];
    const test = await fixture({
      host: "pi",
      deliverBackground: async delivery => {
        handoffStarted.resolve();
        await releaseHandoff.promise;
        accepted.push(delivery);
      },
      evaluateGeneratedEdits: delivery => async (sessionId, batch) => {
        await delivery.requestDelivery(sessionId, "steer", `Generated revision ${batch.revision}`, {
          revision: batch.revision,
          scope: batch.edits.map(edit => edit.path),
          background: true,
          generatedEdit: true,
        });
        return { revision: batch.revision, disposition: "resteer", findings: [], failures: [] };
      },
    });
    try {
      await test.runtime.ingest({
        kind: "tool-call", eventId: "write", sessionId: test.session.sessionId,
        toolCallId: "write", toolName: "write", input: { path: "route.py" },
      });
      await writeFile(join(test.root, "route.py"), "generated()\n");
      let toolResultCompleted = false;
      const toolResult = test.runtime.ingest({
        kind: "tool-result", eventId: "result", sessionId: test.session.sessionId,
        toolCallId: "write", toolName: "write", output: "written",
      }).then(deliveries => {
        toolResultCompleted = true;
        return deliveries;
      });
      await handoffStarted.promise;
      expect(toolResultCompleted).toBe(false);
      releaseHandoff.resolve();
      expect(await toolResult).toEqual([]);
      expect(accepted).toEqual([expect.objectContaining({ content: "Generated revision 1" })]);
    } finally {
      releaseHandoff.resolve();
      await test.close();
    }
  });

  it("does not introduce policy-specific retries when the native host rejects delivery", async () => {
    const errorOutput = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    let attempts = 0;
    const test = await fixture({ deliverBackground: async () => { attempts += 1; throw new Error("host rejected handoff"); } });
    try {
      await test.delivery.requestDelivery(test.session.sessionId, "steer", "Finding", {
        revision: 0, scope: ["route.py"], background: true,
      });
      await expect.poll(() => attempts).toBe(1);
      expect(await test.poll()).toEqual([]);
      expect(await test.poll()).toEqual([]);
      expect(attempts).toBe(1);
    } finally { await test.close(); errorOutput.mockRestore(); }
  });

  it("delivers the executable-policy diagnostic for a shell-written source at that command's boundary", async () => {
    const batches: unknown[] = [];
    const test = await fixture({
      host: "codex",
      prepare: root => writeFile(join(root, "route.py"), "transaction.fetch_value('SELECT 1')\n"),
      evaluateGeneratedEdits: delivery => async (sessionId, batch) => {
        batches.push(batch);
        const violation = batch.edits.some(edit => edit.generatedText?.includes("cursor.execute"));
        if (violation) await delivery.requestDelivery(sessionId, "steer", `CURSOR_EXECUTE revision ${batch.revision}`, {
          revision: batch.revision, scope: batch.edits.map(edit => edit.path),
        });
        return { revision: batch.revision, disposition: "continue", findings: [], failures: [] };
      },
    });
    try {
      const sessionId = test.session.sessionId;
      // The agent rewrites the file from a shell, not an edit tool, so no preimage is captured.
      await test.runtime.ingest({
        kind: "tool-call", eventId: "shell-write", sessionId, toolCallId: "shell-1", toolName: "shell",
        input: { command: ["bash", "-lc", "python3 -c \"...p.write_text(...)\""] },
      });
      await writeFile(join(test.root, "route.py"), "cursor.execute('SELECT 1')\n");
      const writeBoundary = await test.runtime.ingest({
        kind: "tool-result", eventId: "shell-write-result", sessionId, toolCallId: "shell-1", toolName: "shell", output: "",
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
        kind: "tool-call", eventId: "shell-read", sessionId, toolCallId: "shell-2", toolName: "shell",
        input: { command: ["bash", "-lc", "sed -n '1,200p' route.py"] },
      });
      const readBoundary = await test.runtime.ingest({
        kind: "tool-result", eventId: "shell-read-result", sessionId, toolCallId: "shell-2", toolName: "shell", output: "",
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
      await test.delivery.requestDelivery(test.session.sessionId, "steer", "Buffered finding", { revision: 0, scope: ["route.py"] });
      await test.runtime.closeSession(test.session.sessionId);
      await expect(test.delivery.requestDelivery(test.session.sessionId, "steer", "Late finding", {
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
      createPolicyEvaluator: () => ({
        openWorkspace: async sessionId => {
          opened.push(sessionId);
          if (sessionId === "blocked") await gate.promise;
        },
        acceptsPath: () => true,
        evaluateGeneratedEdits: async (_sessionId, batch) => ({
          revision: batch.revision,
          disposition: "continue" as const,
          findings: [],
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
