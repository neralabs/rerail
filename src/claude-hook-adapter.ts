import { rm } from "node:fs/promises";
import { hookText, type AdapterDelivery, type RerailEventLoop } from "./adapter-contract.js";
import { claudeHookEvents } from "./claude-event-adapter.js";
import {
  deliveryRuleIds,
  deliveryText,
  pendingPath,
  prepareGuidanceOutput,
  routeDeliveries,
  type EventTable,
  type HookInput,
  type HookOutput,
} from "./hook-adapter.js";

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

export function claudeHookOutput(hook: string, deliveries: readonly AdapterDelivery[]): HookOutput {
  const content = deliveryText(deliveries);
  if (content === undefined || CONTEXT_EVENTS[hook] !== true) return {};
  return prepareGuidanceOutput(
    { hookSpecificOutput: { hookEventName: hook, additionalContext: content } },
    deliveryRuleIds(deliveries),
  );
}

export async function handleClaudeEvent(
  input: HookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
): Promise<HookOutput> {
  const hook = requiredText(input, "hook_event_name");
  const deliveries: AdapterDelivery[] = [];
  for (const event of claudeHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
  const path = pendingPath(storageRoot, "claude-code", requiredText(input, "session_id"));
  if (hook === "SessionEnd") {
    await rm(path, { force: true });
    return {};
  }
  return claudeHookOutput(hook, await routeDeliveries(path, hook, deliveries, CONTEXT_EVENTS));
}
