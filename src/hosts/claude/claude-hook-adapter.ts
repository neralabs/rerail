import { rm } from "node:fs/promises";
import { hookText, type AdapterDelivery, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { claudeHookEvents, claudeWorkspace } from "./claude-event-adapter.js";
import {
  deliveryRuleIds,
  deliveryText,
  pendingPath,
  prepareGuidanceOutput,
  routeDeliveries,
  type EventTable,
  type HookInput,
  type HookOutput,
} from "../../harness/hook-adapter.js";
import { sessionStartNote, steerHeadlines } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin, linkDeliveries, presentedSessionNoteRules } from "../../presentation/session-presentation.js";

const CLAUDE_EVENTS: EventTable = {
  SessionStart: true,
  SessionEnd: true,
  UserPromptSubmit: true,
  MessageDisplay: true,
  PreToolUse: true,
  PostToolUse: true,
  PostToolUseFailure: true,
  Stop: true,
  StopFailure: true,
};

const CONTEXT_EVENTS: EventTable = {
  SessionStart: true,
  UserPromptSubmit: true,
  PreToolUse: true,
  PostToolUse: true,
  PostToolUseFailure: true,
};

const { required: requiredText } = hookText("Claude");

export function supportsClaudeHook(input: HookInput): boolean {
  return CLAUDE_EVENTS[requiredText(input, "hook_event_name")] === true;
}

/**
 * Steers reach the model as `additionalContext` and the person as one `systemMessage` line per rule
 * (Claude Code shows it under the tool call). `note` is the session-start note, which only the model sees.
 */
export function claudeHookOutput(hook: string, deliveries: readonly AdapterDelivery[], note?: string): HookOutput {
  const content = deliveryText(deliveries);
  const context = [note, content].filter(text => text !== undefined).join("\n\n");
  if (context === "" || CONTEXT_EVENTS[hook] !== true) return {};
  const headlines = content === undefined ? [] : steerHeadlines(content);
  const output = {
    ...(headlines.length === 0 ? {} : { systemMessage: headlines.join("\n") }),
    hookSpecificOutput: { hookEventName: hook, additionalContext: context },
  };
  return content === undefined ? output : prepareGuidanceOutput(output, deliveryRuleIds(deliveries));
}

export async function handleClaudeEvent(
  input: HookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
): Promise<HookOutput> {
  const hook = requiredText(input, "hook_event_name");
  const sessionId = requiredText(input, "session_id");
  const deliveries: AdapterDelivery[] = [];
  for (const event of claudeHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
  const path = pendingPath(storageRoot, "claude-code", sessionId);
  if (hook === "SessionEnd") {
    await rm(path, { force: true });
    return {};
  }
  // The runtime keeps steer links relative; they point at this checkout's dashboard only here.
  const cwd = claudeWorkspace(input.cwd);
  const note = hook === "SessionStart" ? sessionStartNote(await dashboardOrigin(cwd), "claude-code", sessionId, await presentedSessionNoteRules({ host: "claude-code", sessionId, cwd })) : undefined;
  return claudeHookOutput(hook, await linkDeliveries(await routeDeliveries(path, hook, deliveries, CONTEXT_EVENTS), cwd), note);
}
