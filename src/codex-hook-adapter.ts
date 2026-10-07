import { hookText, type AdapterDelivery, type RerailEventLoop } from "./adapter-contract.js";
import { codexHookEvents } from "./codex-event-adapter.js";
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

const CODEX_EVENTS: EventTable = {
  SessionStart: true,
  UserPromptSubmit: true,
  PreToolUse: true,
  PostToolUse: true,
  Stop: true,
};

const CONTEXT_EVENTS: EventTable = {
  SessionStart: true,
  UserPromptSubmit: true,
  PreToolUse: true,
};

const { required: requiredText } = hookText("Codex");

export function supportsCodexHook(input: HookInput): boolean {
  return CODEX_EVENTS[requiredText(input, "hook_event_name")] === true;
}

export function codexHookOutput(hook: string, deliveries: readonly AdapterDelivery[]): HookOutput {
  const content = deliveryText(deliveries);
  if (content === undefined) return {};
  const ruleIds = deliveryRuleIds(deliveries);
  if (hook === "Stop") return prepareGuidanceOutput({ decision: "block", reason: content }, ruleIds);
  if (CONTEXT_EVENTS[hook] === true) {
    return prepareGuidanceOutput({ hookSpecificOutput: { hookEventName: hook, additionalContext: content } }, ruleIds);
  }
  return {};
}

export async function handleCodexEvent(
  input: HookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
): Promise<HookOutput> {
  const hook = requiredText(input, "hook_event_name");
  const deliveries: AdapterDelivery[] = [];
  for (const event of codexHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
  const routed = hook === "PostToolUse" || hook === "Stop"
    ? deliveries.map((delivery): AdapterDelivery =>
        delivery.requested === "steer" ? { ...delivery, requested: "followUp" } : delivery)
    : deliveries;
  const path = pendingPath(storageRoot, "codex", requiredText(input, "session_id"));
  return codexHookOutput(hook, await routeDeliveries(path, hook, routed, CONTEXT_EVENTS));
}
