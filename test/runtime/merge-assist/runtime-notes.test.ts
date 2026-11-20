import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery } from "../../../src/engine/index.js";
import type { MergeAssistRuntime } from "../../../src/runtime/index.js";
import type { PullRequestListing } from "../../../src/runtime/merge-assist/github.js";
import type { SessionSuspension, SuspendedSessions } from "../../../src/runtime/runtime.js";
import { deliveryEntries } from "../../support/ledger/deliveries.js";
import { commit, writeFiles } from "../../support/merge-assist/git.js";
import { FakeGithub, mergeAssistWorkspaces } from "../../support/merge-assist/workspace.js";
import { sessionRuntime } from "./support/conflict-sessions.js";
import { withExperiments } from "../../support/environment/experiments.js";

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");

const workspace = mergeAssistWorkspaces();

/** Their change to line 10 of the model, and 7 lines that build on it. */
const SHARED = ["  shared 10", ...Array.from({ length: 7 }, (_, index) => `  built ${index + 1}`)].join("\n");

/**
 * The wait for the scan a session start asks Merge Assist for. The runtime calls `openSession` while the session start
 * is ingested and does not wait for it; the promise it returns settles when that scan has ended. The tests wait on that
 * promise rather than polling `status`, which starts Git several times synchronously: polled every 50 ms it holds the
 * event loop the scan runs on, and on a busy Windows machine the scan then outlasts the wait. The wait fails at once
 * when no session start called `openSession`, and with the scan's own error when the scan fails.
 */
function sessionScan(service: MergeAssistRuntime): () => Promise<void> {
  let scan: Promise<void> | undefined;
  const openSession = service.openSession.bind(service);
  service.openSession = (...args) => {
    scan = openSession(...args);
    return scan;
  };
  return async () => {
    if (scan === undefined) throw new Error("no session start asked Merge Assist for a scan");
    await scan;
  };
}

