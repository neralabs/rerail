import { existsSync } from "node:fs";
import { isAbsolute } from "node:path";
import { hookText, type AdapterDelivery, type AdapterEvent, type AdapterSession, type RerailEventLoop } from "./adapter-contract.js";

export type ClaudeHookInput = Readonly<Record<string, unknown>>;

const { required: requiredText, optional: optionalText } = hookText("Claude");

/** Hook `cwd` follows `/cd`; CLAUDE_PROJECT_DIR stays the launch root that identifies the session. */
export function claudeWorkspace(cwd: unknown): string {
  const projectDir = process.env.CLAUDE_PROJECT_DIR;
  if (projectDir !== undefined && isAbsolute(projectDir) && existsSync(projectDir)) return projectDir;
  return typeof cwd === "string" && cwd.trim() !== "" ? cwd : process.cwd();
}

function session(input: ClaudeHookInput): AdapterSession {
  return { host: "claude-code", sessionId: requiredText(input, "session_id"), cwd: claudeWorkspace(input.cwd) };
}

export function claudeHookEvents(input: ClaudeHookInput): readonly AdapterEvent[] {
  const hook = requiredText(input, "hook_event_name");
  const descriptor = session(input);
  const sessionId = descriptor.sessionId;
  if (hook === "SessionStart") {
    return [{ kind: "session-start", eventId: `${sessionId}:SessionStart`, session: descriptor }];
  }
  if (hook === "SessionEnd") {
    return [{ kind: "session-end", eventId: `${sessionId}:SessionEnd`, sessionId, session: descriptor }];
  }
  if (hook === "UserPromptSubmit") {
    return [{
      kind: "user-prompt",
      eventId: `${sessionId}:prompt:${requiredText(input, "prompt_id")}`,
      sessionId,
      session: descriptor,
      text: requiredText(input, "prompt"),
    }];
  }
  if (hook === "MessageDisplay") {
    return [{
      kind: "assistant-delta",
      eventId: `${sessionId}:message:${requiredText(input, "message_id")}:${requiredText(input, "index")}`,
      sessionId,
      session: descriptor,
      messageId: requiredText(input, "message_id"),
      delta: requiredText(input, "delta"),
      final: input.final === true || input.final === "true",
    }];
  }
  if (hook === "PreToolUse") {
    const toolUseId = requiredText(input, "tool_use_id");
    return [{
      kind: "call",
      eventId: `${sessionId}:tool-call:${toolUseId}`,
      toolCallId: toolUseId,
      sessionId,
      session: descriptor,
      toolName: requiredText(input, "tool_name"),
      input: input.tool_input,
    }];
  }
  if (hook === "PostToolUse" || hook === "PostToolUseFailure") {
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
  if (hook === "Stop" || hook === "StopFailure") {
    return [{
      kind: "turn-end",
      eventId: `${sessionId}:${hook === "Stop" ? "turn" : "turn-failure"}:${requiredText(input, "prompt_id")}`,
      sessionId,
      session: descriptor,
      text: optionalText(input, "last_assistant_message") ?? "",
    }];
  }
  throw new Error(`Unsupported Claude hook event: ${hook}`);
}

export function createClaudeEventHandler(eventLoop: RerailEventLoop) {
  return async (input: ClaudeHookInput): Promise<readonly AdapterDelivery[]> => {
    const deliveries: AdapterDelivery[] = [];
    for (const event of claudeHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
    return deliveries;
  };
}
