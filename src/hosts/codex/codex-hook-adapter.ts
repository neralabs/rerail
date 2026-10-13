import { hookText, type AdapterDelivery, type RerailEventLoop } from "../../engine/adapter-contract.js";
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
} from "../../harness/hook-adapter.js";
import { headlineText, sessionStartNote, steerHeadlines } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin, linkDeliveries, presentedSessionNoteRules } from "../../presentation/session-presentation.js";

const CODEX_EVENTS: EventTable = {
  SessionStart: true,
  UserPromptSubmit: true,
  PreToolUse: true,
  PostToolUse: true,
  Stop: true,
};

/** Events whose `additionalContext` reaches the model; Codex adds it as a developer message. */
const CONTEXT_EVENTS: EventTable = {
  SessionStart: true,
  UserPromptSubmit: true,
  PreToolUse: true,
  PostToolUse: true,
};

/** Stop carries steers too: Codex continues the turn with it as a `decision: "block"` reason. */
const CARRY_EVENTS: EventTable = { ...CONTEXT_EVENTS, Stop: true };

const { required: requiredText } = hookText("Codex");

export function supportsCodexHook(input: HookInput): boolean {
  return CODEX_EVENTS[requiredText(input, "hook_event_name")] === true;
}

/**
 * Steers reach the model as context (or, at `Stop`, as the reason to keep going) and the person as one
 * `systemMessage` line per rule. `note` is the session-start note, which only the model sees.
 */
export function codexHookOutput(hook: string, deliveries: readonly AdapterDelivery[], note?: string): HookOutput {
  const content = deliveryText(deliveries);
  const ruleIds = deliveryRuleIds(deliveries);
  // Codex prints the reason of a Stop hold under its own "Blocked by hook", so the person's lines would only repeat it.
  if (hook === "Stop") {
    return content === undefined ? {} : prepareGuidanceOutput({ decision: "block", reason: content }, ruleIds);
  }
  const context = [note, content].filter(text => text !== undefined).join("\n\n");
  if (context === "" || CONTEXT_EVENTS[hook] !== true) return {};
  // Codex draws hook text as plain text with no links, so each line prints its `why` link after it.
  const lines = content === undefined ? [] : steerHeadlines(content).map(headline => headlineText(headline, "plain"));
  const output = {
    ...(lines.length === 0 ? {} : { systemMessage: lines.join("\n") }),
    hookSpecificOutput: { hookEventName: hook, additionalContext: context },
  };
  return content === undefined ? output : prepareGuidanceOutput(output, ruleIds);
}

export async function handleCodexEvent(
  input: HookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
): Promise<HookOutput> {
  const hook = requiredText(input, "hook_event_name");
  const sessionId = requiredText(input, "session_id");
  const deliveries: AdapterDelivery[] = [];
  for (const event of codexHookEvents(input)) deliveries.push(...await eventLoop.ingest(event));
  const path = pendingPath(storageRoot, "codex", sessionId);
  // The runtime keeps steer links relative; they point at this checkout's dashboard only here.
  const cwd = requiredText(input, "cwd");
  const note = hook === "SessionStart" ? sessionStartNote(await dashboardOrigin(cwd), "codex", sessionId, await presentedSessionNoteRules({ host: "codex", sessionId, cwd })) : undefined;
  return codexHookOutput(hook, await linkDeliveries(await routeDeliveries(path, hook, deliveries, CARRY_EVENTS), cwd), note);
}
