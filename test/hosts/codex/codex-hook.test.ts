import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { type AdapterDelivery, sessionStartNote, steerBlock } from "../../../src/engine/index.js";
import { codexHookOutput, handleCodexEvent } from "../../../src/hosts/codex/codex-hook-adapter.js";
import { runCodexHook } from "../../../src/hosts/codex/codex-hook.js";
import type { HookOptions } from "../../../src/harness/index.js";
import { dashboardOrigin, presentedSessionNoteRules } from "../../../src/presentation/index.js";
import { hookOptions, scriptedLoop, serveText, temporaryDirectories, sessionDatabaseQuery, eventRows } from "../../support/hook-events.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";

const temporaryDirectory = temporaryDirectories("rerail-codex-hook-");

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "codex-session", requested, mechanism: "additional-context", content, ruleIds };
}

describe("Codex one-shot hook", () => {
  it("persists the documented hook lifecycle into the SQLite session store across separate hook processes", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const common = { session_id: "codex-session", cwd: root, transcript_path: null, model: "gpt-5-codex", permission_mode: "default" };

    const note = sessionStartNote(await dashboardOrigin(root), "codex", "codex-session", await presentedSessionNoteRules({ host: "codex", sessionId: "codex-session", cwd: root }));
    expect(await runCodexHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options)).toEqual({
      hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: note },
    });
    expect(await runCodexHook({ ...common, hook_event_name: "UserPromptSubmit", turn_id: "t1", prompt: "Refactor auth" }, options)).toEqual({});
    expect(await runCodexHook({
      ...common,
      hook_event_name: "PreToolUse",
      turn_id: "t1",
      tool_name: "Bash",
      tool_use_id: "call-1",
      tool_input: { command: "ls" },
    }, options)).toEqual({});
    expect(await runCodexHook({
      ...common,
      hook_event_name: "Stop",
      turn_id: "t1",
      stop_hook_active: false,
      last_assistant_message: "Done.",
    }, options)).toEqual({});

    expect(eventRows(root, ["session-start", "user-prompt", "call", "assistant-message", "turn-end"])).toEqual([
      ["codex-session:session-start:startup", "session-start"],
      ["codex-session:prompt:t1", "user-prompt"],
      ["codex-session:tool-call:call-1", "call"],
      ["codex-session:assistant:t1", "assistant-message"],
      ["codex-session:turn:t1", "turn-end"],
    ]);
  });

  it("ignores hook events the adapter does not model without touching storage", async () => {
    const root = await temporaryDirectory();
    const output = await runCodexHook(
      { session_id: "codex-session", cwd: root, hook_event_name: "PreCompact", turn_id: "t1", trigger: "auto" },
      hookOptions(root),
    );
    expect(output).toEqual({});
    expect(sessionDatabaseQuery(root, "SELECT session_id FROM sessions")).toEqual([]);
  });

  it("does not start or record hook activity where Rerail is not started", async () => {
    const root = await temporaryDirectory({ started: false });
    const start = { session_id: "codex-session", cwd: root, hook_event_name: "SessionStart", source: "startup" };

    expect(await runCodexHook(start, hookOptions(root))).toEqual({});
    await expect(readFile(join(root, "rerail.sqlite"))).rejects.toMatchObject({ code: "ENOENT" });

    startTestRepository(root, hookOptions(root).databasePath);
    stopTestRepository(root, hookOptions(root).databasePath);
    expect(await runCodexHook(start, hookOptions(root))).toEqual({});
    expect(sessionDatabaseQuery(root, "SELECT session_id FROM sessions")).toEqual([]);
  });


  it("answers {} on stdout and exits cleanly when stdin is malformed or a hook after the session's start fails", async () => {
    const root = await temporaryDirectory();
    // The runtime failures below read the default database, so the repository is started there too.
    startTestRepository(root);
    const serve = (stdin: string, options: HookOptions) => serveText(input => runCodexHook(input, options), stdin);
    const unreachable = { storageRoot: join(root, "sessions"), kernelSocketPath: join(root, "missing-engine.sock") };

    const malformed = await serve("[]", hookOptions(root));
    expect(JSON.parse(malformed.stdout)).toEqual({});
    expect(malformed.stderr).toMatch(/^rerail hook: /);

    const toolFailure = await serve(JSON.stringify({
      hook_event_name: "PostToolUse", session_id: "s", cwd: root, turn_id: "t", tool_name: "Bash", tool_use_id: "c", tool_response: "ok",
    }), unreachable);
    expect(JSON.parse(toolFailure.stdout)).toEqual({});
    expect(toolFailure.stderr).toMatch(/^rerail hook: /);
  });

  it("tells the person once, when the session starts, that Rerail is not running, since Codex has no footer", async () => {
    const root = await temporaryDirectory();
    startTestRepository(root);
    const start = await serveText(input => runCodexHook(input, { storageRoot: join(root, "sessions"), kernelSocketPath: join(root, "missing-engine.sock") }),
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "s", cwd: root, source: "startup" }));
    expect(JSON.parse(start.stdout)).toEqual({ systemMessage: "Rerail ✕ not running · start it with: rerail daemon start" });
  });

  it("shows one plain line per steered rule with the link printed, and holds Stop on the block alone", () => {
    const linked = (id: string) => steerBlock(id, `Fix ${id}.`, { mode: "steer" }).replace("#rule/", "http://127.0.0.1:4791/#rule/");
    const steers = [delivery("steer", linked("a"), ["a"]), delivery("steer", linked("b"), ["b"])];
    expect(codexHookOutput("PostToolUse", steers)).toMatchObject({
      systemMessage: "◆ Rerail · a  steered  ·  why → http://127.0.0.1:4791/#rule/a\n◆ Rerail · b  steered  ·  why → http://127.0.0.1:4791/#rule/b",
    });
    expect(codexHookOutput("Stop", steers)).toEqual({ decision: "block", reason: `${linked("a")}\n\n${linked("b")}` });
  });

  it("routes steer and block deliveries through additionalContext, Stop continuation, or the pending file", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "codex-session", cwd: root };

    const startLoop = scriptedLoop({ "session-start": [delivery("steer", "Read the conventions."), delivery("log", "hidden")] });
    expect(await handleCodexEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, startLoop, storageRoot)).toEqual({
      hookSpecificOutput: {
        hookEventName: "SessionStart",
        additionalContext: `${sessionStartNote(await dashboardOrigin(root), "codex", "codex-session", await presentedSessionNoteRules({ host: "codex", sessionId: "codex-session", cwd: root }))}\n\nRead the conventions.`,
      },
    });

    const toolLoop = scriptedLoop({
      result: [delivery("steer", "Generated file."), delivery("block", "Run the tests."), delivery("log", "Recorded only.")],
    });
    expect(await handleCodexEvent({
      ...common,
      hook_event_name: "PostToolUse",
      turn_id: "t1",
      tool_name: "Bash",
      tool_use_id: "call-1",
      tool_response: "ok",
    }, toolLoop, storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext: "Generated file.\n\nRun the tests." },
    });

    const stopLoop = scriptedLoop({ "turn-end": [delivery("steer", "Deferred steer."), delivery("block", "Deferred block.")] });
    expect(await handleCodexEvent({
      ...common,
      hook_event_name: "Stop",
      turn_id: "t1",
      last_assistant_message: "Done.",
    }, stopLoop, storageRoot)).toEqual({
      decision: "block",
      reason: "Deferred steer.\n\nDeferred block.",
    });
    expect(stopLoop.events.map(event => event.kind)).toEqual(["assistant-message", "turn-end"]);

    expect(await handleCodexEvent(
      { ...common, hook_event_name: "UserPromptSubmit", turn_id: "t2", prompt: "next" },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});

    expect(await handleCodexEvent({ ...common, hook_event_name: "Stop", turn_id: "t2", last_assistant_message: "Done." }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(codexHookOutput("SessionEnd", [delivery("steer", "ignored")])).toEqual({});
  });

  it("delivers a tool's steering at its own PostToolUse, leaving nothing for the next tool call or Stop", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "codex-session", cwd: root, turn_id: "t1" };

    expect(await handleCodexEvent(
      { ...common, hook_event_name: "PostToolUse", tool_name: "Bash", tool_use_id: "call-1", tool_response: "ok" },
      scriptedLoop({ result: [delivery("block", "Revert the generated edit.")] }),
      storageRoot,
    )).toEqual({ hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext: "Revert the generated edit." } });
    expect(await handleCodexEvent(
      { ...common, hook_event_name: "PreToolUse", tool_name: "Bash", tool_use_id: "call-2", tool_input: { command: "ls" } },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
    expect(await handleCodexEvent({ ...common, hook_event_name: "Stop", last_assistant_message: "Done." }, scriptedLoop({}), storageRoot)).toEqual({});
  });
});
