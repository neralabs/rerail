import { rm } from "node:fs/promises";
import { z } from "zod/v4";
import { hookText, type AdapterDelivery, type AdapterEvent, type AdapterSession, type RerailEventLoop, type SteerRule } from "../../engine/adapter-contract.js";
import { missedSteerDigestEnabled } from "../../engine/experimental-features.js";
import { ingestCursorTranscript } from "./cursor-transcript.js";
import {
  deliveryRuleIds,
  deliveryRules,
  deliveryText,
  pendingPath,
  persistPending,
  prepareGuidanceOutput,
  routeDeliveries,
  type HookInput,
  type HookOutput,
} from "../../harness/hook-adapter.js";
import { headlineText, sessionStartNote, steerHeadlines } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin, linkDeliveries, presentedSessionNoteRules } from "../../presentation/session-presentation.js";
import { steers } from "../../engine/catalog/rule-mode.js";

export type CursorHookInput = HookInput;
export type CursorHookOutput = HookOutput;

const { required: requiredText, optional: optionalText } = hookText("Cursor");

/**
 * The `stop` hook's `loop_count` (how many follow-ups its stop hooks have already submitted in a row) at which Rerail
 * adds no more: a follow-up whose fix brings new findings could otherwise loop until Cursor's own `loop_limit` (5 by
 * default) ends it. Steers not followed up wait in the pending file for the next tool result or session start.
 */
export const CURSOR_FOLLOWUP_LOOP_LIMIT = 3;

/** 0 on a run's first stop; a missing or malformed count reads as that. */
const loopCountSchema = z.number().int().nonnegative().catch(0);

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

/**
 * Cursor shows the person nothing from a hook, so the model is asked to relay each steer's one line itself. Cursor's
 * chat renders markdown, so the `why?` link sits on the word.
 */
function relayedSteer(content: string, rules: readonly SteerRule[]): string {
  const lines = steerHeadlines(rules).map(headline => headlineText(headline, "markdown"));
  if (lines.length === 0) return content;
  return `${content}\n\nCursor shows the user none of this. Begin your next reply with these lines, exactly as written:\n${lines.join("\n")}`;
}

/** `note` is the session-start note, which only the model sees. */
export function cursorHookOutput(hook: string, deliveries: readonly AdapterDelivery[], note?: string): CursorHookOutput {
  if (hook === "beforeSubmitPrompt") return { continue: true };
  if (hook === "preToolUse") return { permission: "allow" };
  const content = deliveryText(deliveries);
  const ruleIds = deliveryRuleIds(deliveries);
  if (hook === "sessionStart" || hook === "postToolUse") {
    const context = [note, content === undefined ? undefined : relayedSteer(content, deliveryRules(deliveries))].filter(text => text !== undefined).join("\n\n");
    if (context === "") return {};
    return content === undefined ? { additional_context: context } : prepareGuidanceOutput({ additional_context: context }, ruleIds);
  }
  // Cursor submits a stop hook's follow-up as the person's next message, so the agent acts on the steers before it yields.
  if (hook === "stop" && content !== undefined) return prepareGuidanceOutput({ followup_message: relayedSteer(content, deliveryRules(deliveries)) }, ruleIds);

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
    // Experimental, with the missed-steer digest: the end of the agent's run carries what is waiting, the turn end's
    // digest included (from the transcript too, which records it among the pending steers), until the follow-ups reach
    // the bound.
    ...(missedSteerDigestEnabled() && loopCountSchema.parse(input.loop_count) < CURSOR_FOLLOWUP_LOOP_LIMIT ? { stop: true } : {}),
  });
  // The runtime records no links; the rules point at this checkout's dashboard only here.
  const note = hook === "sessionStart" ? sessionStartNote(await dashboardOrigin(descriptor.cwd), "cursor", descriptor.sessionId, await presentedSessionNoteRules(descriptor)) : undefined;
  return cursorHookOutput(hook, await linkDeliveries(available, descriptor.cwd), note);
}
