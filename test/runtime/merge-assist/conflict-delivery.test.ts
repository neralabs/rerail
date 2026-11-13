import { readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, Conflict } from "../../../src/engine/index.js";
import { claudeHookOutput } from "../../../src/hosts/claude/index.js";
import { SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/index.js";
import type { RerailRuntime } from "../../../src/runtime/runtime.js";
import type { MergeAssistRuntime } from "../../../src/runtime/merge-assist/service.js";
import { commit, git, writeFiles } from "../../support/git.js";
import { mergeAssistWorkspaces, takeNotes, type Workspace } from "../../support/merge-assist/workspace.js";
import { body, changed, open, sessionRuntime } from "./support/conflict-sessions.js";

const workspace = mergeAssistWorkspaces();

/** The overlap's line: who on the other side is changing what. */
const SUMMARY = "jbarnes-1 (Tim Belanger) is changing function retry";

/**
 * The base holds `file` (a `body`), branch `jbarnes-1` changes its line 20 (in a commit `subject` says), and this
 * checkout edits line 22, uncommitted: one overlap.
 */
async function overlap(file: string, subject = "Grow retry"): Promise<{ ws: Workspace; theirs: string }> {
  const ws = await workspace({ files: { [file]: body("retry") } });
  const theirs = ws.worktree("jbarnes-1");
  commit(theirs, { [file]: changed("retry") }, subject);
  writeFiles(ws.repo, { [file]: body("retry", { 22: "  const retry22 = \"mine\";" }) });
  return { ws, theirs };
}

/** The footer a session working in `cwd` (the checkout by default) reads. */
function footer(ws: Workspace, sessionId: string, cwd = ws.repo): unknown {
  const store = new SqliteSessionStore(ws.databasePath);
  const reader = new SqliteSessionReader(ws.databasePath);
  try {
    store.openSession({ host: "claude-code", sessionId, cwd }, Date.now());
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

/** One call a hook command got, as it logged it on starting. */
interface HookCall {
  readonly argv: string[];
  readonly event: string;
  readonly id: string;
  readonly cwd: string;
  readonly pid: number;
  readonly stdin: Record<string, unknown>;
}

/** A hook command for a test: its argv, what it logged, and the gate a held command waits on. */
interface HookScript {
  readonly command: string[];
  readonly calls: () => HookCall[];
  /** What it did, in order: `call <event>` on starting, `answer <event>` as it answered. */
  readonly steps: () => string[];
  /** The process an `orphan` command left behind. */
  readonly orphan: () => number;
  /** Lets a `held` command answer, and the process an `orphan` command left behind exit. */
  readonly release: () => void;
}

/** How a test's hook command answers ({@link hookScript}). */
type HookMode = "answer" | "fail" | "hang" | "garbage" | "held" | "orphan";

/**
 * A conflict hook command (`node <script> <log>`): it appends what it got (argv, the two variables, its folder, its
 * stdin and its pid) to the log, then answers as `mode` says: a note and a status (at once, or, `held`, once the test
 * releases it), a failure, a hang, output that is not JSON, or nothing while a process it left behind holds its
 * output until the test releases it. The release is a file next to the log, which both processes poll for.
 */
function hookScript(ws: Workspace, mode: HookMode = "answer"): HookScript {
  const script = join(ws.folder, `hook-${mode}.cjs`);
  const log = join(ws.folder, `hook-${mode}.jsonl`);
  const release = `${log}.release`;
  writeFileSync(log, "");
  const released = (then: string): string => `const poll = setInterval(() => { if (require('node:fs').existsSync(${JSON.stringify(release)})) { clearInterval(poll); ${then}; } }, 20);`;
  writeFileSync(script, [
    "const fs = require('node:fs');",
    "const record = entry => fs.appendFileSync(process.argv[2], JSON.stringify(entry) + '\\n');",
    "let input = '';",
    "process.stdin.on('data', chunk => { input += chunk; });",
    "process.stdin.on('end', () => {",
    "  const event = process.env.RERAIL_CONFLICT_EVENT;",
    "  record({ kind: 'call', argv: process.argv.slice(2), event, id: process.env.RERAIL_CONFLICT_ID, cwd: process.cwd(), pid: process.pid, stdin: JSON.parse(input) });",
    `  const mode = ${JSON.stringify(mode)};`,
    "  if (mode === 'fail') { process.stderr.write('no channel'); process.exit(3); }",
    "  if (mode === 'hang') { setTimeout(() => undefined, 60000); return; }",
    "  if (mode === 'garbage') { process.stdout.write('not json'); return; }",
    // A process it leaves behind in a group of its own, holding its stdout until released (at most a minute, should the
    // test stop first): the command's answer must not wait for it.
    `  if (mode === 'orphan') { const orphan = require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(`${released("process.exit(0)")} setTimeout(() => process.exit(0), 60000);`)}], { detached: true, stdio: ['ignore', 'inherit', 'ignore'] }); record({ kind: 'orphan', event, pid: orphan.pid }); orphan.unref(); return; }`,
    "  const answer = () => { record({ kind: 'answer', event }); process.stdout.write(JSON.stringify({ note: 'Ask Jeremy in #storefront before you go on.', status: 'Jeremy reshapes retry(): ask him first' })); };",
    `  if (mode === 'held') { ${released("answer()")} } else answer();`,
    "});",
  ].join("\n"));
  const entries = (): Array<{ kind: string; event: string; pid: number }> =>
    readFileSync(log, "utf8").split("\n").filter(line => line !== "").map(line => JSON.parse(line) as never);
  return {
    command: [process.execPath, script, log],
    calls: () => entries().filter(entry => entry.kind === "call") as never,
    steps: () => entries().filter(entry => entry.kind !== "orphan").map(entry => `${entry.kind} ${entry.event}`),
    orphan: () => entries().find(entry => entry.kind === "orphan")!.pid,
    release: () => writeFileSync(release, ""),
  };
}

/**
 * Merge Assist on in `ws`, with a `mode` hook command set (`timeoutSeconds` long); `noteWaitMs` bounds how long a
 * conflict's agent note waits for the command.
 */
async function withHook(ws: Workspace, mode: HookMode = "answer", { timeoutSeconds = 10, noteWaitMs }: { timeoutSeconds?: number; noteWaitMs?: number } = {}): Promise<{ hook: HookScript; service: MergeAssistRuntime }> {
  const hook = hookScript(ws, mode);
  const service = ws.service(noteWaitMs === undefined ? {} : { noteWaitMs });
  await service.setEnabled(ws.repo, true);
  await service.updateSettings(ws.repo, { hook: { command: hook.command, timeoutSeconds } });
  return { hook, service };
}

/** {@link overlap} in `FILE`, with Merge Assist on and a `mode` hook command set ({@link withHook}). */
async function hooked(mode: HookMode = "answer", options: { timeoutSeconds?: number } = {}): Promise<{ ws: Workspace; theirs: string; hook: HookScript; service: MergeAssistRuntime }> {
  const { ws, theirs } = await overlap(FILE);
  return { ws, theirs, ...await withHook(ws, mode, options) };
}

const FILE = "src/retry.ts";

describe("conflict delivery", { timeout: 60_000 }, () => {
  describe("where a conflict reaches", () => {
    async function detected(file = FILE): Promise<{ ws: Workspace; service: MergeAssistRuntime; conflict: Conflict }> {
      const { ws } = await overlap(file);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.settled();
      return { ws, service, conflict: conflict! };
    }

    it("the agent's note once at the first boundary, the person's line with it, and the status line", async () => {
      const { ws, service, conflict } = await detected();
      expect(conflict).toMatchObject({ quiet: false, summary: SUMMARY });
      const [note, ...more] = service.pendingNotes("session", ws.repo, new Set(), "claude-code");
      expect(more).toEqual([]);
      expect(note!.content).toContain(`Your edit to ${FILE} overlaps open work: jbarnes-1 (Tim Belanger), described on their side as "Grow retry".`);
      expect(note!.conflict).toEqual({ id: conflict.id, text: "your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry" });
      // Claude Code, at its next tool call: the person's line beside the note.
      const output = claudeHookOutput("PostToolUse", [deliveryOf(note!, "session")], { links: "plain" });
      expect(output).toMatchObject({ systemMessage: expect.stringContaining("Rerail · your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry") });
      // Codex has no status line: its line carries the conflict's line.
      expect(service.pendingNotes("codex-session", ws.repo, new Set(), "codex")[0]!.conflict!.text).toBe(`your edit overlaps jbarnes-1 (Tim Belanger): retry.ts · function retry — ${SUMMARY}`);
      expect(takeNotes(service, "session", ws.repo)).toHaveLength(1);
      expect(takeNotes(service, "session", ws.repo)).toEqual([]);
      expect(footer(ws, "footer")).toEqual({ conflicts: 1, summary: { id: conflict.id, text: SUMMARY } });
    });

    it("the status line of a session that reached the checkout through a symlink, below its root", async () => {
      const { ws, conflict } = await detected();
      const linked = join(ws.folder, "linked");
      symlinkSync(ws.repo, linked);
      expect(footer(ws, "linked-footer", join(linked, "src"))).toEqual({ conflicts: 1, summary: { id: conflict.id, text: SUMMARY } });
    });

    it("quotes their side's summary on one line, cut short however long they wrote it", async () => {
      const { ws } = await overlap(FILE, `Grow retry ${"and more\t".repeat(600)}`);
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      await service.scan(ws.repo);
      await service.settled();
      const [note] = service.pendingNotes("session", ws.repo, new Set(), "claude-code");
      const lines = note!.content.split("\n");
      expect(lines).toHaveLength(4);
      expect(lines[1]).toMatch(/^Your edit to src\/retry\.ts overlaps open work: jbarnes-1 \(Tim Belanger\), described on their side as "Grow retry and more and more .*…"\.$/u);
      expect(lines[1]!.length).toBeLessThan(400);
    });

    it("a quiet conflict, only in docs, reaches neither the agent, the person's line nor the status line", async () => {
      const { ws, service, conflict } = await detected("docs/retry.md");
      expect(conflict).toMatchObject({ quiet: true, status: "open" });
      expect(service.pendingNotes("session", ws.repo, new Set(), "claude-code")).toEqual([]);
      expect(footer(ws, "footer")).toBeUndefined();
    });
  });

  describe("the command hook", () => {
    it("gets the conflict on stdin for detected and closed, never in its arguments", async () => {
      const { ws, hook, service } = await hooked();
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.settled();
      await service.dismissConflict(ws.repo, conflict!.id);
      await service.settled();

      const calls = hook.calls();
      expect(calls.map(call => [call.event, call.id])).toEqual([["detected", conflict!.id], ["closed", conflict!.id]]);
      for (const call of calls) {
        expect(call.argv).toEqual([hook.command[2]]);
        expect(call.cwd).toBe(ws.repo);
        expect(call.stdin).toMatchObject({ version: 1, event: call.event, conflict: { id: conflict!.id }, checkout: { path: ws.repo, branch: "mine", base: "main" }, session: null });
      }
      expect(calls[1]!.stdin).toMatchObject({ conflict: { status: "dismissed" } });
      expect((await service.status(ws.repo)).conflicts[0]!.hook).toMatchObject({ event: "closed", ok: true, error: null });
    });

    it("hears closed when the overlap goes away, and names the session whose boundary ran the pass", async () => {
      const { ws, hook, service } = await hooked();
      expect(await service.detectAfterEdit(ws.repo, "edit-session", "codex")).toBe(true);
      await service.settled();
      writeFiles(ws.repo, { [FILE]: body("retry") });
      await service.scan(ws.repo);
      await service.settled();
      expect(hook.calls().map(call => [call.event, call.stdin.session])).toEqual([["detected", { id: "edit-session", host: "codex" }], ["closed", null]]);
    });

    it("adds its note to the agent's note, and its status to the status line", async () => {
      const { ws, hook, service } = await hooked("held");
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      // The status line does not wait for the command: it shows the conflict's own line meanwhile.
      expect(footer(ws, "f1")).toMatchObject({ summary: { id: conflict!.id, text: SUMMARY } });
      hook.release();
      await service.settled();
      const [note] = service.pendingNotes("session", ws.repo);
      expect(note!.content).toContain("The user's conflict hook adds: Ask Jeremy in #storefront before you go on.");
      expect(footer(ws, "f2")).toMatchObject({ summary: { text: "Jeremy reshapes retry(): ask him first" } });
    });

    /**
     * A Claude Code session in a checkout whose branch has not yet overlapped jbarnes-1's committed change, the hook
     * command `mode` set; `edit` makes the overlapping edit through a tool call, and answers its result's deliveries.
     * `noteWaitMs` bounds how long Rerail's note waits for the command, `mergeAssistWaitMs` how long the edit's result
     * waits for Merge Assist.
     */
    async function editSession(mode: "answer" | "held", bounds: { noteWaitMs: number; mergeAssistWaitMs: number }): Promise<{ ws: Workspace; service: MergeAssistRuntime; runtime: RerailRuntime; hook: HookScript; sessionId: string; edit: () => Promise<AdapterDelivery[]> }> {
      const ws = await workspace({ files: { [FILE]: body("retry") } });
      commit(ws.worktree("jbarnes-1"), { [FILE]: changed("retry") }, "Grow retry");
      const { hook, service } = await withHook(ws, mode, { noteWaitMs: bounds.noteWaitMs });
      const runtime = sessionRuntime(ws, service, { mergeAssistWaitMs: bounds.mergeAssistWaitMs });
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
    /** Bounds no machine runs into: what is waited for always arrives in time. */
    const ROOMY = 20_000;

    it("joins a command's prompt answer to Rerail's note at the edit's result that ran the detection", async () => {
      const { runtime, hook, sessionId, edit } = await editSession("answer", { noteWaitMs: ROOMY, mergeAssistWaitMs: ROOMY });
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

    it("holds Rerail's note only within its bound for a slow command: the note goes out with the edit's result, the hook's note at the next boundary, once", async () => {
      const { service, runtime, hook, sessionId, edit } = await editSession("held", { noteWaitMs: 200, mergeAssistWaitMs: ROOMY });
      try {
        // The command answers only once released: the edit's result hands out Rerail's note while it is still running.
        const [note, ...others] = overlapNotes(await edit());
        expect(hook.steps()).not.toContain("answer detected");
        expect(others).toEqual([]);
        expect(note!.content).not.toContain("conflict hook");
        expect(note!.conflict).toMatchObject({ text: expect.stringContaining("your edit overlaps jbarnes-1") });
        hook.release();
        await service.settled();
        const next = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." });
        expect(overlapNotes(next)).toEqual([]);
        expect(hookNotes(next)).toEqual([expect.stringContaining("The user's conflict hook adds: Ask Jeremy in #storefront before you go on.")]);
        expect(hookNotes(next)[0]).toContain("About the conflict with jbarnes-1");
        expect(hookNotes(await runtime.ingest({ kind: "user-prompt", eventId: "after", sessionId, text: "And on." }))).toEqual([]);
      } finally {
        await runtime.dispose();
      }
    });

    it("answers the edit's result within the session's bound on Merge Assist, though the command still holds the note", async () => {
      const { ws, service, runtime, hook, sessionId, edit } = await editSession("held", { noteWaitMs: ROOMY, mergeAssistWaitMs: 200 });
      try {
        const started = performance.now();
        const deliveries = await edit();
        // Far below the command's own bound: the result did not wait for it.
        expect(performance.now() - started).toBeLessThan(2_500);
        expect(overlapNotes(deliveries)).toEqual([]);
        expect(hook.steps()).not.toContain("answer detected");
        // Once the command answers, the note and the hook's note go out together at the next boundary.
        await vi.waitFor(() => expect(hook.steps()).toEqual(["call detected"]), { timeout: 20_000 });
        hook.release();
        await service.settled();
        await vi.waitFor(() => expect(service.pendingNotes(sessionId, ws.repo)).toHaveLength(1), { timeout: 20_000 });
        const [note, ...others] = overlapNotes(await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." }));
        expect(others).toEqual([]);
        expect(note!.content).toContain("The user's conflict hook adds: Ask Jeremy in #storefront before you go on.");
      } finally {
        await runtime.dispose();
      }
    });

    it("stops a command still running when Merge Assist stops: its process group gets SIGTERM, then SIGKILL", async () => {
      const { ws, hook, service } = await hooked("hang", { timeoutSeconds: 60 });
      await service.scan(ws.repo);
      await vi.waitFor(() => expect(hook.calls()).toHaveLength(1), { timeout: 20_000 });
      const { pid } = hook.calls()[0]!;
      expect(() => process.kill(-pid, 0)).not.toThrow();
      await service.stop(2_000);
      expect(() => process.kill(-pid, 0)).toThrow(expect.objectContaining({ code: "ESRCH" }));
    });

    it("gives a Codex person's line the hook's status as it is when the note is handed over", async () => {
      const { ws, hook, service } = await hooked("held");
      const runtime = sessionRuntime(ws, service);
      const sessionId = "codex-session";
      try {
        await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "codex", sessionId, cwd: ws.repo } });
        // The command answers once released; the note goes out at the next boundary, after it answered.
        await vi.waitFor(() => expect(hook.calls()).toHaveLength(1), { timeout: 20_000 });
        hook.release();
        await service.settled();
        const deliveries = await runtime.ingest({ kind: "user-prompt", eventId: "next", sessionId, text: "Go on." });
        const lines = deliveries.flatMap(delivery => (delivery.conflict === undefined ? [] : [delivery.conflict.text]));
        expect(lines).toEqual([expect.stringMatching(/ — Jeremy reshapes retry\(\): ask him first$/u)]);
      } finally {
        await runtime.dispose();
      }
    });

    it("answers within its timeout and grace though a process it left behind holds its output", async () => {
      const { ws, hook, service } = await hooked("orphan", { timeoutSeconds: 1 });
      await service.scan(ws.repo);
      await service.settled();
      const orphan = hook.orphan();
      try {
        expect(open((await service.status(ws.repo)).conflicts)[0]!.hook).toMatchObject({ ok: false, error: "timed out after 1 s" });
        // The process it left behind exits only once released: the answer came while it still held the output.
        expect(() => process.kill(orphan, 0)).not.toThrow();
      } finally {
        hook.release();
        await vi.waitFor(() => expect(() => process.kill(orphan, 0)).toThrow(expect.objectContaining({ code: "ESRCH" })), { timeout: 20_000 });
      }
    });
    for (const [mode, timeoutSeconds, error] of [
      ["fail", 10, /^exited with code 3: no channel$/u],
      ["hang", 1, /^timed out after 1 s$/u],
      ["garbage", 10, /^its output is not JSON/u],
    ] as const) {
      it(`records a command that ${mode === "fail" ? "exits non-zero" : mode === "hang" ? "times out" : "answers with output that is not JSON"}, and delivers as without it`, async () => {
        const { ws, service } = await hooked(mode, { timeoutSeconds });
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
      const { ws, hook, service } = await hooked("held");
      const [conflict] = open((await service.scan(ws.repo)).conflicts);
      await service.dismissConflict(ws.repo, conflict!.id);
      // The command holds its answer to detected until released; closed is not handed to it meanwhile.
      await vi.waitFor(() => expect(hook.steps()).toEqual(["call detected"]), { timeout: 20_000 });
      hook.release();
      await service.settled();
      expect(hook.calls().map(call => [call.event, call.id])).toEqual([["detected", conflict!.id], ["closed", conflict!.id]]);
      expect(hook.steps()).toEqual(["call detected", "answer detected", "call closed", "answer closed"]);
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
      const { ws, theirs, hook, service } = await hooked();
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
});
