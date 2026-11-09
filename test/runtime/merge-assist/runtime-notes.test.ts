import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { RerailRuntime } from "../../../src/runtime/runtime.js";
import type { AdapterDelivery } from "../../../src/engine/index.js";
import type { JudgeOutcome, JudgeTask } from "../../../src/runtime/merge-assist/agent-judge.js";
import type { MergeAssistRuntime } from "../../../src/runtime/merge-assist/service.js";
import { ownRuleId } from "../../../src/runtime/merge-assist/activation.js";
import {
  candidate,
  candidateFiles,
  captureVerdict,
  commit,
  compareAnswer,
  FakeJudge,
  mergeAssistWorkspaces,
  ORDERS_CLEAN,
  ORDERS_RETRY,
  writeFiles,
  type Workspace,
} from "./support/merge-assist-workspace.js";

const workspace = mergeAssistWorkspaces();

const CORRECTION = "No, don't wrap API calls in retry loops; retries belong in the transport.";

/** A session runtime over the workspace's database with Merge Assist `service`, and an evaluator that finds nothing. */
function sessionRuntime(ws: Workspace, service: MergeAssistRuntime): RerailRuntime {
  return new RerailRuntime({
    autostart: false,
    workspaceRoot: ws.repo,
    storageRoot: join(ws.folder, "sessions"),
    databasePath: ws.databasePath,
    mergeAssist: service,
    embedder: {
      fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
      embed: async texts => texts.map(() => new Float32Array([1, 0])),
    },
    createAuditor: () => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
      close: async () => undefined,
    }),
  });
}

