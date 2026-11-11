import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { type AdapterDelivery, type RerailEventLoop, sessionStartNote } from "../../../src/engine/index.js";
import { claudeHookOutput, handleClaudeEvent } from "../../../src/hosts/claude/claude-hook-adapter.js";
import { runClaudeHook } from "../../../src/hosts/claude/claude-hook.js";
import { bindSteerReceipt, type HookInput, type HookOptions } from "../../../src/harness/index.js";
import { dashboardOrigin, presentedSessionNoteRules } from "../../../src/presentation/index.js";
import { hookOptions, scriptedLoop, serveText, temporaryDirectories, sessionDatabaseQuery, eventRows } from "../../support/hook-events.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";
import { editSteer, steeredRules } from "../../support/steers.js";

const temporaryDirectory = temporaryDirectories("rerail-claude-hook-");

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = [], rules = steeredRules(ruleIds)): AdapterDelivery {
  return { sessionId: "claude-session", requested, mechanism: "additional-context", content, ruleIds, rules };
}

/** Serves hook events through the harness entry as the hook process does, collecting each steer receipt's rule ids. */
function receiptServer(storageRoot: string) {
  const receipts: Array<readonly string[]> = [];
  const serve = async (input: HookInput, loop: RerailEventLoop) => JSON.parse((await serveText(
    parsed => handleClaudeEvent(parsed, loop, storageRoot, { NO_COLOR: "1" }).then(output => bindSteerReceipt(output, (_receipt, ruleIds) => { receipts.push(ruleIds); })),
    JSON.stringify(input),
  )).stdout) as unknown;
  return { receipts, serve };
}

describe("Claude Code hook without a daemon", () => {
  it("answers {} at once after the session's start, without starting a daemon, so no tool call shows a hook error", async () => {
    const root = await temporaryDirectory();
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    try {
      // The test runtime's embedder is left out, so the hook takes the daemon path a coding agent's hook takes.
      const { embedder: _, ...daemonPath } = hookOptions(root);
      const output = await runClaudeHook({
        session_id: "s", cwd: root, hook_event_name: "PostToolUse", tool_name: "Write", tool_use_id: "t", tool_input: {}, tool_response: "ok",
      }, daemonPath);
      expect(output).toEqual({});
      await expect(readFile(join(process.env.RERAIL_HOME!, "daemon.json"))).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      vi.unstubAllEnvs();
    }
  });
});

