import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { z } from "zod";
import type { AdapterDelivery } from "../src/adapter-contract.js";
import { claudeHookOutput, handleClaudeEvent, runClaudeHook } from "../src/claude-hook.js";
import type { HookOptions } from "../src/hook-host.js";
import { hookOptions, scriptedLoop, serveText, temporaryDirectories, sessionDatabaseQuery, eventRows } from "./hook-fixtures.js";
import { startTestRepository, stopTestRepository } from "./support/started-repository.js";

const temporaryDirectory = temporaryDirectories("rerail-claude-hook-");

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "claude-session", requested, mechanism: "additional-context", content, ruleIds };
}

describe("Claude Code one-shot hook", () => {
  it("persists the documented hook lifecycle into the SQLite session store across separate hook processes", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const common = { session_id: "claude-session", cwd: root, transcript_path: "/tmp/transcript.jsonl", permission_mode: "default" };

    expect(await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options)).toEqual({});
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

  it("routes steer and block deliveries through additionalContext or the pending file", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const ContextOutput = z.object({
      hookSpecificOutput: z.object({ hookEventName: z.string(), additionalContext: z.string() }),
    });

    const startLoop = scriptedLoop({ "session-start": [delivery("steer", "Read the conventions."), delivery("log", "hidden")] });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, startLoop, storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: "Read the conventions." },
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
      index: "0",
      delta: "text",
      final: true,
    }, displayLoop, storageRoot)).toEqual({});

    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, scriptedLoop({}), storageRoot)).toEqual({});

    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({}), storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: "Deferred steer.\n\nDeferred block." },
    });

    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p2" }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(claudeHookOutput("SessionEnd", [delivery("steer", "ignored")])).toEqual({});
    expect(claudeHookOutput("Stop", [delivery("block", "ignored")])).toEqual({});
  });

  it("discards pending context when Claude reports that the session has ended", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };

    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "Stop", prompt_id: "p1" },
      scriptedLoop({ "turn-end": [delivery("steer", "Do not reuse this.")] }),
      storageRoot,
    )).toEqual({});
    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "SessionEnd", reason: "clear" },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "new session" },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
  });
});
