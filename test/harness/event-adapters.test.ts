import { describe, expect, it, vi } from "vitest";
import type { AdapterEvent } from "../../src/engine/index.js";
import { claudeHookEvents, createClaudeEventHandler } from "../../src/hosts/claude/index.js";
import { codexHookEvents, createCodexEventHandler } from "../../src/hosts/codex/index.js";

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

  it("takes a Claude MessageDisplay index only as the whole number from 0 that Claude Code sends", () => {
    const batch = { hook_event_name: "MessageDisplay", session_id: "claude-1", cwd: "/workspace", message_id: "m1", delta: "Done.", final: true };
    expect(claudeHookEvents({ ...batch, index: 2 }).map(event => event.eventId)).toEqual(["claude-1:message:m1:2"]);
    expect(() => claudeHookEvents(batch)).toThrow("Claude hook omitted index");
    for (const index of ["0", -1, 1.5, null, true]) {
      expect(() => claudeHookEvents({ ...batch, index }), String(index)).toThrow("Claude hook index must be a whole number from 0");
    }
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
