import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { sessionStartNote, steerBlock, type AdapterDelivery, type AdapterEvent, type RerailEventLoop } from "../../../src/engine/index.js";
import { CURSOR_FOLLOWUP_LOOP_LIMIT, cursorHookEvent, cursorHookOutput, handleCursorEvent } from "../../../src/hosts/cursor/cursor-hook.js";
import { cursorWorkspace } from "../../../src/hosts/cursor/cursor-hook-adapter.js";
import { steeredRules } from "../../support/steers.js";
import { answering, countingDaemon } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

async function json(path: string): Promise<unknown> {
  return JSON.parse(await readFile(path, "utf8")) as unknown;
}

const temporaryDirectories: string[] = [];

afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function temporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-cursor-hook-"));
  temporaryDirectories.push(path);
  return path;
}

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "cursor-session", requested, mechanism: "deferred-context", content, ruleIds, rules: steeredRules(ruleIds) };
}

function scriptedLoop(script: Readonly<Record<string, readonly AdapterDelivery[]>>): RerailEventLoop & { events: AdapterEvent[] } {
  const events: AdapterEvent[] = [];
  return {
    events,
    async ingest(event) {
      events.push(event);
      return script[event.kind] ?? [];
    },
  };
}

const ManifestSchema = z.object({
  name: z.literal("rerail"),
  hooks: z.literal("./hooks/cursor.json"),
});
const HooksSchema = z.object({
  version: z.literal(1),
  hooks: z.record(z.string(), z.array(z.object({ command: z.string() }))),
});

