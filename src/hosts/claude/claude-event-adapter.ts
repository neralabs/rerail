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

function promptEvent(input: ClaudeHookInput, descriptor: AdapterSession): AdapterEvent {
  const sessionId = descriptor.sessionId;
  return {
    kind: "user-prompt",
    eventId: `${sessionId}:prompt:${requiredText(input, "prompt_id")}`,
    sessionId,
    session: descriptor,
    text: requiredText(input, "prompt"),
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

/** One batch of a reply as it streams (`MessageDisplay`). */
function batchEvent(input: ClaudeHookInput, descriptor: AdapterSession): AdapterEvent {
  const sessionId = descriptor.sessionId;
  const messageId = requiredText(input, "message_id");
  return {
    kind: "assistant-delta",
    eventId: `${sessionId}:message:${messageId}:${requiredKey(input, "index")}`,
    sessionId,
    session: descriptor,
    messageId,
    delta: batchDelta(input),
    final: input.final === true || input.final === "true",
  };
}

function toolCallEvent(input: ClaudeHookInput, descriptor: AdapterSession): AdapterEvent {
  const sessionId = descriptor.sessionId;
  const toolUseId = requiredText(input, "tool_use_id");
  // A subagent's hooks come under its parent's session id, with its own agent_id.
  const agentId = optionalText(input, "agent_id");
  return {
    kind: "call",
    eventId: `${sessionId}:tool-call:${toolUseId}`,
    toolCallId: toolUseId,
    sessionId,
    session: descriptor,
    toolName: requiredText(input, "tool_name"),
    input: input.tool_input,
    ...(agentId === undefined ? {} : { agentId }),
  };
}

/** The result `PostToolUse` reports, or the error `PostToolUseFailure` reports in its place (`failed`). */
function toolResultEvent(input: ClaudeHookInput, descriptor: AdapterSession, failed: boolean): AdapterEvent {
  const sessionId = descriptor.sessionId;
  const toolUseId = requiredText(input, "tool_use_id");
  return {
    kind: "result",
    eventId: `${sessionId}:tool-result:${toolUseId}`,
    toolCallId: toolUseId,
    sessionId,
    session: descriptor,
    toolName: requiredText(input, "tool_name"),
    output: failed ? input.error : input.tool_response,
    failed,
  };
}

type HookEvent = (input: ClaudeHookInput, descriptor: AdapterSession) => AdapterEvent;

/** The event each Claude Code hook reports, by the hook's name; `claudeHookEvents` refuses a hook not listed here. */
const hookEvent = new Map<string, HookEvent>([
  ["SessionStart", (_input, descriptor) => ({ kind: "session-start", eventId: `${descriptor.sessionId}:SessionStart`, session: descriptor })],
  ["SessionEnd", (_input, descriptor) => ({
    kind: "session-end",
    eventId: `${descriptor.sessionId}:SessionEnd`,
    sessionId: descriptor.sessionId,
    session: descriptor,
  })],
  ["UserPromptSubmit", promptEvent],
  ["MessageDisplay", batchEvent],
  ["PreToolUse", toolCallEvent],
  ["PostToolUse", (input, descriptor) => toolResultEvent(input, descriptor, false)],
  ["PostToolUseFailure", (input, descriptor) => toolResultEvent(input, descriptor, true)],
  ["Stop", (input, descriptor) => turnEndEvent(input, "Stop", descriptor)],
  ["StopFailure", (input, descriptor) => turnEndEvent(input, "StopFailure", descriptor)],
]);

/**
 * The events a Claude Code hook reports: always one, in the list every coding agent's adapter returns, since a Codex
 * `Stop` reports two.
 */
export function claudeHookEvents(input: ClaudeHookInput): readonly AdapterEvent[] {
  const hook = requiredText(input, "hook_event_name");
  const descriptor = session(input);
  const event = hookEvent.get(hook);
  if (event === undefined) throw new Error(`Unsupported Claude hook event: ${hook}`);
  return [event(input, descriptor)];
}

export function createClaudeEventHandler(eventLoop: RerailEventLoop) {
  return async (input: ClaudeHookInput): Promise<readonly AdapterDelivery[]> => {
    const deliveries: AdapterDelivery[] = [];
    for (const event of claudeHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
    return deliveries;
  };
}
