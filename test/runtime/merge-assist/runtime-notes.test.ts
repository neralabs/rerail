import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { PullRequestSummary } from "../../../src/runtime/merge-assist/github.js";
import { commit, writeFiles } from "../../support/git.js";
import { FakeGithub, mergeAssistWorkspaces } from "../../support/merge-assist/workspace.js";
import { sessionRuntime } from "./support/conflict-sessions.js";

const workspace = mergeAssistWorkspaces();

/** Their change to line 10 of the model, and 7 lines that build on it. */
const SHARED = ["  shared 10", ...Array.from({ length: 7 }, (_, index) => `  built ${index + 1}`)].join("\n");

describe("Merge Assist in a session", { timeout: 30_000 }, () => {
  it("closes a session without waiting for its scan", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const release = Promise.withResolvers<void>();
    const listing = Promise.withResolvers<void>();
    const github = new class extends FakeGithub {
      override async openPullRequests(): Promise<PullRequestSummary[]> {
        listing.resolve();
        await release.promise;
        return await super.openPullRequests();
      }
    }();
    const service = ws.service({ github });
    await service.setEnabled(ws.repo, true);
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "scanning-session", cwd: ws.repo };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await listing.promise;
      // The scan is held listing pull requests until the end of the test: closing returns now, or the test times out.
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
    // Their change to line 10 and the lines built on it, and this edit's line two below: an overlap.
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, SHARED) }, "Share compiled matchers");
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

  it("tells a running session nothing more of a conflict once the person dismisses it", async () => {
    const model = Array.from({ length: 30 }, (_, index) => `  line ${index + 1}`);
    const text = (line: number, value: string) => `${model.map((entry, index) => (index + 1 === line ? value : entry)).join("\n")}\n`;
    const ws = await workspace({ files: { "src/api/model.ts": text(0, "") } });
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, SHARED) }, "Share compiled matchers");
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

      // The person dismisses it elsewhere (the dashboard, the CLI); found again by a later edit, it stays dismissed.
      const [conflict] = (await service.status(ws.repo)).conflicts;
      await service.dismissConflict(ws.repo, conflict!.id);
      await service.settled();
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "second", sessionId, text: "And then?" })).toEqual([]);
      writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned twelve") });
      expect(await service.detectAfterEdit(ws.repo, sessionId, "claude-code")).toBe(true);
      expect((await service.status(ws.repo)).conflicts).toEqual([expect.objectContaining({ id: conflict!.id, status: "dismissed" })]);
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "third", sessionId, text: "Thanks." })).toEqual([]);
    } finally {
      await runtime.dispose();
    }
  });
});