describe("Cursor plugin contract", () => {
  it("uses a relative packaged command for every supported hook", async () => {
    expect(ManifestSchema.parse(await json(".cursor-plugin/plugin.json"))).toMatchObject({ name: "rerail" });
    const hooks = HooksSchema.parse(await json("hooks/cursor.json"));
    expect(Object.keys(hooks.hooks).sort()).toEqual([
      "afterAgentResponse",
      "beforeSubmitPrompt",
      "postToolUse",
      "postToolUseFailure",
      "preToolUse",
      "sessionEnd",
      "sessionStart",
      "stop",
    ]);
    for (const handlers of Object.values(hooks.hooks)) {
      expect(handlers).toEqual([{ command: "node ./dist/hosts/cursor/cursor-hook.js" }]);
    }
  });

  it("normalizes documented Cursor hook payloads and supported delivery outputs", () => {
    const common = {
      conversation_id: "cursor-session",
      generation_id: "generation-1",
      model: "cursor-model",
      cursor_version: "1.7.2",
      workspace_roots: ["/workspace"],
      user_email: null,
      transcript_path: null,
    };
    expect(cursorHookEvent({
      ...common,
      hook_event_name: "beforeSubmitPrompt",
      prompt: "Preserve session state",
      attachments: [],
    })).toEqual({
      kind: "user-prompt",
      eventId: "cursor-session:prompt:generation-1",
      sessionId: "cursor-session",
      session: { host: "cursor", sessionId: "cursor-session", cwd: "/workspace" },
      text: "Preserve session state",
    });
    expect(cursorHookEvent({
      ...common,
      hook_event_name: "postToolUse",
      tool_name: "Write",
      tool_input: { path: "src/a.ts" },
      tool_output: "written",
      tool_use_id: "tool-1",
      cwd: "/workspace",
    })).toMatchObject({
      kind: "result",
      toolCallId: "tool-1",
      toolName: "Write",
      output: "written",
      failed: false,
    });

    const delivery = {
      sessionId: "cursor-session",
      requested: "steer" as const,
      mechanism: "deferred-context" as const,
      content: "Preserve the session.",
      ruleIds: [],
      rules: [],
    };
    expect(cursorHookOutput("beforeSubmitPrompt", [delivery])).toEqual({ continue: true });
    expect(cursorHookOutput("postToolUse", [delivery])).toEqual({ additional_context: "Preserve the session." });
    expect(cursorHookOutput("postToolUse", [{ ...delivery, requested: "block" }])).toEqual({ additional_context: "Preserve the session." });
    expect(cursorHookOutput("stop", [delivery])).toEqual({ followup_message: "Preserve the session." });
    expect(cursorHookOutput("stop", [])).toEqual({});
  });

  it("reads a workspace on Windows as a native path, in every form Cursor may send it", () => {
    const root = (value: string) => cursorWorkspace({ hook_event_name: "sessionStart", workspace_roots: [value] }, "win32");
    for (const form of ["file:///c:/Users/me/repo", "file:///c%3A/Users/me/repo", "/c:/Users/me/repo", "/c/Users/me/repo", "c:\\Users\\me\\repo", "C:/Users/me/repo"]) {
      expect(root(form)).toBe("C:\\Users\\me\\repo");
    }
    expect(cursorWorkspace({ hook_event_name: "postToolUse", cwd: "/d/work", workspace_roots: ["C:\\repo"] }, "win32")).toBe("D:\\work");
    // A workspace at a drive's root keeps the separator: `C:` alone would name the drive's current folder.
    for (const form of ["file:///c:/", "/c:", "/c"]) expect(root(form)).toBe("C:\\");
    expect(root("\\\\server\\share\\repo")).toBe("\\\\server\\share\\repo");
    expect(root("/workspace")).toBe("/workspace");
    expect(cursorWorkspace({ workspace_roots: ["file:///home/me/my%20repo"] }, "linux")).toBe("/home/me/my repo");
    expect(cursorWorkspace({ workspace_roots: ["/c/repo"] }, "linux")).toBe("/c/repo");
    // A URI that names another host has no path on this machine, so it stays as sent.
    expect(cursorWorkspace({ workspace_roots: ["file://server/share/repo"] }, "linux")).toBe("file://server/share/repo");
  });

  it("maps Cursor session and assistant lifecycle hooks to the shared session event vocabulary", () => {
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", workspace_roots: ["/workspace"] };

    expect(cursorHookEvent({ ...common, hook_event_name: "sessionStart" })).toMatchObject({
      kind: "session-start",
      eventId: "cursor-session:session-start:generation-1",
      session: { host: "cursor", sessionId: "cursor-session", cwd: "/workspace" },
    });
    expect(cursorHookEvent({ ...common, hook_event_name: "afterAgentResponse", text: "Completed." })).toMatchObject({
      kind: "assistant-message",
      eventId: "cursor-session:assistant:generation-1",
      messageId: "generation-1",
      text: "Completed.",
    });
    expect(cursorHookEvent({ ...common, hook_event_name: "stop" })).toMatchObject({
      kind: "turn-end",
      eventId: "cursor-session:turn:generation-1",
      text: "",
    });
    expect(cursorHookEvent({ ...common, hook_event_name: "sessionEnd" })).toMatchObject({
      kind: "session-end",
      eventId: "cursor-session:session-end:generation-1",
    });
  });

  it("builds the session note and links a steer from the daemon's answer, asking it nothing more, and reads an older daemon after it", async () => {
    const root = await temporaryDirectory();
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    const daemon = await countingDaemon(home);
    try {
      const common = { conversation_id: "cursor-session", cwd: root, workspace_roots: [root] };
      const dashboard = "http://127.0.0.1:4791/?repository=clone%3Aapp&checkout=%2Fwork%2Fapp";
      const sessionNote = { setUp: true as const, ruleCount: 3 };
      expect(await handleCursorEvent({ ...common, hook_event_name: "sessionStart" }, answering(scriptedLoop({}), { dashboard, sessionNote }), join(root, "sessions")))
        .toMatchObject({ additional_context: sessionStartNote(dashboard, "cursor", "cursor-session", sessionNote) });

      const steer = [delivery("steer", "Remove print.", ["acme:no-print"])];
      const result = { ...common, hook_event_name: "postToolUse", tool_name: "Write", tool_input: {}, tool_output: "written" };
      await handleCursorEvent({ ...result, tool_use_id: "write-1" }, answering(scriptedLoop({ result: steer }), { dashboard }), join(root, "sessions"));
      expect(daemon.requests).toEqual([]);

      // A daemon older than these facts answers without them: the hook reads the dashboard from it, as before.
      await handleCursorEvent({ ...result, tool_use_id: "write-2" }, scriptedLoop({ result: steer }), join(root, "sessions"));
      expect(daemon.requests).toContain("/api/repository");
    } finally {
      await daemon.close();
    }
  });

  it("links a steer it holds with the answer that found it, so the hook that carries it asks the daemon nothing after its answer", async () => {
    const root = await temporaryDirectory();
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    const daemon = await countingDaemon(home);
    try {
      const common = { conversation_id: "cursor-session", cwd: root, workspace_roots: [root], tool_name: "Shell", tool_input: { command: "ls" } };
      const dashboard = "http://127.0.0.1:4791/?repository=clone%3Aapp&checkout=%2Fwork%2Fapp";
      const steer = [delivery("steer", "Remove print.", ["acme:no-print"])];
      await handleCursorEvent({ ...common, tool_use_id: "t1", hook_event_name: "preToolUse" }, answering(scriptedLoop({ call: steer }), { dashboard }), join(root, "sessions"));
      await handleCursorEvent({ ...common, hook_event_name: "beforeSubmitPrompt", prompt: "next" }, answering(scriptedLoop({ "user-prompt": steer }), { dashboard }), join(root, "sessions"));

      // The result's answer links nothing of its own, so it carries no dashboard.
      const result = await handleCursorEvent({ ...common, tool_use_id: "t1", hook_event_name: "postToolUse", tool_output: "a" }, scriptedLoop({}), join(root, "sessions"));
      // Either steer held unlinked would have the hook read the dashboard from the daemon.
      expect(JSON.stringify(result)).toContain(`${dashboard}#rule/acme%3Ano-print`);
      expect(daemon.requests).toEqual([]);
    } finally {
      await daemon.close();
    }
  });

  it("links a steer the transcript's lines found with the answer that found it, so the hook that carries it asks the daemon nothing after its answer", async () => {
    const root = await temporaryDirectory();
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    const daemon = await countingDaemon(home);
    try {
      const transcript = join(root, "transcript.jsonl");
      await writeFile(transcript, `${JSON.stringify({ role: "assistant", message: { content: [{ type: "text", text: "answer one" }] } })}\n`);
      const common = { conversation_id: "cursor-session", cwd: root, transcript_path: transcript, tool_use_id: "t1", tool_name: "Shell", tool_input: { command: "ls" } };
      const dashboard = "http://127.0.0.1:4791/?repository=clone%3Aapp&checkout=%2Fwork%2Fapp";
      const steer = [delivery("steer", "Remove print.", ["acme:no-print"])];
      await handleCursorEvent({ ...common, hook_event_name: "preToolUse" }, answering(scriptedLoop({ "assistant-message": steer }), { dashboard }), join(root, "sessions"));

      // The result's answer links nothing of its own, so it carries no dashboard.
      const result = await handleCursorEvent({ ...common, hook_event_name: "postToolUse", tool_output: "a" }, scriptedLoop({}), join(root, "sessions"));
      expect(JSON.stringify(result)).toContain(`${dashboard}#rule/acme%3Ano-print`);
      expect(daemon.requests).toEqual([]);
    } finally {
      await daemon.close();
    }
  });

  it("preserves pre-tool code rule violations until Cursor can expose them after the tool", async () => {
    const root = await temporaryDirectory();
    const storage = join(root, "sessions");
    const common = {
      conversation_id: "cursor-session",
      cwd: root,
      tool_use_id: "write-1",
      tool_name: "Write",
      tool_input: { file_path: "api/route.py", content: "cursor.execute('SELECT 1')" },
    };
    const diagnostic = "CURSOR_EXECUTE: Direct cursor access bypasses the typed API.";
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "preToolUse" },
      scriptedLoop({ call: [delivery("steer", diagnostic)] }),
      storage,
    )).toEqual({ permission: "allow" });
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "postToolUse", tool_output: "written" },
      scriptedLoop({}),
      storage,
    )).toEqual({ additional_context: diagnostic });
    expect(await handleCursorEvent(
      { ...common, tool_use_id: "read-2", tool_name: "Read", hook_event_name: "postToolUse" },
      scriptedLoop({}),
      storage,
    )).toEqual({});
  });

  it("durably carries prompt steers to the next tool result past a stop at the follow-up bound, then clears them when the session ends", async () => {
    const root = await temporaryDirectory();
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", workspace_roots: [root] };
    const toolResult = { ...common, hook_event_name: "postToolUse", tool_use_id: "read-1", tool_name: "Read", tool_output: "ok" };
    const promptLoop = scriptedLoop({
      "user-prompt": [delivery("steer", "Continue the plan."), delivery("block", "Keep the API typed."), delivery("log", "not user-visible")],
    });

    expect(await handleCursorEvent(
      { ...common, hook_event_name: "beforeSubmitPrompt", prompt: "Start work" },
      promptLoop,
      join(root, "sessions"),
    )).toEqual({ continue: true });
    expect(promptLoop.events.map(event => event.kind)).toEqual(["user-prompt"]);
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "stop", loop_count: CURSOR_FOLLOWUP_LOOP_LIMIT },
      scriptedLoop({}),
      join(root, "sessions"),
    )).toEqual({});
    expect(await handleCursorEvent(toolResult, scriptedLoop({}), join(root, "sessions"))).toEqual({
      additional_context: "Continue the plan.\n\nKeep the API typed.",
    });
    expect(await handleCursorEvent(toolResult, scriptedLoop({}), join(root, "sessions"))).toEqual({});

    await handleCursorEvent(
      { ...common, hook_event_name: "beforeSubmitPrompt", prompt: "Another turn" },
      scriptedLoop({ "user-prompt": [delivery("steer", "Discard this.")] }),
      join(root, "sessions"),
    );
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "sessionEnd" },
      scriptedLoop({}),
      join(root, "sessions"),
    )).toEqual({});
    expect(await handleCursorEvent(toolResult, scriptedLoop({}), join(root, "sessions"))).toEqual({});
  });

  it("with the experimental digest on, follows the end of the agent's run up with the steers its turn end carries, until the follow-ups reach the bound", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const root = await temporaryDirectory();
    const storage = join(root, "sessions");
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", workspace_roots: [root], hook_event_name: "stop" };
    const digest = steerBlock({ kind: "late" }, [{ ruleId: "test:no-bad", mode: "steer", message: "Do not write bad.", location: "a.ts:2:7" }]);
    const stopLoop = scriptedLoop({ "turn-end": [delivery("steer", digest, ["test:no-bad"])] });

    const followUp = await handleCursorEvent({ ...common, loop_count: 0 }, stopLoop, storage);
    expect(stopLoop.events.map(event => event.kind)).toEqual(["turn-end"]);
    expect(followUp).toEqual({ followup_message: expect.stringMatching(/^Rerail finished checking your earlier edits/u) });
    expect(followUp.followup_message).toContain("- a.ts:2:7 — Do not write bad. [test:no-bad]");
    // The person sees none of a hook's text in Cursor, so the agent is asked to relay the rule's line (with no dashboard
    // running here, the rule stays unlinked and the line carries no link).
    expect(followUp.followup_message).toContain("Begin your next reply with these lines, exactly as written:\n◆ Rerail · test:no-bad  steered");
    // Carried once.
    expect(await handleCursorEvent({ ...common, loop_count: 1 }, scriptedLoop({}), storage)).toEqual({});

    // At the bound the turn end still reaches Rerail, and its steer waits for the next tool result.
    expect(await handleCursorEvent({ ...common, loop_count: CURSOR_FOLLOWUP_LOOP_LIMIT }, stopLoop, storage)).toEqual({});
    const toolResult = { ...common, hook_event_name: "postToolUse", tool_use_id: "read-1", tool_name: "Read", tool_output: "ok" };
    expect(await handleCursorEvent(toolResult, scriptedLoop({}), storage)).toEqual({
      additional_context: expect.stringContaining("- a.ts:2:7 — Do not write bad. [test:no-bad]"),
    });
  });

  it("with the experimental digest off, answers the end of the agent's run with nothing and keeps its steers for the next tool result", async () => {
    const root = await temporaryDirectory();
    const storage = join(root, "sessions");
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", workspace_roots: [root], hook_event_name: "stop" };
    expect(await handleCursorEvent({ ...common, loop_count: 0 }, scriptedLoop({ "turn-end": [delivery("steer", "Do not write bad.")] }), storage)).toEqual({});
    const toolResult = { ...common, hook_event_name: "postToolUse", tool_use_id: "read-1", tool_name: "Read", tool_output: "ok" };
    expect(await handleCursorEvent(toolResult, scriptedLoop({}), storage)).toEqual({ additional_context: expect.stringContaining("Do not write bad.") });
  });
});

