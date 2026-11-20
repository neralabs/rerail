import { answeredFacts, hookText, type AdapterDelivery, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { codexHookEvents } from "./codex-event-adapter.js";
import {
  deliveryConflicts,
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
import { conflictLine, headlineText, steerHeadlines } from "../../engine/code-rule/rerail-steer.js";
import { linkDeliveries, linkedByAnswer, presentedSessionStartNote } from "../../presentation/session-presentation.js";

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

/**
 * The person's lines, one per rule and one per conflict a note surfaces; Codex draws hook text as plain text with no
 * links, so each prints its link after it.
 */
const steerLines: SteerLines = deliveries => {
  const lines = [
    ...steerHeadlines(deliveryRules(deliveries)).map(headline => headlineText(headline, "plain")),
    ...deliveryConflicts(deliveries).map(conflict => conflictLine(conflict, "plain")),
  ];
  return lines.length === 0 ? {} : { systemMessage: lines.join("\n") };
};

/**
 * Steers reach the model as `additionalContext` and the person as one `systemMessage` line per rule, in the same
 * output (`steerHookOutput`). `Stop` carries neither: Codex records a `Stop` block's reason as the user's turn, so
 * its steers wait, line and all, for the next context event (`holdStopSteers`). `note` is the session-start note,
 * which only the model sees.
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
  // Left for a daemon that is not reading: the session's held steers wait in their file for the next hook it answers.
  if (eventLoop.untaken === true) return {};
  const path = pendingPath(storageRoot, "codex", sessionId);
  // The daemon's answer carries the note's link and rules and the steers' links; an older daemon's is read after it. A
  // steer held for a later event keeps this answer's links, so that event's answer need carry none.
  const answered = answeredFacts(eventLoop);
  const found = linkedByAnswer(deliveries, answered);
  if (hook === "Stop") {
    await holdStopSteers(path, found);
    return {};
  }
  // The runtime records no links; the rules point at this checkout's dashboard only here.
  const cwd = requiredText(input, "cwd");
  const note = hook === "SessionStart" ? await presentedSessionStartNote({ host: "codex", sessionId, cwd }, answered) : undefined;
  const routed = await routeDeliveries(path, hook, found, CONTEXT_EVENTS);
  // `linkDeliveries` copies each delivery, so a held steer keeps its `late` mark.
  return codexHookOutput(hook, await linkDeliveries(routed, cwd, answered), note);
}
