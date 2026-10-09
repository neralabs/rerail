import { describe, expect, it, vi } from "vitest";
import type { AdapterEvent } from "../src/adapter-contract.js";
import { claudeHookEvents, createClaudeEventHandler } from "../src/claude-event-adapter.js";
import { codexHookEvents, createCodexEventHandler } from "../src/codex-event-adapter.js";

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
    expect(codexHookEvents({ ...input, stop_hook_active: true }).map(event => event.eventId)).toEqual([
      "codex-1:assistant:turn-1:continued",
      "codex-1:turn:turn-1:continued",
    ]);
  });
});
