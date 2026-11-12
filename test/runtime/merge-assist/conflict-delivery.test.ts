import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, Conflict, ConflictDelivery } from "../../../src/engine/index.js";
import { claudeHookOutput } from "../../../src/hosts/claude/index.js";
import { SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/index.js";
import { RerailRuntime } from "../../../src/runtime/runtime.js";
import type { AgentJudge, JudgeOutcome, JudgeTask } from "../../../src/runtime/merge-assist/agent-judge.js";
import type { MergeAssistRuntime } from "../../../src/runtime/merge-assist/service.js";
import { commit, defaultAnswer, FakeJudge, git, mergeAssistWorkspaces, SUMMARY, takeNotes, writeFiles, type Workspace } from "./support/merge-assist-workspace.js";

const workspace = mergeAssistWorkspaces();

/** `function <name>() {`, 38 numbered lines, `}`: a file whose line 20 both sides change. */
const body = (name: string, changes: Readonly<Record<number, string>> = {}): string =>
  `${[`function ${name}() {`, ...Array.from({ length: 38 }, (_, index) => `  const ${name}${index + 2} = ${index + 2};`), "}"]
    .map((line, index) => changes[index + 1] ?? line).join("\n")}\n`;
/** Line 20 changed and `extra` lines built on it, as theirs commits it. */
const grown = (name: string, extra: number): string =>
  body(name, { 20: [`  const ${name}20 = "theirs";`, ...Array.from({ length: extra }, (_, index) => `  const grown${index} = ${name}20;`)].join("\n") });

/** Lines theirs builds on its change: enough for a checkpoint, and few enough for an ambient overlap. */
const CHECKPOINT = 30;
const AMBIENT = 8;

/**
 * The base holds `file` (a `body`), branch `jbarnes-1` changes its line 20 and builds `extra` lines on it, and this
 * checkout edits line 22, uncommitted: one overlap.
 */
async function overlap(file: string, extra: number): Promise<{ ws: Workspace; theirs: string }> {
  const ws = await workspace({ files: { [file]: body("retry") } });
  const theirs = ws.worktree("jbarnes-1");
  commit(theirs, { [file]: grown("retry", extra) }, "Grow retry");
  writeFiles(ws.repo, { [file]: body("retry", { 22: "  const retry22 = \"mine\";" }) });
  return { ws, theirs };
}

const open = (conflicts: readonly Conflict[]): Conflict[] => conflicts.filter(conflict => conflict.status === "open");

/** The footer a session working in the checkout reads. */
function footer(ws: Workspace, sessionId: string): unknown {
  const store = new SqliteSessionStore(ws.databasePath);
  const reader = new SqliteSessionReader(ws.databasePath);
  try {
    store.openSession({ host: "claude-code", sessionId, cwd: ws.repo }, Date.now());
    return reader.hud(sessionId).mergeAssist;
  } finally {
    store.close();
    reader.close();
  }
}

/** A delivery as the runtime builds one from a note, for a host's hook output. */
const deliveryOf = (note: { content: string; conflict?: AdapterDelivery["conflict"] }, sessionId: string): AdapterDelivery => ({
  sessionId, requested: "steer", mechanism: "additional-context", content: note.content, ruleIds: [], rules: [], strategy: "message",
  ...(note.conflict === undefined ? {} : { conflict: note.conflict }),
});

/** A judge whose answers wait until `release`; it counts the calls it was asked. */
function heldJudge(): { judge: AgentJudge; release: () => void; asked: Array<JudgeTask<unknown>> } {
  const gate = Promise.withResolvers<void>();
  const asked: Array<JudgeTask<unknown>> = [];
  const judge: AgentJudge = {
    readiness: () => ({ available: true, runner: "claude", problem: null }),
    budget: () => ({ used: asked.length, limit: 40, windowMinutes: 60 }),
    judge: async <T>(task: JudgeTask<T>): Promise<JudgeOutcome<T>> => {
      asked.push(task as JudgeTask<unknown>);
      await gate.promise;
      return { ok: true, value: task.parse(defaultAnswer()), runner: "claude", cached: false };
    },
  };
  return { judge, release: () => gate.resolve(), asked };
}

/** A hook command for a test: its argv, and each call it got, as it logged it. */
interface HookScript {
  readonly command: string[];
  readonly calls: () => Array<{ argv: string[]; event: string; id: string; cwd: string; pid: number; at: number; stdin: Record<string, unknown> }>;
}

/**
 * A conflict hook command (`node <script> <log>`): it appends what it got (argv, the two variables, its folder, its
 * stdin and its pid) to the log, then answers as `mode` says: a note and a status (at once, after 1.5 s, or after 3 s),
 * a failure, a hang, or output that is not JSON. Its waits are a real command's, in its own process.
 */
function hookScript(ws: Workspace, mode: "answer" | "fail" | "hang" | "garbage" | "slow" | "late" | "orphan" = "answer"): HookScript {
  const script = join(ws.folder, `hook-${mode}.cjs`);
  const log = join(ws.folder, `hook-${mode}.jsonl`);
  writeFileSync(log, "");
  writeFileSync(script, [
    "const fs = require('node:fs');",
    "let input = '';",
    "process.stdin.on('data', chunk => { input += chunk; });",
    "process.stdin.on('end', () => {",
    "  fs.appendFileSync(process.argv[2], JSON.stringify({ argv: process.argv.slice(2), event: process.env.RERAIL_CONFLICT_EVENT, id: process.env.RERAIL_CONFLICT_ID, cwd: process.cwd(), pid: process.pid, at: Date.now(), stdin: JSON.parse(input) }) + '\\n');",
    `  const mode = ${JSON.stringify(mode)};`,
    "  if (mode === 'fail') { process.stderr.write('no channel'); process.exit(3); }",
    "  if (mode === 'hang') { setTimeout(() => undefined, 60000); return; }",
    "  if (mode === 'garbage') { process.stdout.write('not json'); return; }",
    // A process it leaves behind in a group of its own, holding its stdout: the command's answer must not wait for it.
    "  if (mode === 'orphan') { require('node:child_process').spawn(process.execPath, ['-e', 'setTimeout(() => undefined, 6000)'], { detached: true, stdio: ['ignore', 'inherit', 'ignore'] }).unref(); return; }",
    "  const answer = () => process.stdout.write(JSON.stringify({ note: 'Ask Jeremy in #storefront before you go on.', status: 'Jeremy reshapes retry(): ask him first' }));",
    "  if (mode === 'slow') setTimeout(answer, 1500); else if (mode === 'late') setTimeout(answer, 3000); else answer();",
    "});",
  ].join("\n"));
  return {
    command: [process.execPath, script, log],
    calls: () => readFileSync(log, "utf8").split("\n").filter(line => line !== "").map(line => JSON.parse(line) as never),
  };
}

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

const FILE = "src/retry.ts";

describe("conflict delivery", { timeout: 60_000 }, () => {
  describe("modes", () => {
    async function detected(delivery: ConflictDelivery): Promise<{ ws: Workspace; service: MergeAssistRuntime; conflict: Conflict }> {
      const { ws } = await overlap(FILE, AMBIENT);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { delivery });
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.settled();
      expect(conflict).toMatchObject({ level: "ambient" });
      return { ws, service, conflict: conflict! };
    }

    it("chat: the agent's note once at the first boundary, the person's line with it, and no status line", async () => {
      const { ws, service, conflict } = await detected("chat");
      const [note, ...more] = service.pendingNotes("chat-session", ws.repo, new Set(), "claude-code");
      expect(more).toEqual([]);
      expect(note!.content).toContain(`Your edit to ${FILE} overlaps open work: jbarnes-1 (Tim Belanger) Grow retry.`);
      expect(note!.conflict).toEqual({ id: conflict.id, text: "your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry" });
      expect(takeNotes(service, "chat-session", ws.repo)).toHaveLength(1);
      expect(takeNotes(service, "chat-session", ws.repo)).toEqual([]);
      expect(footer(ws, "chat-footer")).toBeUndefined();
    });

    it("terminal: no context for the agent, only the person's line, and the summary on the status line", async () => {
      const { ws, service, conflict } = await detected("terminal");
      const [note, ...more] = service.pendingNotes("terminal-session", ws.repo, new Set(), "claude-code");
      expect(more).toEqual([]);
      expect(note).toMatchObject({ content: "", conflict: { id: conflict.id, text: "your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry" } });
      // Claude Code, at its next tool call: the person's line alone, nothing for the model.
      const output = claudeHookOutput("PostToolUse", [deliveryOf(note!, "terminal-session")], { links: "plain" });
      expect(output).toEqual({ systemMessage: expect.stringContaining("Rerail · your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry") });
      // Codex has no status line: its line carries the summary. OMP and Pi draw no person's line: nothing to hand them.
      expect(service.pendingNotes("codex-session", ws.repo, new Set(), "codex")[0]!.conflict!.text).toBe(`your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry — ${SUMMARY}`);
      expect(service.pendingNotes("omp-session", ws.repo, new Set(), "omp")).toEqual([]);
      expect(footer(ws, "terminal-footer")).toEqual({ conflicts: 1, decide: null, summary: { id: conflict.id, text: SUMMARY } });
    });

    it("both: the agent's note and the status line", async () => {
      const { ws, service, conflict } = await detected("both");
      const [note] = service.pendingNotes("both-session", ws.repo, new Set(), "claude-code");
      expect(note!.content).toContain(`Your edit to ${FILE} overlaps open work`);
      expect(footer(ws, "both-footer")).toEqual({ conflicts: 1, decide: null, summary: { id: conflict.id, text: SUMMARY } });
    });

    it("terminal: a checkpoint reaches the person's line and the status line, not the agent", async () => {
      const { ws } = await overlap(FILE, CHECKPOINT);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { delivery: "terminal" });
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      expect(conflict).toMatchObject({ level: "checkpoint" });
      expect(service.pendingNotes("s", ws.repo, new Set(), "claude-code")).toEqual([
        expect.objectContaining({ content: "", conflict: { id: conflict!.id, text: "decide: retry.ts · function retry (jbarnes-1)" } }),
      ]);
      expect(footer(ws, "f")).toMatchObject({ decide: { id: conflict!.id, title: conflict!.title } });
    });

    it("stops before an edit to a contested file in every mode", async () => {
      const migration = "db/migrations/users.sql";
      const { ws } = await overlap(migration, 60);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      expect(await service.detectAfterEdit(ws.repo, "budget-session")).toBe(true);
      expect(open((await service.status(ws.repo)).conflicts)[0]).toMatchObject({ level: "stop" });
      for (const delivery of ["chat", "terminal", "both"] as const) {
        await service.updateSettings(ws.repo, { delivery });
        const [stop] = service.stopNotes(`stop-${delivery}`, ws.repo, { toolName: "Edit", input: { file_path: join(ws.repo, migration) } }, true, new Set(), "claude-code");
        expect(stop!.content, delivery).toContain("Rerail asked the user before this edit");
        expect(stop!.conflict!.ask, delivery).toContain(`Rerail: ${migration} is contested with jbarnes-1 (Tim Belanger)`);
      }
    });
  });

  describe("the quick call", () => {
    it("shows the fallback line at once, and the summary replaces it when the call answers", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const held = heldJudge();
      const service = ws.service({ judge: held.judge });
      await service.setEnabled(ws.repo, true);
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      expect(conflict!.summary).toMatchObject({ text: "jbarnes-1 (Tim Belanger) is changing function retry", via: "fallback", reason: "pending" });
      expect(footer(ws, "before")).toMatchObject({ summary: { text: "jbarnes-1 (Tim Belanger) is changing function retry" } });
      await vi.waitFor(() => expect(held.asked.map(task => task.kind)).toEqual(["summary"]));
      expect(held.asked[0]!.prompt).toContain("Grow retry");
      held.release();
      await service.settled();
      expect(open((await service.status(ws.repo)).conflicts)[0]!.summary).toMatchObject({ text: SUMMARY, via: "judge" });
      expect(footer(ws, "after")).toMatchObject({ summary: { text: SUMMARY } });
    });

    it("keeps the fallback, with why, when the budget is spent", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const judge = new FakeJudge(task => (task.kind === "summary" ? { ok: false, reason: "budget", detail: "40 judgment calls in the last hour" } : defaultAnswer()));
      const service = ws.service({ judge });
      await service.setEnabled(ws.repo, true);
      await service.scan(ws.repo);
      await service.settled();
      expect(open((await service.status(ws.repo)).conflicts)[0]!.summary).toMatchObject({
        text: "jbarnes-1 (Tim Belanger) is changing function retry", via: "fallback", reason: "budget: 40 judgment calls in the last hour",
      });
    });

    it("never holds the boundary: the note goes out while the call is still running", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const held = heldJudge();
      const service = ws.service({ judge: held.judge });
      await service.setEnabled(ws.repo, true);
      const runtime = sessionRuntime(ws, service);
      const sessionId = "slow-judge-session";
      try {
        await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId, cwd: ws.repo } });
        await vi.waitFor(() => expect(held.asked).toHaveLength(1), { timeout: 20_000 });
        const started = Date.now();
        const deliveries = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." });
        expect(Date.now() - started).toBeLessThan(2_000);
        expect(deliveries.map(delivery => delivery.content)).toEqual([expect.stringContaining("overlaps open work")]);
        expect(open((await service.status(ws.repo)).conflicts)[0]!.summary).toMatchObject({ via: "fallback" });
      } finally {
        held.release();
        await runtime.dispose();
      }
    });
  });

  describe("the command hook", () => {
    it("gets the conflict on stdin for detected, level-changed and closed, never in its arguments", async () => {
      const { ws, theirs } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 10 } });
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.settled();
      commit(theirs, { [FILE]: grown("retry", CHECKPOINT) }, "Grow retry further");
      expect(open((await service.scan(ws.repo)).conflicts)[0]).toMatchObject({ id: conflict!.id, level: "checkpoint" });
      await service.settled();
      await service.dismissConflict(ws.repo, conflict!.id);
      await service.settled();

      const calls = hook.calls();
      expect(calls.map(call => [call.event, call.id])).toEqual([["detected", conflict!.id], ["level-changed", conflict!.id], ["closed", conflict!.id]]);
      for (const call of calls) {
        expect(call.argv).toEqual([hook.command[2]]);
        expect(call.cwd).toBe(ws.repo);
        expect(call.stdin).toMatchObject({ version: 1, event: call.event, conflict: { id: conflict!.id }, checkout: { path: ws.repo, branch: "mine", base: "main" }, session: null });
      }
      expect(calls[2]!.stdin).toMatchObject({ conflict: { status: "dismissed" } });
      expect((await service.status(ws.repo)).conflicts[0]!.hook).toMatchObject({ event: "closed", ok: true, error: null });
    });

    it("hears closed when the overlap goes away, and names the session whose boundary ran the pass", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 10 } });
      expect(await service.detectAfterEdit(ws.repo, "edit-session", "codex")).toBe(true);
      await service.settled();
      writeFiles(ws.repo, { [FILE]: body("retry") });
      await service.scan(ws.repo);
      await service.settled();
      expect(hook.calls().map(call => [call.event, call.stdin.session])).toEqual([["detected", { id: "edit-session", host: "codex" }], ["closed", null]]);
    });

    it("adds its note to the agent's note in chat modes, and its status to the status line in terminal modes", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws, "slow");
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { delivery: "both", hook: { command: hook.command, timeoutSeconds: 10 } });
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      // The agent's note waits for the command, so its note joins it; the status line does not wait.
      expect(service.pendingNotes("waiting", ws.repo)).toEqual([]);
      expect(footer(ws, "f1")).toMatchObject({ summary: { id: conflict!.id } });
      await service.settled();
      const [note] = service.pendingNotes("both", ws.repo);
      expect(note!.content).toContain("The user's conflict hook adds: Ask Jeremy in #storefront before you go on.");
      expect(footer(ws, "f2")).toMatchObject({ summary: { text: "Jeremy reshapes retry(): ask him first" } });

      await service.updateSettings(ws.repo, { delivery: "chat" });
      expect(service.pendingNotes("chat", ws.repo)[0]!.content).toContain("The user's conflict hook adds:");
      expect(footer(ws, "f3")).toBeUndefined();

      await service.updateSettings(ws.repo, { delivery: "terminal" });
      expect(service.pendingNotes("terminal", ws.repo)[0]!.content).toBe("");
      expect(footer(ws, "f4")).toMatchObject({ summary: { text: "Jeremy reshapes retry(): ask him first" } });
    });

    /**
     * A Claude Code session in a checkout whose branch has not yet overlapped jbarnes-1's committed change, the hook
     * command `mode` set; `edit` makes the overlapping edit through a tool call, and answers its result's deliveries.
     */
    async function editSession(mode: "answer" | "late"): Promise<{ ws: Workspace; service: MergeAssistRuntime; runtime: RerailRuntime; hook: HookScript; sessionId: string; edit: () => Promise<AdapterDelivery[]> }> {
      const ws = await workspace({ files: { [FILE]: body("retry") } });
      commit(ws.worktree("jbarnes-1"), { [FILE]: grown("retry", AMBIENT) }, "Grow retry");
      const hook = hookScript(ws, mode);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 10 } });
      const runtime = sessionRuntime(ws, service);
      const sessionId = `hook-${mode}-session`;
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId, cwd: ws.repo } });
      await vi.waitFor(async () => expect((await service.status(ws.repo)).lastScan).not.toBeNull(), { timeout: 20_000, interval: 100 });
      expect((await service.status(ws.repo)).conflicts).toEqual([]);
      const edit = async (): Promise<AdapterDelivery[]> => {
        await runtime.ingest({ kind: "call", eventId: "edit", sessionId, toolCallId: "1", toolName: "Edit", input: { file_path: join(ws.repo, FILE), old_string: "22;", new_string: "\"mine\";" } });
        writeFiles(ws.repo, { [FILE]: body("retry", { 22: "  const retry22 = \"mine\";" }) });
        return [...await runtime.ingest({ kind: "result", eventId: "edited", sessionId, toolCallId: "1", toolName: "Edit", output: "ok" })];
      };
      return { ws, service, runtime, hook, sessionId, edit };
    }

    const overlapNotes = (deliveries: readonly AdapterDelivery[]): AdapterDelivery[] => deliveries.filter(delivery => delivery.content.includes("overlaps open work"));
    const hookNotes = (deliveries: readonly AdapterDelivery[]): string[] => deliveries.map(delivery => delivery.content).filter(content => content.includes("The user's conflict hook adds:"));

    it("joins a command's prompt answer to Rerail's note at the edit's result that ran the detection", async () => {
      const { runtime, hook, sessionId, edit } = await editSession("answer");
      try {
        const [note, ...others] = overlapNotes(await edit());
        expect(others).toEqual([]);
        expect(note!.content).toContain("The user's conflict hook adds: Ask Jeremy in #storefront before you go on.");
        expect(note!.conflict).toMatchObject({ text: expect.stringContaining("your edit overlaps jbarnes-1") });
        expect(hook.calls()[0]).toMatchObject({ event: "detected", stdin: { session: { id: sessionId, host: "claude-code" } } });
        expect(hookNotes(await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." }))).toEqual([]);
      } finally {
        await runtime.dispose();
      }
    });

    it("holds Rerail's note at most a second for a slow command: the note goes out with the edit's result, the hook's note at the next boundary, once", async () => {
      const { service, runtime, sessionId, edit } = await editSession("late");
      try {
        const started = Date.now();
        const handed = await edit();
        expect(Date.now() - started).toBeLessThan(3_000);
        const [note, ...others] = overlapNotes(handed);
        expect(others).toEqual([]);
        expect(note!.content).not.toContain("conflict hook");
        expect(note!.conflict).toMatchObject({ text: expect.stringContaining("your edit overlaps jbarnes-1") });
        await service.settled();
        const next = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." });
        expect(overlapNotes(next)).toEqual([]);
        expect(hookNotes(next)).toEqual([[
          "This note comes from Rerail, not the user.",
          "About the conflict with jbarnes-1: retry.ts · function retry.",
          "The user's conflict hook adds: Ask Jeremy in #storefront before you go on.",
        ].join("\n")]);
        expect(hookNotes(await runtime.ingest({ kind: "user-prompt", eventId: "after", sessionId, text: "And on." }))).toEqual([]);
      } finally {
        await runtime.dispose();
      }
    });

    it("stops a command still running when Merge Assist stops: its process group gets SIGTERM, then SIGKILL", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws, "hang");
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 60 } });
      await service.scan(ws.repo);
      await vi.waitFor(() => expect(hook.calls()).toHaveLength(1), { timeout: 20_000 });
      const { pid } = hook.calls()[0]!;
      expect(() => process.kill(-pid, 0)).not.toThrow();
      const started = Date.now();
      await service.stop(2_000);
      expect(Date.now() - started).toBeLessThan(2_500);
      expect(() => process.kill(-pid, 0)).toThrow(expect.objectContaining({ code: "ESRCH" }));
    });

    it("gives a Codex person's line in terminal mode the status line text as it is when the note is handed over", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws, "slow");
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { delivery: "terminal", hook: { command: hook.command, timeoutSeconds: 10 } });
      const runtime = sessionRuntime(ws, service);
      const sessionId = "codex-terminal-session";
      try {
        await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "codex", sessionId, cwd: ws.repo } });
        // The note is queued at once, with the fallback line, while the command is still answering.
        await vi.waitFor(() => expect(hook.calls()).toHaveLength(1), { timeout: 20_000 });
        expect(service.pendingNotes("probe", ws.repo, new Set(), "codex")[0]!.conflict!.text).not.toContain("Jeremy");
        await service.settled();
        const deliveries = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." });
        const lines = deliveries.flatMap(delivery => (delivery.conflict === undefined ? [] : [delivery.conflict.text]));
        expect(lines).toEqual([expect.stringMatching(/ — Jeremy reshapes retry\(\): ask him first$/u)]);
      } finally {
        await runtime.dispose();
      }
    });

    it("answers within its timeout and grace though a process it left behind holds its output", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws, "orphan");
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 1 } });
      const started = Date.now();
      await service.scan(ws.repo);
      await service.settled();
      expect(Date.now() - started).toBeLessThan(5_000);
      expect(open((await service.status(ws.repo)).conflicts)[0]!.hook).toMatchObject({ ok: false, error: "timed out after 1 s" });
    });
    for (const [mode, timeoutSeconds, error] of [
      ["fail", 10, /^exited with code 3: no channel$/u],
      ["hang", 1, /^timed out after 1 s$/u],
      ["garbage", 10, /^its output is not JSON/u],
    ] as const) {
      it(`records a command that ${mode === "fail" ? "exits non-zero" : mode === "hang" ? "times out" : "answers with output that is not JSON"}, and delivers as without it`, async () => {
        const { ws } = await overlap(FILE, AMBIENT);
        const hook = hookScript(ws, mode);
        const service = ws.service();
        await service.setEnabled(ws.repo, true);
        await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds } });
        const [conflict] = open((await service.scan(ws.repo)).conflicts);
        await service.settled();
        expect(open((await service.status(ws.repo)).conflicts)[0]!.hook).toMatchObject({ event: "detected", ok: false, error: expect.stringMatching(error), note: null, status: null });
        const [note] = service.pendingNotes("s", ws.repo);
        expect(note!.content).toContain(`Your edit to ${FILE} overlaps open work`);
        expect(note!.content).not.toContain("conflict hook");
        expect(footer(ws, "f")).toMatchObject({ summary: { id: conflict!.id, text: SUMMARY } });
      });
    }
  });

  describe("the order of a conflict's events", () => {
    it("hands a command hook a dismissal right after a scan only once it has answered detected", async () => {
      const { ws } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws, "slow");
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 10 } });
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.dismissConflict(ws.repo, conflict!.id);
      await service.settled();
      const calls = hook.calls();
      expect(calls.map(call => [call.event, call.id])).toEqual([["detected", conflict!.id], ["closed", conflict!.id]]);
      // The slow command answers 1.5 s after it starts: closed started only after detected's run ended.
      expect(calls[1]!.at - calls[0]!.at).toBeGreaterThanOrEqual(1_400);
      expect((await service.status(ws.repo)).conflicts[0]!.hook).toMatchObject({ event: "closed", ok: true });
    });
  });

  describe("a conflict's identity", () => {
    /** Main gains `count` lines at the top of `FILE`; this checkout (its edit committed) and theirs both merge it. */
    function mainGainsLinesAbove(ws: Workspace, theirs: string, count: number): void {
      const tip = ws.worktree("main-tip");
      const above = Array.from({ length: count }, (_, index) => `// header ${index + 1}`).join("\n");
      commit(tip, { [FILE]: `${above}\n${readFileSync(join(tip, FILE), "utf8")}` }, "Header above");
      git(ws.repo, "branch", "-f", "main", "main-tip");
      commit(ws.repo, {}, "Mine");
      git(ws.repo, "merge", "-q", "--no-edit", "main");
      git(theirs, "merge", "-q", "--no-edit", "main");
    }

    it("keeps its id and its dismissal when main gains lines above it and both sides merge main, and the hook hears nothing", async () => {
      const { ws, theirs } = await overlap(FILE, AMBIENT);
      const hook = hookScript(ws);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds: 10 } });
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.dismissConflict(ws.repo, conflict!.id);
      await service.settled();
      expect(hook.calls().map(call => [call.event, call.id])).toEqual([["detected", conflict!.id], ["closed", conflict!.id]]);

      mainGainsLinesAbove(ws, theirs, 3);
      const status = await service.scan(ws.repo);
      await service.settled();
      expect(status.conflicts).toEqual([expect.objectContaining({ id: conflict!.id, status: "dismissed" })]);
      expect(hook.calls()).toHaveLength(2);
      // The overlap is still there, three lines down in the base, under the same id: found afresh once the store is cleared.
      await service.setEnabled(ws.repo, false);
      await service.setEnabled(ws.repo, true);
      expect(open((await service.scan(ws.repo)).conflicts)).toEqual([expect.objectContaining({ id: conflict!.id, theirs: expect.objectContaining({ location: `${FILE}:23` }) })]);
    });
  });

  describe("the card", () => {
    it("says what the conflict is, whom to talk to, and that it can be dismissed; only the person decides", async () => {
      const { ws } = await overlap(FILE, CHECKPOINT);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      const [card] = takeNotes(service, "session-1", ws.repo);
      expect(card).toContain(`◆ RERAIL · conflict #${conflict!.shortId}: retry.ts · function retry`);
      expect(card).toContain("  Theirs   \"Grow retry\"  (written by Tim Belanger · jbarnes-1)");
      expect(card).toContain("  → Talk to Tim Belanger about it (jbarnes-1).");
      expect(card).toContain("    Not a conflict? Dismiss it on the Merge Assist page of the dashboard.");
      expect(card).toContain("Don't decide it yourself; only the user decides.");
      // Handed once to the session: a later boundary carries it no more.
      expect(takeNotes(service, "session-1", ws.repo)).toEqual([]);
    });
  });
});
