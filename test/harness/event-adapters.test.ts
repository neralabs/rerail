import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import type { AdapterEvent, AdapterSession } from "../../src/engine/index.js";
import { claudeHookEvents, createClaudeEventHandler } from "../../src/hosts/claude/index.js";
import { codexHookEvents, createCodexEventHandler } from "../../src/hosts/codex/index.js";
import { AdapterEventSchema } from "../../src/runtime/index.js";

describe("lifecycle event adapters", () => {
  it("normalizes Claude hook input before calling the event loop", async () => {
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly []>>().mockResolvedValue([]);
    const input = {
      hook_event_name: "PostToolUseFailure",
      session_id: "claude-1",
      cwd: "/workspace",
      tool_use_id: "tool-1",
      tool_name: "Bash",
      error: "exit 1",
    };
    expect(claudeHookEvents(input)).toEqual([{
      kind: "result",
      eventId: "claude-1:tool-result:tool-1",
      toolCallId: "tool-1",
      sessionId: "claude-1",
      session: { host: "claude-code", sessionId: "claude-1", cwd: "/workspace" },
      toolName: "Bash",
      output: "exit 1",
      failed: true,
    }]);
    await createClaudeEventHandler({ ingest })(input);
    expect(ingest).toHaveBeenCalledWith(claudeHookEvents(input)[0]);
  });

  it("keeps the subagent a Claude tool call came from, which reaches Rerail under its parent's session", () => {
    const call = { hook_event_name: "PreToolUse", session_id: "parent", cwd: "/workspace", tool_use_id: "tool-1", tool_name: "Write", tool_input: {} };
    expect(claudeHookEvents({ ...call, agent_id: "agent-a", agent_type: "general-purpose" })).toEqual([expect.objectContaining({ kind: "call", sessionId: "parent", agentId: "agent-a" })]);
    expect(claudeHookEvents(call)[0]).not.toHaveProperty("agentId");
    // The daemon reads the event as the hook sends it.
    expect(AdapterEventSchema.parse(claudeHookEvents({ ...call, agent_id: "agent-a" })[0])).toMatchObject({ agentId: "agent-a" });
  });

  it("builds one event for each Claude Code hook Rerail registers, and refuses any other hook", () => {
    const session: AdapterSession = { host: "claude-code", sessionId: "claude-1", cwd: "/workspace" };
    const own = { sessionId: "claude-1", session };
    const tool = { tool_use_id: "tool-1", tool_name: "Bash" };
    const called = { toolCallId: "tool-1", toolName: "Bash", ...own };
    const hooks: readonly (readonly [string, Readonly<Record<string, unknown>>, AdapterEvent])[] = [
      ["SessionStart", { source: "startup" }, { kind: "session-start", eventId: "claude-1:SessionStart", session }],
      ["SessionEnd", { reason: "exit" }, { kind: "session-end", eventId: "claude-1:SessionEnd", ...own }],
      ["UserPromptSubmit", { prompt_id: "p1", prompt: "Plan it." }, { kind: "user-prompt", eventId: "claude-1:prompt:p1", text: "Plan it.", ...own }],
      ["MessageDisplay", { message_id: "m1", index: "0", delta: "Done.\n", final: false }, {
        kind: "assistant-delta", eventId: "claude-1:message:m1:0", messageId: "m1", delta: "Done.\n", final: false, ...own,
      }],
      ["PreToolUse", { ...tool, tool_input: { command: "ls" } }, { kind: "call", eventId: "claude-1:tool-call:tool-1", input: { command: "ls" }, ...called }],
      ["PostToolUse", { ...tool, tool_response: "a.ts" }, { kind: "result", eventId: "claude-1:tool-result:tool-1", output: "a.ts", failed: false, ...called }],
      ["PostToolUseFailure", { ...tool, error: "exit 1" }, { kind: "result", eventId: "claude-1:tool-result:tool-1", output: "exit 1", failed: true, ...called }],
      ["Stop", { prompt_id: "p1", last_assistant_message: "Done." }, { kind: "turn-end", eventId: "claude-1:turn:p1", text: "Done.", ...own }],
      ["StopFailure", { prompt_id: "p1", error: "rate_limit" }, { kind: "turn-end", eventId: "claude-1:turn-failure:p1", text: "", ...own }],
    ];
    // The plugin's hook file registers the same hooks as set-up (both are pinned in test/setup), and no other.
    const plugin: unknown = JSON.parse(readFileSync(new URL("../../hooks/claude.json", import.meta.url), "utf8"));
    expect(plugin).toEqual({ hooks: Object.fromEntries(hooks.map(([hook]) => [hook, expect.anything()])) });
    for (const [hook, fields, event] of hooks) {
      expect(claudeHookEvents({ hook_event_name: hook, session_id: "claude-1", cwd: "/workspace", ...fields }), hook).toEqual([event]);
    }
    expect(() => claudeHookEvents({ hook_event_name: "Notification", session_id: "claude-1", cwd: "/workspace" }))
      .toThrow("Unsupported Claude hook event: Notification");
  });

  it("takes any text as a Claude MessageDisplay delta, blank or empty included, and rejects a missing one", () => {
    const batch = { hook_event_name: "MessageDisplay", session_id: "claude-1", cwd: "/workspace", message_id: "m1", index: 0, final: true };
    for (const delta of ["", "\n", "Done."]) {
      expect(claudeHookEvents({ ...batch, delta })).toEqual([expect.objectContaining({ kind: "assistant-delta", delta, final: true })]);
    }
    expect(() => claudeHookEvents(batch)).toThrow("Claude hook omitted delta");
    expect(() => claudeHookEvents({ ...batch, delta: 7 })).toThrow("Claude hook delta must be text, not number");
  });

  it("maps one Codex stop hook to ordered assistant and turn events", async () => {
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly []>>().mockResolvedValue([]);
    const input = {
      hook_event_name: "Stop",
      session_id: "codex-1",
      cwd: "/workspace",
      turn_id: "turn-1",
      last_assistant_message: "Completed",
    };
    expect(codexHookEvents(input).map(event => event.kind)).toEqual(["assistant-message", "turn-end"]);
    await createCodexEventHandler({ ingest })(input);
    expect(ingest.mock.calls.map(([event]) => event.eventId)).toEqual([
      "codex-1:assistant:turn-1",
      "codex-1:turn:turn-1",
    ]);
    // Each continuation (`stop_hook_active`) is its own turn: told apart by the message it ended on, so the same Stop
    // delivered twice keeps one id.
    const continued = (message: string) => codexHookEvents({ ...input, stop_hook_active: true, last_assistant_message: message }).map(event => event.eventId);
    const [assistant, turnEnd] = continued("Fixed the test.");
    expect(assistant).toMatch(/^codex-1:assistant:turn-1:continued:/);
    expect(turnEnd).toMatch(/^codex-1:turn:turn-1:continued:/);
    expect(turnEnd!.slice("codex-1:turn:turn-1".length)).toBe(assistant!.slice("codex-1:assistant:turn-1".length));
    expect(continued("Goal met.")[1]).not.toBe(turnEnd);
    expect(continued("Fixed the test.")).toEqual([assistant, turnEnd]);
  });
});