describe("a Cursor hook while the daemon is not reading", () => {
  /** A transcript-mode session (no beforeSubmitPrompt) whose transcript holds a prompt and an answer no hook has read. */
  async function unreadTranscript(): Promise<{ storage: string; common: Record<string, unknown> }> {
    const root = await temporaryDirectory();
    const transcript = join(root, "transcript.jsonl");
    const line = (role: string, text: string) => JSON.stringify({ role, message: { content: [{ type: "text", text }] } });
    await writeFile(transcript, `${line("user", "<timestamp>t</timestamp>\n<user_query>\nprompt one\n</user_query>")}\n${line("assistant", "answer one")}\n`);
    return { storage: join(root, "sessions"), common: { conversation_id: "cursor-session", generation_id: "g1", cwd: root, transcript_path: transcript } };
  }

  /** A loop that leaves each event for a daemon that is not reading, as a hook does once one was left untaken. */
  function untakenLoop(): RerailEventLoop & { events: AdapterEvent[] } {
    return { ...scriptedLoop({}), untaken: true };
  }

  it("answers as with nothing to deliver, keeping the steers held for the session for the next hook the daemon answers", async () => {
    const root = await temporaryDirectory();
    const storage = join(root, "sessions");
    const common = { conversation_id: "cursor-held", generation_id: "g1", cwd: root };
    const tool = (id: string) => ({ tool_use_id: id, tool_name: "Shell", tool_input: { command: "ls" } });
    const steer: AdapterDelivery = { sessionId: "cursor-held", requested: "steer", mechanism: "deferred-context", content: "Held steer.", ruleIds: [], rules: [] };
    // Cursor shows nothing at a call, so its steer waits for a result that can carry it.
    expect(await handleCursorEvent({ ...common, ...tool("t1"), hook_event_name: "preToolUse" }, scriptedLoop({ call: [steer] }), storage)).toEqual({ permission: "allow" });
    const left = untakenLoop();
    // Before, the hook routed the held steer into its own answer and emptied the session's file, under the lock.
    expect(await handleCursorEvent({ ...common, ...tool("t1"), hook_event_name: "postToolUse", tool_output: "a" }, left, storage)).toEqual({});
    expect(await handleCursorEvent({ ...common, ...tool("t2"), hook_event_name: "postToolUse", tool_output: "a" }, scriptedLoop({}), storage))
      .toEqual({ additional_context: expect.stringContaining("Held steer.") });
  });

  it("leaves the transcript unread for the next hook the daemon answers, and leaves only the hook's own event", async () => {
    const { storage, common } = await unreadTranscript();
    const tool = { tool_use_id: "shell-1", tool_name: "Shell", tool_input: { command: "ls" } };
    const left = untakenLoop();
    expect(await handleCursorEvent({ ...common, ...tool, hook_event_name: "preToolUse" }, left, storage)).toEqual({ permission: "allow" });
    expect(left.events.map(event => event.kind)).toEqual(["call"]);
    const answer = untakenLoop();
    expect(await handleCursorEvent({ ...common, hook_event_name: "afterAgentResponse", text: "answer one" }, answer, storage)).toEqual({});
    expect(answer.events).toEqual([]);
    const live = scriptedLoop({});
    await handleCursorEvent({ ...common, ...tool, tool_use_id: "shell-2", hook_event_name: "preToolUse" }, live, storage);
    expect(live.events.map(event => event.kind)).toEqual(["user-prompt", "assistant-message", "call"]);
  });

  /** The kinds of event each hook of `inputs` leaves, or the daemon takes, in order, one loop a hook. */
  async function kindsLeft(inputs: readonly Record<string, unknown>[], storage: string, untaken: boolean): Promise<string[]> {
    const kinds: string[] = [];
    for (const input of inputs) {
      const loop = untaken ? untakenLoop() : scriptedLoop({});
      await handleCursorEvent(input, loop, storage);
      kinds.push(...loop.events.map(event => event.kind));
    }
    return kinds;
  }

  it("leaves every event of a turn whose session sends its own text, as the daemon takes them when it reads", async () => {
    const { storage, common } = await unreadTranscript();
    const turn = (session: string) => [
      { ...common, conversation_id: session, hook_event_name: "beforeSubmitPrompt", prompt: "prompt one" },
      { ...common, conversation_id: session, hook_event_name: "afterAgentResponse", text: "answer one" },
      { ...common, conversation_id: session, hook_event_name: "stop", status: "completed", loop_count: 0 },
    ];
    const taken = await kindsLeft(turn("cursor-taken"), storage, false);
    expect(taken).toEqual(["user-prompt", "assistant-message", "turn-end"]);
    expect(await kindsLeft(turn("cursor-left"), storage, true)).toEqual(taken);
  });

  it("leaves the hook's text event where the transcript it names is not there yet, as the daemon takes it when it reads", async () => {
    const root = await temporaryDirectory();
    const storage = join(root, "sessions");
    const answer = (session: string) => [{ conversation_id: session, generation_id: "g1", cwd: root, transcript_path: join(root, "none.jsonl"), hook_event_name: "afterAgentResponse", text: "answer one" }];
    const taken = await kindsLeft(answer("cursor-taken"), storage, false);
    expect(taken).toEqual(["assistant-message"]);
    expect(await kindsLeft(answer("cursor-left"), storage, true)).toEqual(taken);
  });
});
