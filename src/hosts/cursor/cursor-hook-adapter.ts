import { rm } from "node:fs/promises";
import { hookText, type AdapterDelivery, type AdapterEvent, type AdapterSession, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { ingestCursorTranscript } from "./cursor-transcript.js";
import {
  deliveryRuleIds,
  deliveryText,
  pendingPath,
  persistPending,
  prepareGuidanceOutput,
  routeDeliveries,
  type HookInput,
  type HookOutput,
} from "../../harness/hook-adapter.js";
import { sessionStartNote, steerHeadlines } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin, linkDeliveries } from "../../presentation/session-presentation.js";
import { steers } from "../../engine/catalog/rule-mode.js";

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
      kind: "call",
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
      kind: "result",
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

/** Cursor has no message line for the person, so the model is asked to relay each steer's one line itself. */
function relayedSteer(content: string): string {
  const headlines = steerHeadlines(content);
  if (headlines.length === 0) return content;
  return `${content}\n\nCursor shows the person none of this. Start your next reply to them with:\n${headlines.join("\n")}`;
}

/** `note` is the session-start note, which only the model sees. */
export function cursorHookOutput(hook: string, deliveries: readonly AdapterDelivery[], note?: string): CursorHookOutput {
  if (hook === "beforeSubmitPrompt") return { continue: true };
  if (hook === "preToolUse") return { permission: "allow" };
  const content = deliveryText(deliveries);
  const ruleIds = deliveryRuleIds(deliveries);
  if (hook === "sessionStart" || hook === "postToolUse") {
    const context = [note, content === undefined ? undefined : relayedSteer(content)].filter(text => text !== undefined).join("\n\n");
    if (context === "") return {};
    return content === undefined ? { additional_context: context } : prepareGuidanceOutput({ additional_context: context }, ruleIds);
  }

  return {};
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
  const transcriptOwnsText = hook !== "sessionStart" && await ingestCursorTranscript(input, descriptor, eventLoop, storageRoot);
  const deliveries = transcriptOwnsText && (hook === "afterAgentResponse" || hook === "stop")
    ? []
    : await eventLoop.ingest(event);
  if (hook === "beforeSubmitPrompt") {
    await persistPending(path, deliveries.filter(delivery => steers(delivery.requested)));
    return cursorHookOutput(hook, []);
  }
  if (hook === "sessionEnd") {
    await rm(path, { force: true });
    return {};
  }
  const available = await routeDeliveries(path, hook, deliveries, {
    // Cursor only exposes preToolUse.agent_message when a tool is denied.
    // Preserve the steer until a post-tool result can actually carry it.
    sessionStart: true, postToolUse: true,
  });
  // The runtime keeps steer links relative; they point at this checkout's dashboard only here.
  const note = hook === "sessionStart" ? sessionStartNote(await dashboardOrigin(descriptor.cwd), "cursor", descriptor.sessionId) : undefined;
  return cursorHookOutput(hook, await linkDeliveries(available, descriptor.cwd), note);
}