describe("Claude Code one-shot hook", () => {
  it("persists the documented hook lifecycle into the SQLite session store across separate hook processes", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const common = { session_id: "claude-session", cwd: root, transcript_path: "/tmp/transcript.jsonl", permission_mode: "default" };

    const note = sessionStartNote(await dashboardOrigin(root), "claude-code", "claude-session", await presentedSessionNoteRules({ host: "claude-code", sessionId: "claude-session", cwd: root }));
    expect(await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options)).toEqual({
      hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: note },
    });
    expect(await runClaudeHook({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "Refactor auth" }, options)).toEqual({});
    expect(await runClaudeHook({
      ...common,
      hook_event_name: "PreToolUse",
      tool_name: "Read",
      tool_use_id: "tool-1",
      tool_input: { file_path: "src/auth.ts" },
    }, options)).toEqual({});
    expect(await runClaudeHook({
      ...common,
      hook_event_name: "Stop",
      prompt_id: "p1",
      stop_hook_active: false,
      last_assistant_message: "Done.",
    }, options)).toEqual({});

    expect(eventRows(root, ["session-start", "user-prompt", "call", "turn-end"])).toEqual([
      ["claude-session:SessionStart", "session-start"],
      ["claude-session:prompt:p1", "user-prompt"],
      ["claude-session:tool-call:tool-1", "call"],
      ["claude-session:turn:p1", "turn-end"],
    ]);
  });

  it("records each Stop of a turn other Stop hooks keep going as a turn end of its own, and one Stop delivered twice once", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const common = { session_id: "claude-session", cwd: root, hook_event_name: "Stop", prompt_id: "p1" };
    await runClaudeHook({ session_id: "claude-session", cwd: root, hook_event_name: "SessionStart", source: "startup" }, options);
    await runClaudeHook({ ...common, stop_hook_active: false, last_assistant_message: "Done." }, options);
    await runClaudeHook({ ...common, stop_hook_active: true, last_assistant_message: "Fixed the test." }, options);
    await runClaudeHook({ ...common, stop_hook_active: true, last_assistant_message: "Goal met." }, options);
    await runClaudeHook({ ...common, stop_hook_active: true, last_assistant_message: "Goal met." }, options);

    const turnEnds = eventRows(root, ["turn-end"]).map(([eventId]) => eventId);
    expect(turnEnds).toEqual([
      "claude-session:turn:p1",
      expect.stringMatching(/^claude-session:turn:p1:continued:/),
      expect.stringMatching(/^claude-session:turn:p1:continued:/),
    ]);
    expect(new Set(turnEnds).size).toBe(3);
  });

  it("records a MessageDisplay batch as Claude Code sends it, with its index a number", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    await runClaudeHook({ session_id: "claude-session", cwd: root, hook_event_name: "SessionStart", source: "startup" }, options);

    // The fields of MessageDisplay's input in Claude Code's hooks reference, with the prompt_id a session has after its
    // first prompt.
    expect(await runClaudeHook({
      session_id: "claude-session",
      transcript_path: "/Users/me/.claude/projects/my-project/transcript.jsonl",
      cwd: root,
      prompt_id: "550e8400-e29b-41d4-a716-446655440000",
      hook_event_name: "MessageDisplay",
      turn_id: "0c9e6a2f-7d41-4f4e-9a15-3f4f7c2b8d10",
      message_id: "5b2a9c8e-1f63-4d8a-b7c4-9e0d2a6f1c3b",
      index: 0,
      final: false,
      delta: "Here is the plan:\n",
    }, options)).toEqual({});

    expect(eventRows(root, ["assistant-delta"])).toEqual([
      ["claude-session:message:5b2a9c8e-1f63-4d8a-b7c4-9e0d2a6f1c3b:0", "assistant-delta"],
    ]);
  });

  it("ignores hook events the adapter does not model without touching storage", async () => {
    const root = await temporaryDirectory();
    const output = await runClaudeHook(
      { session_id: "claude-session", cwd: root, hook_event_name: "Notification", message: "hi" },
      hookOptions(root),
    );
    expect(output).toEqual({});
    expect(sessionDatabaseQuery(root, "SELECT session_id FROM sessions")).toEqual([]);
  });

  it("records the session at CLAUDE_PROJECT_DIR when the hook cwd has moved below it", async () => {
    const root = await temporaryDirectory();
    const nested = join(root, "nested");
    await mkdir(nested);
    process.env.CLAUDE_PROJECT_DIR = root;
    try {
      await runClaudeHook({ session_id: "claude-session", cwd: nested, hook_event_name: "SessionStart", source: "startup" }, hookOptions(root));
    } finally {
      delete process.env.CLAUDE_PROJECT_DIR;
    }
    expect(sessionDatabaseQuery(root, "SELECT cwd FROM sessions")).toEqual([{ cwd: root }]);
  });

  it("does not start or record hook activity where Rerail is not started", async () => {
    const root = await temporaryDirectory({ started: false });
    const start = { session_id: "claude-session", cwd: root, hook_event_name: "SessionStart" as const, source: "startup" };

    expect(await runClaudeHook(start, hookOptions(root))).toEqual({});
    await expect(readFile(join(root, "rerail.sqlite"))).rejects.toMatchObject({ code: "ENOENT" });

    startTestRepository(root, hookOptions(root).databasePath);
    stopTestRepository(root, hookOptions(root).databasePath);
    expect(await runClaudeHook(start, hookOptions(root))).toEqual({});
    expect(sessionDatabaseQuery(root, "SELECT session_id FROM sessions")).toEqual([]);
  });

  it("answers {} on stdout and exits cleanly when stdin is malformed or the runtime fails", async () => {
    const root = await temporaryDirectory();
    // The runtime failure below reads the default database, so the repository is started there too.
    startTestRepository(root);
    const serve = (stdin: string, options: HookOptions) => serveText(input => runClaudeHook(input, options), stdin);

    const malformed = await serve("{not json", hookOptions(root));
    expect(JSON.parse(malformed.stdout)).toEqual({});
    expect(malformed.stderr).toMatch(/^rerail hook: /);

    const runtimeFailure = await serve(
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "s", cwd: root, source: "startup" }),
      { storageRoot: join(root, "sessions"), kernelSocketPath: join(root, "missing-engine.sock") },
    );
    expect(JSON.parse(runtimeFailure.stdout)).toEqual({});
    expect(runtimeFailure.stderr).toMatch(/^rerail hook: /);
  });

  it("links the session note and a steer's why? to the checkout's dashboard when the daemon is on loopback", async () => {
    const root = await temporaryDirectory();
    const home = join(root, "home");
    await mkdir(home);
    await writeFile(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 4791, url: "http://127.0.0.1:4791/",
      version: "test", databasePath: join(home, "rerail.sqlite"), kernelSocketPath: join(home, "kernel.sock"), ui: "bundled", startedAt: 1, packageRoot: home }));
    vi.stubEnv("RERAIL_HOME", home);
    try {
      const common = { session_id: "claude-session", cwd: root };
      const start = await handleClaudeEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, scriptedLoop({}), join(root, "sessions"));
      expect(JSON.stringify(start)).toContain("http://127.0.0.1:4791/?sessionId=claude-session&host=claude-code#sessions");

      const steer = scriptedLoop({ result: [delivery("steer", editSteer("acme:no-print", "Remove print."), ["acme:no-print"])] });
      const output = await handleClaudeEvent({
        ...common, hook_event_name: "PostToolUse", tool_name: "Write", tool_use_id: "tool-1", tool_input: {}, tool_response: "ok",
      }, steer, join(root, "sessions"), { TERM_PROGRAM: "iTerm.app", COLORTERM: "truecolor" });
      expect(output).toMatchObject({
        // Claude Code draws ANSI colour, so Rerail is its coloured mark, each letter its own colour.
        systemMessage: expect.stringMatching(/^◆ \u001b\[38;2;165;173;251mR\u001b\[39m .* · acme:no-print {2}steered {2}· {2}\u001b\]8;;http:\/\/127\.0\.0\.1:4791\/#rule\/acme%3Ano-print\u0007why\?\u001b\]8;;\u0007$/u),
        hookSpecificOutput: {
          hookEventName: "PostToolUse",
          additionalContext: editSteer("acme:no-print", "Remove print."),
        },
      });
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("gives a turn-end steer to the coding agent at Stop, with the person's line and its count, and nothing at the next prompt", async () => {
    const root = await temporaryDirectory();
    const common = { session_id: "claude-session", cwd: root };
    const { receipts, serve } = receiptServer(join(root, "sessions"));
    const turnEnd = [delivery("steer", editSteer("a", "Fix a."), ["a"]), delivery("steer", editSteer("b", "Fix b."), ["b"])];

    // Without a daemon the rules stay unlinked, so each line is the header alone; without colour the mark is plain.
    expect(await serve({ ...common, hook_event_name: "Stop", prompt_id: "p1", stop_hook_active: false }, scriptedLoop({ "turn-end": turnEnd }))).toEqual({
      systemMessage: "\n◆ R E R A I L · a  steered\n◆ R E R A I L · b  steered",
      hookSpecificOutput: { hookEventName: "Stop", additionalContext: [editSteer("a", "Fix a."), editSteer("b", "Fix b.")].join("\n\n") },
    });
    expect(receipts).toEqual([["a", "b"]]);

    // Delivered at Stop, nothing of them is left for the person's next prompt or its first tool call.
    expect(await serve({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({}))).toEqual({});
    expect(await serve({ ...common, hook_event_name: "PreToolUse", tool_name: "Bash", tool_use_id: "tool-2", tool_input: { command: "git log" } }, scriptedLoop({}))).toEqual({});
    expect(receipts).toEqual([["a", "b"]]);
  });

  it("keeps a turn going once: a continued Stop holds its steer, which goes out late with its line and count at the next tool call", async () => {
    const root = await temporaryDirectory();
    const common = { session_id: "claude-session", cwd: root };
    const { receipts, serve } = receiptServer(join(root, "sessions"));

    expect(await serve({ ...common, hook_event_name: "Stop", prompt_id: "p1", stop_hook_active: true }, scriptedLoop({ "turn-end": [delivery("steer", editSteer("a", "Fix a."), ["a"])] })))
      .toEqual({});
    expect(receipts).toEqual([]);

    expect(await serve({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-2", tool_input: {} }, scriptedLoop({}))).toEqual({
      systemMessage: "◆ R E R A I L · a  steered",
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        additionalContext: editSteer("a", "Fix a.").replace("Rerail found 1 problem in your last edit:", "Rerail finished checking your earlier edits after you moved on:"),
      },
    });
    expect(receipts).toEqual([["a"]]);
  });

  it("leaves held steers held and counts nothing at a Stop without prompt_id, as Claude Code before 2.1.196 sends it", async () => {
    const root = await temporaryDirectory();
    const common = { session_id: "claude-session", cwd: root };
    const { receipts, serve } = receiptServer(join(root, "sessions"));
    await serve(
      { ...common, hook_event_name: "MessageDisplay", message_id: "m1", index: 0, delta: "text", final: true },
      scriptedLoop({ "assistant-delta": [delivery("steer", editSteer("a", "Fix a."), ["a"])] }),
    );

    // Claude Code drops Stop context before 2.1.163 and sends no prompt_id before 2.1.196: such a Stop must not
    // spend the held steer on an agent that cannot be told.
    expect(await serve({ ...common, hook_event_name: "Stop", stop_hook_active: false }, scriptedLoop({}))).toEqual({});
    expect(receipts).toEqual([]);

    expect(await serve({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-2", tool_input: {} }, scriptedLoop({})))
      .toEqual({ systemMessage: "◆ R E R A I L · a  steered", hookSpecificOutput: { hookEventName: "PreToolUse", additionalContext: editSteer("a", "Fix a.") } });
    expect(receipts).toEqual([["a"]]);
  });

  it("holds a steer found at the person's prompt, line and all, for the next tool call", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const prompted = scriptedLoop({ "user-prompt": [delivery("steer", editSteer("c", "Fix c."), ["c"])] });

    // UserPromptSubmit's context lands inside the person's turn.
    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "next" }, prompted, storageRoot, { NO_COLOR: "1" })).toEqual({});
    expect(await handleClaudeEvent({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-1", tool_input: {} }, scriptedLoop({}), storageRoot, { NO_COLOR: "1" }))
      .toEqual({ systemMessage: "◆ R E R A I L · c  steered", hookSpecificOutput: { hookEventName: "PreToolUse", additionalContext: editSteer("c", "Fix c.") } });
  });

  it("prints the why link after the word when the terminal cannot draw links", () => {
    const linked = delivery("steer", editSteer("a", "Fix a."), ["a"], steeredRules(["a"], "http://127.0.0.1:4791/"));
    expect(claudeHookOutput("PostToolUse", [linked], { links: "plain" })).toMatchObject({
      systemMessage: "◆ Rerail · a  steered  ·  why? → http://127.0.0.1:4791/#rule/a",
    });
  });

  it("routes steer and block deliveries through additionalContext or the pending file, never the person's prompt", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const ContextOutput = z.object({
      hookSpecificOutput: z.object({ hookEventName: z.string(), additionalContext: z.string() }),
    });

    const startLoop = scriptedLoop({ "session-start": [delivery("steer", "Read the conventions."), delivery("log", "hidden")] });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, startLoop, storageRoot)).toEqual({
      hookSpecificOutput: {
        hookEventName: "SessionStart",
        additionalContext: `${sessionStartNote(await dashboardOrigin(root), "claude-code", "claude-session", await presentedSessionNoteRules({ host: "claude-code", sessionId: "claude-session", cwd: root }))}\n\nRead the conventions.`,
      },
    });

    const toolLoop = scriptedLoop({
      call: [delivery("steer", "Careful with auth."), delivery("block", "Do not edit generated files."), delivery("log", "Recorded only.")],
    });
    const toolOutput = ContextOutput.parse(await handleClaudeEvent({
      ...common,
      hook_event_name: "PreToolUse",
      tool_name: "Edit",
      tool_use_id: "tool-1",
      tool_input: {},
    }, toolLoop, storageRoot));
    expect(toolOutput.hookSpecificOutput).toEqual({
      hookEventName: "PreToolUse",
      additionalContext: "Careful with auth.\n\nDo not edit generated files.",
    });

    const displayLoop = scriptedLoop({ "assistant-delta": [delivery("steer", "Deferred steer."), delivery("block", "Deferred block.")] });
    expect(await handleClaudeEvent({
      ...common,
      hook_event_name: "MessageDisplay",
      message_id: "m1",
      index: 0,
      delta: "text",
      final: true,
    }, displayLoop, storageRoot)).toEqual({});

    // A turn that ends with steers still held hands them to the agent at Stop, which continues the turn.
    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, scriptedLoop({}), storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "Stop", additionalContext: "Deferred steer.\n\nDeferred block." },
    });

    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(await handleClaudeEvent({
      ...common, hook_event_name: "PostToolUse", tool_name: "Read", tool_use_id: "tool-2", tool_input: {}, tool_response: "ok",
    }, scriptedLoop({}), storageRoot)).toEqual({});

    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p2" }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(claudeHookOutput("SessionEnd", [delivery("steer", "ignored")])).toEqual({});
  });

  it("discards pending context when Claude reports that the session has ended", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };

    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "next" },
      scriptedLoop({ "user-prompt": [delivery("steer", "Do not reuse this.")] }),
      storageRoot,
    )).toEqual({});
    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "SessionEnd", reason: "clear" },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-1", tool_input: {} },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
  });
});
