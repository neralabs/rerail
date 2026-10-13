import { rm } from "node:fs/promises";
import { hookText, type AdapterDelivery, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { steers } from "../../engine/catalog/rule-mode.js";
import { claudeHookEvents, claudeWorkspace } from "./claude-event-adapter.js";
import {
  deliveryRuleIds,
  deliveryText,
  pendingPath,
  persistPending,
  prepareGuidanceOutput,
  routeDeliveries,
  type EventTable,
  type HookInput,
  type HookOutput,
} from "../../harness/hook-adapter.js";
import { headlineText, sessionStartNote, steerHeadlines, type HeadlineLinkStyle } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin, linkDeliveries, presentedSessionNoteRules } from "../../presentation/session-presentation.js";
import { terminalDrawsHyperlinks } from "../../presentation/terminal-hyperlinks.js";

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

/** A steer held for the next message; `shown` once the person has already seen its line, at `Stop`. */
type HeldDelivery = AdapterDelivery & { readonly shown?: true };

/**
 * The person's lines for `deliveries`, one per rule, as one `systemMessage`. Claude Code prefixes it with
 * `<Event>:<Tool> says:`; with more than one line a leading newline puts that prefix on its own line, so the lines align.
 */
function steerLines(deliveries: readonly HeldDelivery[], links: HeadlineLinkStyle): { systemMessage: string } | Record<string, never> {
  const content = deliveryText(deliveries.filter(delivery => delivery.shown !== true));
  const lines = content === undefined ? [] : steerHeadlines(content).map(headline => headlineText(headline, links));
  if (lines.length === 0) return {};
  return { systemMessage: lines.length === 1 ? lines[0]! : `\n${lines.join("\n")}` };
}

export interface ClaudeOutputOptions {
  /** The session-start note, which only the model sees. */
  readonly note?: string | undefined;
  /** How the `why` link is drawn; `hyperlink` unless the terminal cannot draw OSC 8. */
  readonly links?: HeadlineLinkStyle;
}

/**
 * Steers reach the model as `additionalContext` and the person as one line per rule in `systemMessage` (Claude Code
 * shows it under the tool call). At `Stop`, which cannot reach the model without showing it the whole block as an
 * error, only the person's lines are shown; the block waits for the next message (design N4c).
 */
export function claudeHookOutput(hook: string, deliveries: readonly HeldDelivery[], { note, links = "hyperlink" }: ClaudeOutputOptions = {}): HookOutput {
  if (hook === "Stop") return steerLines(deliveries, links);
  const content = deliveryText(deliveries);
  const context = [note, content].filter(text => text !== undefined).join("\n\n");
  if (context === "" || CONTEXT_EVENTS[hook] !== true) return {};
  const output = { ...steerLines(deliveries, links), hookSpecificOutput: { hookEventName: hook, additionalContext: context } };
  return content === undefined ? output : prepareGuidanceOutput(output, deliveryRuleIds(deliveries));
}

export async function handleClaudeEvent(
  input: HookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
  env: NodeJS.ProcessEnv = process.env,
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
  const links = terminalDrawsHyperlinks(env) ? "hyperlink" : "plain";
  if (hook === "Stop") {
    const held = deliveries.filter(delivery => steers(delivery.requested));
    await persistPending(path, held.map(delivery => ({ ...delivery, shown: true as const })));
    return claudeHookOutput(hook, await linkDeliveries(held, cwd), { links });
  }
  const note = hook === "SessionStart" ? sessionStartNote(await dashboardOrigin(cwd), "claude-code", sessionId, await presentedSessionNoteRules({ host: "claude-code", sessionId, cwd })) : undefined;
  const routed: HeldDelivery[] = await routeDeliveries(path, hook, deliveries, CONTEXT_EVENTS);
  // `linkDeliveries` copies each delivery, so a held steer keeps its `shown` mark.
  return claudeHookOutput(hook, await linkDeliveries(routed, cwd), { note, links });
}
