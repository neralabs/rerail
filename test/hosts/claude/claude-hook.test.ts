import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { type AdapterDelivery, type RerailEventLoop, sessionStartNote, steerBlock } from "../../../src/engine/index.js";
import { claudeHookOutput, handleClaudeEvent } from "../../../src/hosts/claude/claude-hook-adapter.js";
import { runClaudeHook } from "../../../src/hosts/claude/claude-hook.js";
import { bindSteerReceipt, type HookInput, type HookOptions } from "../../../src/harness/index.js";
import { dashboardOrigin, presentedSessionNoteRules } from "../../../src/presentation/index.js";
import { hookOptions, scriptedLoop, serveText, temporaryDirectories, sessionDatabaseQuery, eventRows } from "../../support/hook-events.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";

const temporaryDirectory = temporaryDirectories("rerail-claude-hook-");

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "claude-session", requested, mechanism: "additional-context", content, ruleIds };
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

  it("links the session note and a steer's Why? to the checkout's dashboard when the daemon is on loopback", async () => {
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

      const steer = scriptedLoop({ result: [delivery("steer", steerBlock("acme:no-print", "Remove print.", { mode: "steer" }), ["acme:no-print"])] });
      const output = await handleClaudeEvent({
        ...common, hook_event_name: "PostToolUse", tool_name: "Write", tool_use_id: "tool-1", tool_input: {}, tool_response: "ok",
      }, steer, join(root, "sessions"), { TERM_PROGRAM: "iTerm.app", COLORTERM: "truecolor" });
      expect(output).toMatchObject({
        // Claude Code draws ANSI colour, so Rerail is its coloured mark, each letter its own colour.
        systemMessage: expect.stringMatching(/^◆ \u001b\[38;2;165;173;251mR\u001b\[39m .* · acme:no-print {2}steered {2}· {2}\u001b\]8;;http:\/\/127\.0\.0\.1:4791\/#rule\/acme%3Ano-print\u0007why\?\u001b\]8;;\u0007$/u),
        hookSpecificOutput: {
          hookEventName: "PostToolUse",
          additionalContext: expect.stringContaining("Remove print.\nWhy? → http://127.0.0.1:4791/#rule/acme%3Ano-print"),
        },
      });
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("shows a late steer's line at Stop and gives the coding agent its block with the next message, without the line again", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const blocks = [
      delivery("steer", steerBlock("a", "Fix a.", { mode: "steer" }), ["a"]),
      delivery("steer", steerBlock("b", "Fix b.", { mode: "steer" }), ["b"]),
    ];
    // Without a daemon the links stay relative, so each line is the header alone; without colour the mark is plain.
    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, scriptedLoop({ "turn-end": blocks }), storageRoot, { NO_COLOR: "1" }))
      .toEqual({ systemMessage: "\n◆ R E R A I L · a  steered\n◆ R E R A I L · b  steered" });

    const next = scriptedLoop({ "user-prompt": [delivery("steer", steerBlock("c", "Fix c.", { mode: "steer" }), ["c"])] });
    const output = await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, next, storageRoot, { NO_COLOR: "1" });
    expect(output).toEqual({
      systemMessage: "◆ R E R A I L · c  steered",
      hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: expect.stringMatching(/Fix a\.[\s\S]*Fix b\.[\s\S]*Fix c\./u) },
    });
  });

  it("counts a late steer once, when the person sees its line at Stop, not again when its block reaches the agent", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const receipts: Array<readonly string[]> = [];
    const serve = (input: HookInput, loop: RerailEventLoop) => serveText(
      parsed => handleClaudeEvent(parsed, loop, storageRoot, { NO_COLOR: "1" }).then(output => bindSteerReceipt(output, (_receipt, ruleIds) => { receipts.push(ruleIds); })),
      JSON.stringify(input),
    );
    const late = [delivery("steer", steerBlock("a", "Fix a.", { mode: "steer" }), ["a"]), delivery("steer", steerBlock("b", "Fix b.", { mode: "steer" }), ["b"])];

    await serve({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, scriptedLoop({ "turn-end": late }));
    expect(receipts).toEqual([["a", "b"]]);

    await serve({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({ "user-prompt": [delivery("steer", steerBlock("c", "Fix c.", { mode: "steer" }), ["c"])] }));
    expect(receipts).toEqual([["a", "b"], ["c"]]);
  });

  it("prints the why link after the word when the terminal cannot draw links", () => {
    const block = steerBlock("a", "Fix a.", { mode: "steer" }).replace("#rule/", "http://127.0.0.1:4791/#rule/");
    expect(claudeHookOutput("PostToolUse", [delivery("steer", block, ["a"])], { links: "plain" })).toMatchObject({
      systemMessage: "◆ Rerail · a  steered  ·  why? → http://127.0.0.1:4791/#rule/a",
    });
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