// A test here builds a repository with a worktree, starts a session runtime and runs Merge Assist's detection once or
// twice: on a 4-core Windows machine whose cores other tests keep busy, one can take over 30 s. The limit is
// conflict-delivery.test.ts's.
describe("Merge Assist in a session", { timeout: 60_000 }, () => {
  it("closes a session without waiting for its scan", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const release = Promise.withResolvers<void>();
    const listing = Promise.withResolvers<void>();
    const github = new class extends FakeGithub {
      override async openPullRequests(): Promise<PullRequestListing> {
        listing.resolve();
        await release.promise;
        return await super.openPullRequests();
      }
    }();
    const service = ws.service({ github });
    await service.setEnabled(ws.repo, true);
    const scanEnded = sessionScan(service);
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
      await scanEnded();
      expect((await service.status(ws.repo)).scanning).toBe(false);
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
    const scanEnded = sessionScan(service);
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "overlap-session", cwd: ws.repo };
    const { sessionId } = session;
    const file = join(ws.repo, "src/api/model.ts");
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await scanEnded();
      expect((await service.status(ws.repo)).lastScan).not.toBeNull();
      expect((await service.status(ws.repo)).conflicts).toEqual([]);
      await runtime.ingest({ kind: "call", eventId: "edit", sessionId, toolCallId: "1", toolName: "Write", input: { file_path: file, content: text(12, "  cloned 12") } });
      writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned 12") });
      const deliveries = [...await runtime.ingest({ kind: "result", eventId: "edited", sessionId, toolCallId: "1", toolName: "Write", output: "ok" })];
      await noted.promise;
      deliveries.push(...await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." }));

      const [conflict] = (await service.status(ws.repo)).conflicts;
      expect(deliveries.filter(delivery => delivery.conflict !== undefined)).toEqual([expect.objectContaining({
        content: expect.stringContaining("Your edit to src/api/model.ts overlaps open work: \"jbarnes-model\" (\"Tim Belanger\"), described on their side as \"Share compiled matchers\"."),
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
    const scanEnded = sessionScan(service);
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "running-session", cwd: ws.repo };
    const { sessionId } = session;
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await scanEnded();
      expect((await service.status(ws.repo)).conflicts).toHaveLength(1);
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

  it("gives a note whose answer no hook heard to the session's next boundary, recorded and marked given once", async () => {
    const model = Array.from({ length: 30 }, (_, index) => `  line ${index + 1}`);
    const text = (line: number, value: string) => `${model.map((entry, index) => (index + 1 === line ? value : entry)).join("\n")}\n`;
    const ws = await workspace({ files: { "src/api/model.ts": text(0, "") } });
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, SHARED) }, "Share compiled matchers");
    writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned 12") });
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    let marked = 0;
    const markNotesDelivered = service.markNotesDelivered.bind(service);
    service.markNotesDelivered = (...args) => {
      marked += 1;
      markNotesDelivered(...args);
    };
    const scanEnded = sessionScan(service);
    const runtime = sessionRuntime(ws, service);
    const session = { host: "claude-code" as const, sessionId: "unheard-session", cwd: ws.repo };
    const { sessionId } = session;
    const contents = (deliveries: readonly AdapterDelivery[]): string[] => deliveries.map(delivery => delivery.content);
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await scanEnded();
      const unheard = await runtime.ingest({ kind: "user-prompt", eventId: "unheard", sessionId, text: "Go on." });
      expect(contents(unheard)).toEqual([expect.stringContaining("overlaps open work")]);
      // Its hook gave up before the answer came: the engine service gives back what the answer took.
      await runtime.returnUndelivered(sessionId, unheard);
      expect(contents(await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." }))).toEqual(contents(unheard));
      expect(await runtime.ingest({ kind: "user-prompt", eventId: "after", sessionId, text: "Thanks." })).toEqual([]);
      expect({ recorded: await deliveryEntries(join(ws.folder, "sessions"), unheard[0]!.content), marked }).toEqual({ recorded: 1, marked: 1 });
    } finally {
      await runtime.dispose();
    }
  });

  it("lets a quiet session close while its note waits, and gives the note once the session opens again", async () => {
    const model = Array.from({ length: 30 }, (_, index) => `  line ${index + 1}`);
    const text = (line: number, value: string) => `${model.map((entry, index) => (index + 1 === line ? value : entry)).join("\n")}\n`;
    const ws = await workspace({ files: { "src/api/model.ts": text(0, "") } });
    commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, SHARED) }, "Share compiled matchers");
    writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned 12") });
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const queued = Promise.withResolvers<void>();
    const pendingNotes = service.pendingNotes.bind(service);
    service.pendingNotes = (...args) => {
      const notes = pendingNotes(...args);
      if (notes.length > 0) queued.resolve();
      return notes;
    };
    const scans: Array<Promise<void>> = [];
    const openSession = service.openSession.bind(service);
    service.openSession = (...args) => {
      const scan = openSession(...args);
      scans.push(scan);
      return scan;
    };
    // Keeps the session it lets go of while quiet, owner claim included, as the engine service does.
    const suspended = new Map<string, SessionSuspension>();
    const suspendedSessions: SuspendedSessions = {
      keep: suspension => void suspended.set(suspension.descriptor.sessionId, suspension),
      get: id => suspended.get(id),
      take: id => {
        const suspension = suspended.get(id);
        suspended.delete(id);
        return suspension;
      },
    };
    const runtime = sessionRuntime(ws, service, { suspendedSessions });
    const session = { host: "claude-code" as const, sessionId: "quiet-session", cwd: ws.repo };
    const { sessionId } = session;
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      // The scan's note waits for the session's next boundary, and the session sends nothing.
      await queued.promise;
      expect(await runtime.closeIdleSession(sessionId)).toMatchObject({ kind: "suspended" });
      const next = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, session, text: "Go on." });
      expect(next.map(delivery => delivery.content)).toEqual([expect.stringContaining("overlaps open work")]);
      // The session opened again scans its checkout again.
      await vi.waitFor(() => expect(scans).toHaveLength(2));
      await Promise.all(scans);
    } finally {
      await runtime.dispose();
    }
  });

  it("finds no conflicts at the late result of an edit its turn's end already settled", async () => {
    const ws = await workspace({ files: { "src/api/model.ts": "export {};\n" } });
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    let detections = 0;
    const detectAfterEdit = service.detectAfterEdit.bind(service);
    service.detectAfterEdit = (...args) => {
      detections += 1;
      return detectAfterEdit(...args);
    };
    const runtime = sessionRuntime(ws, service);
    const sessionId = "settled-session";
    const file = join(ws.repo, "src/api/model.ts");
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId, cwd: ws.repo } });
      await vi.waitFor(async () => expect((await service.status(ws.repo)).lastScan).not.toBeNull(), { timeout: 10_000 });
      await runtime.ingest({ kind: "call", eventId: "edit", sessionId, toolCallId: "1", toolName: "Write", input: { file_path: file, content: "export const a = 1;\n" } });
      // The person refused the write: the turn ends with no result, which settles the call.
      await runtime.ingest({ kind: "turn-end", eventId: "stop", sessionId, text: "" });
      await runtime.ingest({ kind: "result", eventId: "late", sessionId, toolCallId: "1", toolName: "Write", output: "ok" });
      expect(detections).toBe(0);
    } finally {
      await runtime.dispose();
    }
  });

  for (const withHookCommand of [false, true]) {
    it(`never rejects, nor fails the edit's result, when reading the notes fails after a detection${withHookCommand ? ", with a hook command set" : ""}`, async () => {
      const model = Array.from({ length: 30 }, (_, index) => `  line ${index + 1}`);
      const text = (line: number, value: string) => `${model.map((entry, index) => (index + 1 === line ? value : entry)).join("\n")}\n`;
      const ws = await workspace({ files: { "src/api/model.ts": text(0, "") } });
      commit(ws.worktree("jbarnes-model"), { "src/api/model.ts": text(10, SHARED) }, "Share compiled matchers");
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      if (withHookCommand) {
        await service.updateSettings(ws.repo, { hook: { command: [process.execPath, "-e", "process.stdin.resume()"], timeoutSeconds: 10 } });
      }
      // The database is busy whenever the session's notes are read.
      service.pendingNotes = () => {
        throw new Error("SQLITE_BUSY: database is locked");
      };
      const detections: Array<Promise<boolean>> = [];
      const detectAfterEdit = service.detectAfterEdit.bind(service);
      service.detectAfterEdit = (...args) => {
        const detection = detectAfterEdit(...args);
        detections.push(detection);
        return detection;
      };
      const unhandled: unknown[] = [];
      const onUnhandled = (reason: unknown): void => {
        unhandled.push(reason);
      };
      process.on("unhandledRejection", onUnhandled);
      const scanEnded = sessionScan(service);
      const runtime = sessionRuntime(ws, service, { mergeAssistWaitMs: 20_000 });
      const sessionId = "busy-session";
      const file = join(ws.repo, "src/api/model.ts");
      try {
        await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId, cwd: ws.repo } });
        await scanEnded();
        expect((await service.status(ws.repo)).lastScan).not.toBeNull();
        await runtime.ingest({ kind: "call", eventId: "edit", sessionId, toolCallId: "1", toolName: "Write", input: { file_path: file, content: text(12, "  cloned 12") } });
        writeFiles(ws.repo, { "src/api/model.ts": text(12, "  cloned 12") });
        await expect(runtime.ingest({ kind: "result", eventId: "edited", sessionId, toolCallId: "1", toolName: "Write", output: "ok" })).resolves.toEqual([]);
        expect(await Promise.all(detections)).toEqual([true]);
        await service.settled();
        // Node reports a rejection nothing handles once a task's microtasks drain: one turn of the event loop.
        const turn = Promise.withResolvers<void>();
        setImmediate(turn.resolve);
        await turn.promise;
        expect(unhandled).toEqual([]);
      } finally {
        process.off("unhandledRejection", onUnhandled);
        await runtime.dispose();
      }
    });
  }
});
