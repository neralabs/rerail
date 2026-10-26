import { hookText, type AdapterDelivery, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { codexHookEvents } from "./codex-event-adapter.js";
import {
  deliveryRules,
  holdStopSteers,
  pendingPath,
  routeDeliveries,
  steerHookOutput,
  type EventTable,
  type HeldDelivery,
  type HookInput,
  type HookOutput,
  type SteerLines,
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

const { required: requiredText } = hookText("Codex");

export function supportsCodexHook(input: HookInput): boolean {
  return CODEX_EVENTS[requiredText(input, "hook_event_name")] === true;
}

/** The person's lines, one per rule; Codex draws hook text as plain text with no links, so each prints its `why?` link after it. */
const steerLines: SteerLines = deliveries => {
  const lines = steerHeadlines(deliveryRules(deliveries)).map(headline => headlineText(headline, "plain"));
  return lines.length === 0 ? {} : { systemMessage: lines.join("\n") };
};

/**
 * Steers reach the model as `additionalContext` and the person as one `systemMessage` line per rule; at `Stop`, only
 * the person's lines (`steerHookOutput`), since Codex records a `Stop` block's reason as the user's turn. `note` is the
 * session-start note, which only the model sees.
 */
export function codexHookOutput(hook: string, deliveries: readonly HeldDelivery[], note?: string): HookOutput {
  return steerHookOutput(hook, deliveries, CONTEXT_EVENTS, steerLines, note);
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
  // The runtime records no links; the rules point at this checkout's dashboard only here.
  const cwd = requiredText(input, "cwd");
  if (hook === "Stop") return codexHookOutput(hook, await linkDeliveries(await holdStopSteers(path, deliveries), cwd));
  const note = hook === "SessionStart" ? sessionStartNote(await dashboardOrigin(cwd), "codex", sessionId, await presentedSessionNoteRules({ host: "codex", sessionId, cwd })) : undefined;
  const routed: HeldDelivery[] = await routeDeliveries(path, hook, deliveries, CONTEXT_EVENTS);
  // `linkDeliveries` copies each delivery, so a held steer keeps its `shown` mark.
  return codexHookOutput(hook, await linkDeliveries(routed, cwd), note);
}
