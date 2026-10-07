import { hookText, type AdapterDelivery, type AdapterEvent, type AdapterSession, type RerailEventLoop } from "./adapter-contract.js";

export type CodexHookInput = Readonly<Record<string, unknown>>;

const { required: requiredText } = hookText("Codex");

function session(input: CodexHookInput): AdapterSession {
  return {
    host: "codex",
    sessionId: requiredText(input, "session_id"),
    cwd: requiredText(input, "cwd"),
  };
}

export function codexHookEvents(input: CodexHookInput): readonly AdapterEvent[] {
  const hook = requiredText(input, "hook_event_name");
  const descriptor = session(input);
  const sessionId = descriptor.sessionId;
  if (hook === "SessionStart") {
    return [{
      kind: "session-start",
      eventId: `${sessionId}:session-start:${requiredText(input, "source")}`,
      session: descriptor,
    }];
  }
  if (hook === "UserPromptSubmit") {
    return [{
      kind: "user-prompt",
      eventId: `${sessionId}:prompt:${requiredText(input, "turn_id")}`,
      sessionId,
      session: descriptor,
      text: requiredText(input, "prompt"),
    }];
  }
  if (hook === "PreToolUse") {
    const toolUseId = requiredText(input, "tool_use_id");
    return [{
      kind: "tool-call",
      eventId: `${sessionId}:tool-call:${toolUseId}`,
      toolCallId: toolUseId,
      sessionId,
      session: descriptor,
      toolName: requiredText(input, "tool_name"),
      input: input.tool_input,
    }];
  }
  if (hook === "PostToolUse") {
    const toolUseId = requiredText(input, "tool_use_id");
    return [{
      kind: "tool-result",
      eventId: `${sessionId}:tool-result:${toolUseId}`,
      toolCallId: toolUseId,
      sessionId,
      session: descriptor,
      toolName: requiredText(input, "tool_name"),
      output: input.tool_response,
    }];
  }
  if (hook === "Stop") {
    const turnId = requiredText(input, "turn_id");
    const continuation = input.stop_hook_active === true ? ":continued" : "";
    const text = input.last_assistant_message;
    if (text !== null && typeof text !== "string") throw new TypeError("Codex Stop requires a string or null last_assistant_message");
    const events: AdapterEvent[] = [];
    if (text !== null) events.push({
      kind: "assistant-message",
      eventId: `${sessionId}:assistant:${turnId}${continuation}`,
      sessionId,
      session: descriptor,
      messageId: `${turnId}${continuation}`,
      text,
    });
    events.push({
      kind: "turn-end",
      eventId: `${sessionId}:turn:${turnId}${continuation}`,
      sessionId,
      session: descriptor,
      text: text ?? "",
    });
    return events;
  }
  throw new Error(`Unsupported Codex hook event: ${hook}`);
}

export function createCodexEventHandler(eventLoop: RerailEventLoop) {
  return async (input: CodexHookInput): Promise<readonly AdapterDelivery[]> => {
    const deliveries: AdapterDelivery[] = [];
    for (const event of codexHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
    return deliveries;
  };
}
