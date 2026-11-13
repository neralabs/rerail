import { describe, expect, it, vi } from "vitest";
import type { AdapterEvent } from "../../src/engine/index.js";
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
