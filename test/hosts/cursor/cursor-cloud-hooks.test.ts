import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { codeRuleSchema, defaultRerailDatabasePath, sessionStartNote, type AdapterDelivery, type AdapterEvent, type RerailEventLoop } from "../../../src/engine/index.js";
import { markCursorCloud } from "../../../src/hosts/cursor/cursor-cloud.js";
import { handleCursorEvent, runCursorHook } from "../../../src/hosts/cursor/cursor-hook.js";
import type { HookInput, HookOptions, HookOutput } from "../../../src/harness/index.js";
import { answering, scriptedLoop, temporaryDirectories } from "../../support/hook-events.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../../support/native-daemon.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { steeredRules } from "../../support/steers.js";

const temporaryDirectory = temporaryDirectories("rerail-cursor-cloud-");

afterEach(async () => {
  vi.unstubAllEnvs();
  await closeNativeTestDaemons();
});

/** What the daemon's answer to a session's start says of its rules, as a daemon that knows it answers. */
const NOTE_RULES = { setUp: true, ruleCount: 3 } as const;
/** The note a Cursor session's start carries where the answer names no dashboard and those rules. */
const note = (sessionId: string) => sessionStartNote(undefined, "cursor", sessionId, NOTE_RULES);

/** A loop whose answers carry the facts a daemon's do: no dashboard to open, and at a start what the note says. */
function daemonLoop(script: Readonly<Record<string, readonly AdapterDelivery[]>> = {}): RerailEventLoop & { events: AdapterEvent[] } {
  return answering(scriptedLoop(script), { dashboard: null, sessionNote: NOTE_RULES });
}

/** A loop that leaves each event for a daemon that is not reading, as a hook does once one was left untaken. */
function untakenLoop(): RerailEventLoop & { events: AdapterEvent[] } {
  return { ...scriptedLoop({}), untaken: true };
}

function steer(sessionId: string, content: string): AdapterDelivery {
  return { sessionId, requested: "steer", mechanism: "deferred-context", content, ruleIds: ["acme:no-print"], rules: steeredRules(["acme:no-print"]) };
}

/** A Cursor cloud agent's hooks for one tool call, its end of run and one more tool call: no `sessionStart`. */
function cloudRun(sessionId: string, cwd: string) {
  const common = { conversation_id: sessionId, generation_id: "g1", cwd };
  const tool = (id: string) => ({ ...common, tool_use_id: id, tool_name: "Shell", tool_input: { command: "ls" } });
  return {
    call: { ...tool("t1"), hook_event_name: "preToolUse" },
    result: { ...tool("t1"), hook_event_name: "postToolUse", tool_output: "a" },
    stop: { ...common, hook_event_name: "stop", status: "completed", loop_count: 0 },
    nextResult: { ...tool("t2"), hook_event_name: "postToolUse", tool_output: "b" },
  };
}

describe("Cursor hooks on a Cursor cloud agent's VM, where Cursor fires no sessionStart", () => {
  it("records the agent's own session start once, before its first hook's event, and gives the note with its first postToolUse only", async () => {
    const root = await temporaryDirectory({ started: false });
    const storage = join(root, "sessions");
    const run = cloudRun("cloud-session", root);

    const call = daemonLoop();
    expect(await handleCursorEvent(run.call, call, storage, undefined, true)).toEqual({ permission: "allow" });
    // The same start Cursor's sessionStart hook sends locally, under the agent's own session id.
    expect(call.events.map(event => [event.kind, event.eventId])).toEqual([
      ["session-start", "cloud-session:session-start"],
      ["call", "cloud-session:tool-call:t1:Shell"],
    ]);
    expect(call.events[0]).toMatchObject({ session: { host: "cursor", sessionId: "cloud-session", cwd: root } });

    const result = daemonLoop({ result: [steer("cloud-session", "Remove print.")] });
    const first = await handleCursorEvent(run.result, result, storage, undefined, true);
    expect(result.events.map(event => event.kind)).toEqual(["result"]);
    expect(first).toMatchObject({ additional_context: expect.stringMatching(/^Rerail is on in this repo: 3 rules are on\./u) });
    expect(String(first.additional_context)).toContain(note("cloud-session"));
    expect(String(first.additional_context)).toContain("Remove print.");

    const stop = daemonLoop();
    expect(await handleCursorEvent(run.stop, stop, storage, undefined, true)).toEqual({});
    const next = daemonLoop({ result: [steer("cloud-session", "Remove print again.")] });
    const second = await handleCursorEvent(run.nextResult, next, storage, undefined, true);
    expect([...stop.events, ...next.events].map(event => event.kind)).toEqual(["turn-end", "result"]);
    expect(second).toMatchObject({ additional_context: expect.stringContaining("Remove print again.") });
    expect(String(second.additional_context)).not.toContain("Rerail is on in this repo");
  });

  it("sends no start and gives no note on a person's machine, where Cursor's own sessionStart does both", async () => {
    const root = await temporaryDirectory({ started: false });
    const storage = join(root, "sessions");
    const run = cloudRun("local-session", root);
    const loops = [daemonLoop(), daemonLoop({ result: [steer("local-session", "Remove print.")] }), daemonLoop(), daemonLoop()];
    const answers: HookOutput[] = [];
    for (const [index, input] of [run.call, run.result, run.stop, run.nextResult].entries()) answers.push(await handleCursorEvent(input, loops[index]!, storage));
    expect(loops.flatMap(loop => loop.events.map(event => event.kind))).toEqual(["call", "result", "turn-end", "result"]);
    expect(JSON.stringify(answers)).toContain("Remove print.");
    expect(JSON.stringify(answers)).not.toContain("Rerail is on");
  });

  it("leaves the start before the first hook's event for a daemon that is not reading, and owes its note to the first postToolUse the daemon answers", async () => {
    const root = await temporaryDirectory({ started: false });
    const storage = join(root, "sessions");
    const run = cloudRun("left-session", root);

    const left = untakenLoop();
    expect(await handleCursorEvent(run.call, left, storage, undefined, true)).toEqual({ permission: "allow" });
    expect(left.events.map(event => event.kind)).toEqual(["session-start", "call"]);
    // Left untaken too, the result has no answer to carry the note: it stays owed.
    const leftResult = untakenLoop();
    expect(await handleCursorEvent(run.result, leftResult, storage, undefined, true)).toEqual({});
    expect(leftResult.events.map(event => event.kind)).toEqual(["result"]);

    // The start's answer never came, so the note is built from the answer that carries it.
    const next = daemonLoop();
    expect(await handleCursorEvent(run.nextResult, next, storage, undefined, true)).toEqual({ additional_context: note("left-session") });
    expect(next.events.map(event => event.kind)).toEqual(["result"]);
    expect(await handleCursorEvent({ ...run.nextResult, tool_use_id: "t3" }, daemonLoop(), storage, undefined, true)).toEqual({});
  });

  it("sends no second start after a sessionStart Cursor did fire, whose answer gave the note", async () => {
    const root = await temporaryDirectory({ started: false });
    const storage = join(root, "sessions");
    const run = cloudRun("started-session", root);
    const start = daemonLoop();
    const started = { conversation_id: "started-session", cwd: root, hook_event_name: "sessionStart" };
    expect(await handleCursorEvent(started, start, storage, undefined, true)).toEqual({ additional_context: note("started-session") });
    expect(start.events.map(event => event.kind)).toEqual(["session-start"]);
    const result = daemonLoop();
    expect(await handleCursorEvent(run.result, result, storage, undefined, true)).toEqual({});
    expect(result.events.map(event => event.kind)).toEqual(["result"]);
  });
});

