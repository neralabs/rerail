import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { z } from "zod";
import type { AdapterDelivery } from "../src/adapter-contract.js";
import { claudeHookOutput, handleClaudeEvent, runClaudeHook } from "../src/claude-hook.js";
import type { HookOptions } from "../src/hook-host.js";
import { hookOptions, scriptedLoop, serveText, temporaryDirectories, transcriptQuery, transcriptRows } from "./hook-fixtures.js";

const temporaryDirectory = temporaryDirectories("rerail-claude-hook-");

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "claude-session", requested, mechanism: "additional-context", content, ruleIds };
}

describe("Claude Code one-shot hook", () => {
  it("persists the documented hook lifecycle into the SQLite transcript across separate hook processes", async () => {
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

    expect(transcriptRows(root, ["session-start", "user-prompt", "tool-call", "turn-end"])).toEqual([
      ["claude-session:SessionStart", "session-start"],
      ["claude-session:prompt:p1", "user-prompt"],
      ["claude-session:tool-call:tool-1", "tool-call"],
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
    await expect(readFile(join(root, "rerail.sqlite"))).rejects.toMatchObject({ code: "ENOENT" });
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
    expect(transcriptQuery(root, "SELECT cwd FROM sessions")).toEqual([{ cwd: root }]);
  });

  it("does not start or record hook activity when this checkout is disabled", async () => {
    const root = await temporaryDirectory();
    await mkdir(join(root, ".rerail"));
    await writeFile(join(root, ".rerail", "disabled"), "");

    expect(await runClaudeHook(
      { session_id: "claude-session", cwd: root, hook_event_name: "SessionStart", source: "startup" },
      hookOptions(root),
    )).toEqual({});
    await expect(readFile(join(root, "rerail.sqlite"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("answers {} on stdout and exits cleanly when stdin is malformed or the runtime fails", async () => {
    const root = await temporaryDirectory();
    const serve = (stdin: string, options: HookOptions) => serveText(input => runClaudeHook(input, options), stdin);

    const malformed = await serve("{not json", hookOptions(root));
    expect(JSON.parse(malformed.stdout)).toEqual({});
    expect(malformed.stderr).toMatch(/^rerail hook: /);

    const runtimeFailure = await serve(
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "s", cwd: root, source: "startup" }),
      { storageRoot: join(root, "sessions"), kernelSocketPath: join(root, "missing-kernel.sock") },
    );
    expect(JSON.parse(runtimeFailure.stdout)).toEqual({});
    expect(runtimeFailure.stderr).toMatch(/^rerail hook: /);
  });

  it("routes deliveries through additionalContext, Stop continuation, or the pending file", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const ContextOutput = z.object({
      hookSpecificOutput: z.object({ hookEventName: z.string(), additionalContext: z.string() }),
    });

    const startLoop = scriptedLoop({ "session-start": [delivery("steer", "Read the conventions."), delivery("observe", "hidden")] });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, startLoop, storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: "Read the conventions." },
    });

    const toolLoop = scriptedLoop({
      "tool-call": [delivery("steer", "Careful with auth."), delivery("followUp", "Run the tests."), delivery("nextTurn", "Summarize next time.")],
    });
    const toolOutput = ContextOutput.parse(await handleClaudeEvent({
      ...common,
      hook_event_name: "PreToolUse",
      tool_name: "Edit",
      tool_use_id: "tool-1",
      tool_input: {},
    }, toolLoop, storageRoot));
    expect(toolOutput.hookSpecificOutput).toEqual({ hookEventName: "PreToolUse", additionalContext: "Careful with auth." });

    const displayLoop = scriptedLoop({ "assistant-delta": [delivery("steer", "Deferred steer.")] });
    expect(await handleClaudeEvent({
      ...common,
      hook_event_name: "MessageDisplay",
      message_id: "m1",
      index: "0",
      delta: "text",
      final: true,
    }, displayLoop, storageRoot)).toEqual({});

    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, scriptedLoop({}), storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "Stop", additionalContext: "Run the tests." },
    });

    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({}), storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: "Summarize next time.\n\nDeferred steer." },
    });

    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p2" }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(claudeHookOutput("SessionEnd", [delivery("steer", "ignored")])).toEqual({});
    expect(claudeHookOutput("StopFailure", [delivery("followUp", "ignored")])).toEqual({});
  });

  it("defers next-turn context but continues immediately for a follow-up produced at Stop", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };

    // A rule firing on this prompt asks for "next turn": not this prompt.
    const promptLoop = scriptedLoop({ "user-prompt": [delivery("nextTurn", "Next time, plan first."), delivery("steer", "Now: mind the API.")] });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "go" }, promptLoop, storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: "Now: mind the API." },
    });

    // A follow-up produced at Stop starts another assistant cycle without waiting for a user prompt.
    const stopLoop = scriptedLoop({ "turn-end": [delivery("followUp", "Also run the linter.")] });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, stopLoop, storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "Stop", additionalContext: "Also run the linter." },
    });

    // The next prompt carries only the deferred next-turn context.
    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "again" }, scriptedLoop({}), storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: "Next time, plan first." },
    });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p2" }, scriptedLoop({}), storageRoot)).toEqual({});
  });

  it("discards pending context when Claude reports that the session has ended", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };

    await handleClaudeEvent(
      { ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "first" },
      scriptedLoop({ "user-prompt": [delivery("nextTurn", "Do not reuse this.")] }),
      storageRoot,
    );
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
