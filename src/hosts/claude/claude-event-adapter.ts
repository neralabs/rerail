import { existsSync } from "node:fs";
import { isAbsolute } from "node:path";
import { hookText, stopContinuation, type AdapterDelivery, type AdapterEvent, type AdapterSession, type RerailEventLoop } from "../../engine/adapter-contract.js";

export type ClaudeHookInput = Readonly<Record<string, unknown>>;

const { required: requiredText, requiredKey, optional: optionalText } = hookText("Claude");

/** Hook `cwd` follows `/cd`; CLAUDE_PROJECT_DIR stays the launch root that identifies the session. */
export function claudeWorkspace(cwd: unknown): string {
  const projectDir = process.env.CLAUDE_PROJECT_DIR;
  if (projectDir !== undefined && isAbsolute(projectDir) && existsSync(projectDir)) return projectDir;
  return typeof cwd === "string" && cwd.trim() !== "" ? cwd : process.cwd();
}

function session(input: ClaudeHookInput): AdapterSession {
  return { host: "claude-code", sessionId: requiredText(input, "session_id"), cwd: claudeWorkspace(input.cwd) };
}

/**
 * The turn end `Stop` or `StopFailure` reports. A Stop after a Stop hook kept the turn going (`stop_hook_active`) ends
 * the same prompt again, so it is a turn end of its own, as is each continuation after it (`stopContinuation`).
 */
function turnEndEvent(input: ClaudeHookInput, hook: "Stop" | "StopFailure", descriptor: AdapterSession): AdapterEvent {
  const continuation = stopContinuation(input.stop_hook_active, input.last_assistant_message);
  return {
    kind: "turn-end",
    eventId: `${descriptor.sessionId}:${hook === "Stop" ? "turn" : "turn-failure"}:${requiredText(input, "prompt_id")}${continuation}`,
    sessionId: descriptor.sessionId,
    session: descriptor,
    text: optionalText(input, "last_assistant_message") ?? "",
  };
}

/**
 * `MessageDisplay`'s `delta`, the lines completed since the batch before: any text, blank included. A batch can be one
 * blank line, and the last batch of a reply that ends on a newline is empty, since `final` marks the end.
 */
function batchDelta(input: ClaudeHookInput): string {
  const { delta } = input;
  if (delta === undefined) throw new Error("Claude hook omitted delta");
  if (typeof delta !== "string") throw new Error(`Claude hook delta must be text, not ${delta === null ? "null" : typeof delta}`);
  return delta;
}

export function claudeHookEvents(input: ClaudeHookInput): readonly AdapterEvent[] {
  const hook = requiredText(input, "hook_event_name");
  const descriptor = session(input);
  const sessionId = descriptor.sessionId;
  switch (hook) {
    case "SessionStart":
      return [{ kind: "session-start", eventId: `${sessionId}:SessionStart`, session: descriptor }];
    case "SessionEnd":
      return [{ kind: "session-end", eventId: `${sessionId}:SessionEnd`, sessionId, session: descriptor }];
    case "UserPromptSubmit":
      return [{
        kind: "user-prompt",
        eventId: `${sessionId}:prompt:${requiredText(input, "prompt_id")}`,
        sessionId,
        session: descriptor,
        text: requiredText(input, "prompt"),
      }];
    case "MessageDisplay":
      return [{
        kind: "assistant-delta",
        eventId: `${sessionId}:message:${requiredText(input, "message_id")}:${requiredKey(input, "index")}`,
        sessionId,
        session: descriptor,
        messageId: requiredText(input, "message_id"),
        delta: batchDelta(input),
        final: input.final === true || input.final === "true",
      }];
    case "PreToolUse": {
      const toolUseId = requiredText(input, "tool_use_id");
      // A subagent's hooks come under its parent's session id, with its own agent_id.
      const agentId = optionalText(input, "agent_id");
      return [{
        kind: "call",
        eventId: `${sessionId}:tool-call:${toolUseId}`,
        toolCallId: toolUseId,
        sessionId,
        session: descriptor,
        toolName: requiredText(input, "tool_name"),
        input: input.tool_input,
        ...(agentId === undefined ? {} : { agentId }),
      }];
    }
    case "PostToolUse":
    case "PostToolUseFailure": {
      const toolUseId = requiredText(input, "tool_use_id");
      const failed = hook === "PostToolUseFailure";
      return [{
        kind: "result",
        eventId: `${sessionId}:tool-result:${toolUseId}`,
        toolCallId: toolUseId,
        sessionId,
        session: descriptor,
        toolName: requiredText(input, "tool_name"),
        output: failed ? input.error : input.tool_response,
        failed,
      }];
    }
    case "Stop":
    case "StopFailure":
      return [turnEndEvent(input, hook, descriptor)];
    default:
      throw new Error(`Unsupported Claude hook event: ${hook}`);
  }
}

export function createClaudeEventHandler(eventLoop: RerailEventLoop) {
  return async (input: ClaudeHookInput): Promise<readonly AdapterDelivery[]> => {
    const deliveries: AdapterDelivery[] = [];
    for (const event of claudeHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
    return deliveries;
  };
}