/** A code rule that steers every `print(` in a Python file. */
const NO_PRINT = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:no-print", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{
    id: "no-print",
    violation: { code: "PRINT", anchor: "witness.span", message: "Use logging" },
    forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
  }],
});

describe("a simulated Cursor cloud run against a running daemon", () => {
  /**
   * A Git repository under `root` with `NO_PRINT` on, served by a daemon, and hook options that take the daemon path a
   * coding agent's hook takes and read the database the daemon serves.
   */
  async function servedRepository(root: string): Promise<{ repository: string; options: HookOptions }> {
    const repository = join(root, "repository");
    await mkdir(repository, { recursive: true });
    execFileSync("git", ["init", "-q"], { cwd: repository });
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "first"], { cwd: repository });
    setUpTestRepository(repository, { library: [{ kind: "code", rule: NO_PRINT }] });
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    await startNativeTestDaemon(repository);
    return { repository, options: { storageRoot: join(root, "sessions") } };
  }

  /**
   * Cursor's hooks for an edit that breaks the rule, the end of the agent's run and one more tool call, with no
   * `sessionStart`: each answer, and the kinds the database records for the session, in order.
   */
  async function editRun(repository: string, sessionId: string, options: HookOptions): Promise<{ answers: HookOutput[]; kinds: string[] }> {
    const common = { session_id: sessionId, cwd: repository };
    const file = join(repository, "app.py");
    const content = "print(1)\n";
    const write = { ...common, tool_name: "Write", tool_use_id: "write-1", tool_input: { file_path: file, content } };
    const read = { ...common, tool_name: "Read", tool_use_id: "read-1", tool_input: { file_path: file } };
    const run = (input: HookInput) => runCursorHook(input, options);
    await run({ ...write, hook_event_name: "preToolUse" });
    await writeFile(file, content);
    const answers = [
      await run({ ...write, hook_event_name: "postToolUse", tool_output: "ok" }),
      await run({ ...common, hook_event_name: "stop", status: "completed", loop_count: 0 }),
      await run({ ...read, hook_event_name: "postToolUse", tool_output: content }),
    ];
    const database = new Database(defaultRerailDatabasePath(), { readonly: true });
    try {
      const rows = database.prepare<[string], { kind: string }>(
        "SELECT kind FROM events WHERE session_id = ? AND kind IN ('session-start', 'call', 'result', 'turn-end') ORDER BY sequence",
      ).all(sessionId);
      return { answers, kinds: rows.map(row => row.kind) };
    } finally {
      database.close();
    }
  }

  it("records every event under the agent's session id, steers the edit, and gives the note with the first postToolUse only", { timeout: 60_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const { repository, options } = await servedRepository(root);
    await markCursorCloud();

    const { answers: [first, stop, second], kinds } = await editRun(repository, "cloud-session", options);

    expect(kinds).toEqual(["session-start", "call", "result", "turn-end", "result"]);
    expect(String(first!.additional_context)).toMatch(/^Rerail is on in this repo/u);
    expect(JSON.stringify(second)).not.toContain("Rerail is on in this repo");
    expect(JSON.stringify([first, stop, second])).toContain("test:no-print");
  });

  it("runs the same hooks as on a person's machine without the marker: the events and the steer, and no note", { timeout: 60_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const { repository, options } = await servedRepository(root);

    const { answers, kinds } = await editRun(repository, "local-session", options);

    expect(kinds).toEqual(["call", "result", "turn-end", "result"]);
    expect(JSON.stringify(answers)).not.toContain("Rerail is on in this repo");
    expect(JSON.stringify(answers)).toContain("test:no-print");
  });
});
