import { rm } from "node:fs/promises";
import { hookText, type AdapterDelivery, type AdapterEvent, type AdapterSession, type RerailEventLoop } from "./adapter-contract.js";
import { ingestCursorTranscript } from "./cursor-transcript.js";
import {
  deliveryRuleIds,
  deliveryText,
  pendingPath,
  prepareGuidanceOutput,
  readPending,
  replacePending,
  routeDeliveries,
  type HookInput,
  type HookOutput,
} from "./hook-adapter.js";

export type CursorHookInput = HookInput;
export type CursorHookOutput = HookOutput;

const { required: requiredText, optional: optionalText } = hookText("Cursor");

export function cursorWorkspace(input: CursorHookInput): string {
  const cwd = optionalText(input, "cwd");
  if (cwd !== undefined) return cwd;
  const roots = input.workspace_roots;
  if (Array.isArray(roots)) {
    const root = roots.find(candidate => typeof candidate === "string" && candidate.trim() !== "");
    if (typeof root === "string") return root;
  }
  return process.cwd();
}

export function cursorSession(input: CursorHookInput): AdapterSession {
  return {
    host: "cursor",
    sessionId: optionalText(input, "session_id") ?? requiredText(input, "conversation_id"),
    cwd: cursorWorkspace(input),
  };
}

function eventId(input: CursorHookInput, suffix: string): string {
  const descriptor = cursorSession(input);
  const generation = optionalText(input, "generation_id");
  const generationSuffix = generation === undefined ? "" : `:${generation}`;
  return `${descriptor.sessionId}:${suffix}${generationSuffix}`;
}

export function cursorHookEvent(input: CursorHookInput): AdapterEvent {
  const hook = requiredText(input, "hook_event_name");
  const descriptor = cursorSession(input);
  if (hook === "sessionStart") {
    return { kind: "session-start", eventId: eventId(input, "session-start"), session: descriptor };
  }
  if (hook === "sessionEnd") {
    return {
      kind: "session-end",
      eventId: eventId(input, "session-end"),
      sessionId: descriptor.sessionId,
      session: descriptor,
    };
  }
  if (hook === "beforeSubmitPrompt") {
    return {
      kind: "user-prompt",
      eventId: eventId(input, "prompt"),
      sessionId: descriptor.sessionId,
      session: descriptor,
      text: requiredText(input, "prompt"),
    };
  }
  if (hook === "afterAgentResponse") {
    return {
      kind: "assistant-message",
      eventId: eventId(input, "assistant"),
      sessionId: descriptor.sessionId,
      session: descriptor,
      messageId: optionalText(input, "generation_id") ?? eventId(input, "assistant-message"),
      text: requiredText(input, "text"),
    };
  }
  if (hook === "preToolUse") {
    const toolUseId = requiredText(input, "tool_use_id");
    const toolName = requiredText(input, "tool_name");
    return {
      kind: "tool-call",
      eventId: `${descriptor.sessionId}:tool-call:${toolUseId}:${toolName}`,
      toolCallId: toolUseId,
      sessionId: descriptor.sessionId,
      session: descriptor,
      toolName,
      input: input.tool_input,
    };
  }
  if (hook === "postToolUse" || hook === "postToolUseFailure") {
    const toolUseId = requiredText(input, "tool_use_id");
    const toolName = requiredText(input, "tool_name");
    const failed = hook === "postToolUseFailure";
    return {
      kind: "tool-result",
      eventId: `${descriptor.sessionId}:tool-result:${toolUseId}:${toolName}:${failed ? "failure" : "success"}`,
      toolCallId: toolUseId,
      sessionId: descriptor.sessionId,
      session: descriptor,
      toolName,
      output: failed ? input.error_message : input.tool_output,
      failed,
    };
  }
  if (hook === "stop") {
    return {
      kind: "turn-end",
      eventId: eventId(input, "turn"),
      sessionId: descriptor.sessionId,
      session: descriptor,
      text: "",
    };
  }
  throw new Error(`Unsupported Cursor hook event: ${hook}`);
}

export function cursorHookOutput(hook: string, deliveries: readonly AdapterDelivery[]): CursorHookOutput {
  if (hook === "beforeSubmitPrompt") return { continue: true };
  if (hook === "preToolUse") return { permission: "allow" };
  const content = deliveryText(deliveries);
  if (content === undefined) return {};
  const ruleIds = deliveryRuleIds(deliveries);
  if (hook === "sessionStart" || hook === "postToolUse") {
    return prepareGuidanceOutput({ additional_context: content }, ruleIds);
  }
  if (hook === "stop") return prepareGuidanceOutput({ followup_message: content }, ruleIds);
  return {};
}

async function promoteNextTurn(path: string): Promise<void> {
  const pending = (await readPending(path)).map(delivery => delivery.requested === "nextTurn"
    ? { ...delivery, requested: "steer" as const, mechanism: "additional-context" as const }
    : delivery);
  await replacePending(path, pending);
}

export async function handleCursorEvent(
  input: CursorHookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
): Promise<CursorHookOutput> {
  const hook = requiredText(input, "hook_event_name");
  const event = cursorHookEvent(input);
  const descriptor = event.session ?? cursorSession(input);
  const path = pendingPath(storageRoot, "cursor", descriptor.sessionId);
  const transcriptOwnsText = hook !== "sessionStart" && await ingestCursorTranscript(
    input, descriptor, eventLoop, storageRoot, () => promoteNextTurn(path),
  );
  const deliveries = transcriptOwnsText && (hook === "afterAgentResponse" || hook === "stop")
    ? []
    : await eventLoop.ingest(event);
  if (hook === "beforeSubmitPrompt") {
    await promoteNextTurn(path);
    await replacePending(path, [...await readPending(path), ...deliveries.filter(delivery => delivery.requested !== "observe")]);
    return cursorHookOutput(hook, []);
  }
  if (hook === "sessionEnd") {
    await rm(path, { force: true });
    return {};
  }
  const available = await routeDeliveries(path, hook === "stop" ? "Stop" : hook, deliveries, {
    // Cursor only exposes preToolUse.agent_message when a tool is denied.
    // Preserve steering until a post-tool result can actually carry it.
    sessionStart: true, postToolUse: true,
  });
  return cursorHookOutput(hook, available);
}