describe("Merge Assist in a session", { timeout: 30_000 }, () => {
  it("captures a correction after the turn ends and gives the agent the saved decision at the next boundary, once, though edits came between", async () => {
    const ws = await workspace();
    const release = Promise.withResolvers<void>();
    const judge = new class extends FakeJudge {
      override async judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> {
        await release.promise;
        return await super.judge(task);
      }
    }(() => captureVerdict());
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    // Settles when the runtime queues a note for the session: capture and apply are done by then.
    const noted = Promise.withResolvers<void>();
    const pendingNotes = service.pendingNotes.bind(service);
    service.pendingNotes = (sessionId, checkoutPath, inFlight) => {
      const notes = pendingNotes(sessionId, checkoutPath, inFlight);
      if (notes.length > 0) noted.resolve();
      return notes;
    };
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "merge-assist-session", cwd: ws.repo };
    const { sessionId } = session;
    const orders = join(ws.repo, "src/api/orders.ts");
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.ingest({ kind: "user-prompt", eventId: "task", sessionId, text: "Add an orders endpoint." });
      await runtime.ingest({ kind: "call", eventId: "write", sessionId, toolCallId: "1", toolName: "Write", input: { file_path: orders, content: ORDERS_RETRY } });
      await runtime.ingest({ kind: "result", eventId: "written", sessionId, toolCallId: "1", toolName: "Write", output: "ok" });
      await runtime.ingest({ kind: "turn-end", eventId: "turn-1", sessionId, text: "Done." });
      await runtime.ingest({ kind: "user-prompt", eventId: "correction", sessionId, text: CORRECTION });
      await runtime.ingest({
        kind: "call", eventId: "edit", sessionId, toolCallId: "2", toolName: "Edit", input: { file_path: orders, old_string: ORDERS_RETRY, new_string: ORDERS_CLEAN },
      });
      await runtime.ingest({ kind: "result", eventId: "edited", sessionId, toolCallId: "2", toolName: "Edit", output: "ok" });
      // The turn end answers at once; the capture runs behind it, held in its judgment for now.
      expect(await runtime.ingest({ kind: "turn-end", eventId: "turn-2", sessionId, text: "Fixed." })).toEqual([]);

      // The agent goes on: a new prompt, and a write whose result comes after the note is queued. That result advances
      // the session's revision; the note speaks for no source, so it still goes.
      const deliveries = [...await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Now add pagination." })];
      const later = join(ws.repo, "src/api/later.ts");
      deliveries.push(...await runtime.ingest({ kind: "call", eventId: "write-2", sessionId, toolCallId: "3", toolName: "Write", input: { file_path: later, content: "export const later = 1;\n" } }));
      release.resolve();
      await noted.promise;
      const [saved] = (await service.status(ws.repo)).own;
      expect(saved).toMatchObject({ level: "code", source: { sessionId } });
      expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, saved!.id)]);
      writeFiles(ws.repo, { "src/api/later.ts": "export const later = 1;\n" });
      deliveries.push(...await runtime.ingest({ kind: "result", eventId: "written-2", sessionId, toolCallId: "3", toolName: "Write", output: "ok" }));
      deliveries.push(...await runtime.ingest({ kind: "user-prompt", eventId: "after-write", sessionId, text: "And sorting." }));

      const notes = deliveries.filter((delivery: AdapterDelivery) => delivery.content.includes("Rerail saved your correction"));
      expect(notes).toEqual([{
        sessionId,
        requested: "steer",
        mechanism: expect.any(String),
        content: "This note comes from Rerail, not the user.\n"
          + `Rerail saved your correction as a decision for this branch: "${saved!.decision}" It applies from now on. `
          + `Undo: Merge Assist page, or \`rerail merge-assist reject ${saved!.id}\`.\nTell the user so in one line.`,
        ruleIds: [],
        rules: [],
        strategy: "message",
      }]);
      expect(judge.count("capture")).toBe(1);
      const after = await runtime.ingest({ kind: "user-prompt", eventId: "later", sessionId, text: "Thanks." });
      expect(after.filter(delivery => delivery.content.includes("Rerail saved"))).toEqual([]);
      // Handed over once, it is recorded as given.
      expect(service.pendingNotes(sessionId, ws.repo)).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });

  it("closes a session without waiting for its scan", async () => {
    const ws = await workspace();
    writeFiles(ws.repo, candidateFiles(candidate({ id: "candidate:errors-are-thrown-7a8b9c", decision: "API errors are thrown as exceptions.", author: "tbelanger" })));
    commit(ws.worktree("jbarnes-files"), candidateFiles(candidate({ id: "candidate:name-files-by-resource-2c3d4e", decision: "Name API files after their resource." })));
    const release = Promise.withResolvers<void>();
    const judging = Promise.withResolvers<void>();
    const judge = new class extends FakeJudge {
      override async judge<T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> {
        judging.resolve();
        await release.promise;
        return await super.judge(task);
      }
    }(task => compareAnswer(task));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "scanning-session", cwd: ws.repo };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await judging.promise;
      // The scan is held in its judgment until the end of the test: closing returns now, or the test times out.
      await runtime.closeSession(session.sessionId);
      expect((await service.status(ws.repo)).scanning).toBe(true);
    } finally {
      release.resolve();
      await vi.waitFor(async () => expect((await service.status(ws.repo)).scanning).toBe(false), { timeout: 10_000 });
      await runtime.dispose();
    }
  });

  it("finds overlapping work after an edit, without holding the edit's boundary, and gives the agent its note with the person's line", async () => {
    const model = Array.from({ length: 30 }, (_, index) => `  line ${index + 1}`);
    const text = (line: number, value: string) => `${model.map((entry, index) => (index + 1 === line ? value : entry)).join("\n")}\n`;
    const ws = await workspace({ files: { "src/api/model.ts": text(0, "") } });
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, "  shared 10") }, "Share compiled matchers");
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const noted = Promise.withResolvers<void>();
    const pendingNotes = service.pendingNotes.bind(service);
    service.pendingNotes = (sessionId, checkoutPath, inFlight) => {
      const notes = pendingNotes(sessionId, checkoutPath, inFlight);
      if (notes.some(note => note.conflict !== undefined)) noted.resolve();
      return notes;
    };
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "overlap-session", cwd: ws.repo };
    const { sessionId } = session;
    const file = join(ws.repo, "src/api/model.ts");
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await vi.waitFor(async () => expect((await service.status(ws.repo)).lastScan).not.toBeNull(), { timeout: 10_000 });
      expect((await service.status(ws.repo)).conflicts).toEqual([]);
      await runtime.ingest({ kind: "call", eventId: "edit", sessionId, toolCallId: "1", toolName: "Write", input: { file_path: file, content: text(12, "  cloned 12") } });
      writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned 12") });
      const deliveries = [...await runtime.ingest({ kind: "result", eventId: "edited", sessionId, toolCallId: "1", toolName: "Write", output: "ok" })];
      await noted.promise;
      deliveries.push(...await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." }));

      const [conflict] = (await service.status(ws.repo)).conflicts;
      expect(deliveries.filter(delivery => delivery.conflict !== undefined)).toEqual([expect.objectContaining({
        content: expect.stringContaining("Your edit to src/api/model.ts overlaps open work: jbarnes-model (Tim Belanger) Share compiled matchers."),
        conflict: { id: conflict!.id, text: "your edit overlaps jbarnes-model (Tim Belanger): model.ts" },
        rules: [],
      })]);
      expect(service.pendingNotes(sessionId, ws.repo)).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });

  it("gives a running session the decision a person settled a conflict with, at its very next boundary", async () => {
    const model = Array.from({ length: 30 }, (_, index) => `  line ${index + 1}`);
    const text = (line: number, value: string) => `${model.map((entry, index) => (index + 1 === line ? value : entry)).join("\n")}\n`;
    const ws = await workspace({ files: { "src/api/model.ts": text(0, "") } });
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, "  shared 10") }, "Share compiled matchers");
    writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned 12") });
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "running-session", cwd: ws.repo };
    const { sessionId } = session;
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await vi.waitFor(async () => expect((await service.status(ws.repo)).conflicts).toHaveLength(1), { timeout: 10_000 });
      // The overlap's note goes out at the next boundary; nothing else is waiting.
      const before = await runtime.ingest({ kind: "user-prompt", eventId: "first", sessionId, text: "Go on." });
      expect(before.map(delivery => delivery.content)).toEqual([expect.stringContaining("overlaps open work")]);

      // The person settles it elsewhere (the dashboard, the CLI): no scan or capture runs in the session.
      const [conflict] = (await service.status(ws.repo)).conflicts;
      await service.resolveConflict(ws.repo, conflict!.id, "theirs");
      const after = await runtime.ingest({ kind: "user-prompt", eventId: "second", sessionId, text: "And then?" });
      expect(after.map(delivery => delivery.content)).toEqual([[
        "This note comes from Rerail, not the user.",
        "The user settled a conflict with jbarnes-model with a decision for this branch: \"Share compiled matchers\". It applies from now on.",
        "Follow it where it applies to your work; your task is unchanged.",
      ].join("\n")]);
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "third", sessionId, text: "Thanks." })).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });
});
