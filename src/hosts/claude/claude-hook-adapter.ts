import { rm } from "node:fs/promises";
import { hookText, type AdapterDelivery, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { claudeHookEvents, claudeWorkspace } from "./claude-event-adapter.js";
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
import { headlineText, sessionStartNote, steerHeadlines, type HeadlineLinkStyle } from "../../engine/code-rule/rerail-steer.js";
import { dashboardOrigin, linkDeliveries, presentedSessionNoteRules } from "../../presentation/session-presentation.js";
import { terminalDrawsHyperlinks } from "../../presentation/terminal-hyperlinks.js";
import { colourDepth, rerailMark } from "../../presentation/rerail-mark.js";

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

/**
 * Events whose `additionalContext` reaches the model beside a tool call or at the session's start. `UserPromptSubmit`'s
 * lands inside the person's own turn, so a steer held then waits for the next tool call.
 */
const CONTEXT_EVENTS: EventTable = {
  SessionStart: true,
  PreToolUse: true,
  PostToolUse: true,
  PostToolUseFailure: true,
};

const { required: requiredText } = hookText("Claude");

export function supportsClaudeHook(input: HookInput): boolean {
  return CLAUDE_EVENTS[requiredText(input, "hook_event_name")] === true;
}

/**
 * The person's lines for `deliveries`, one per rule, as one `systemMessage`. Claude Code prefixes it with
 * `<Event>:<Tool> says:`; with more than one line a leading newline puts that prefix on its own line, so the lines align.
 */
function steerLines(links: HeadlineLinkStyle, mark: string | undefined): SteerLines {
  return deliveries => {
    const lines = steerHeadlines(deliveryRules(deliveries)).map(headline => headlineText(headline, links, mark));
    if (lines.length === 0) return {};
    return { systemMessage: lines.length === 1 ? lines[0]! : `\n${lines.join("\n")}` };
  };
}

export interface ClaudeOutputOptions {
  /** The session-start note, which only the model sees. */
  readonly note?: string | undefined;
  /** How the `why?` link is drawn; `hyperlink` unless the terminal cannot draw OSC 8. */
  readonly links?: HeadlineLinkStyle;
  /** The coloured `R E R A I L` that stands for Rerail in the line (Claude Code draws ANSI colour); the plain word without. */
  readonly mark?: string | undefined;
}

/**
 * Steers reach the model as `additionalContext` and the person as one line per rule in `systemMessage` (Claude Code
 * shows it under the tool call); at `Stop`, only the person's lines (`steerHookOutput`).
 */
export function claudeHookOutput(hook: string, deliveries: readonly HeldDelivery[], { note, links = "hyperlink", mark }: ClaudeOutputOptions = {}): HookOutput {
  return steerHookOutput(hook, deliveries, CONTEXT_EVENTS, steerLines(links, mark), note);
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
  // The runtime records no links; the rules point at this checkout's dashboard only here.
  const cwd = claudeWorkspace(input.cwd);
  const links = terminalDrawsHyperlinks(env) ? "hyperlink" : "plain";
  const mark = rerailMark(colourDepth(env));
  if (hook === "Stop") return claudeHookOutput(hook, await linkDeliveries(await holdStopSteers(path, deliveries), cwd), { links, mark });
  const note = hook === "SessionStart" ? sessionStartNote(await dashboardOrigin(cwd), "claude-code", sessionId, await presentedSessionNoteRules({ host: "claude-code", sessionId, cwd })) : undefined;
  const routed: HeldDelivery[] = await routeDeliveries(path, hook, deliveries, CONTEXT_EVENTS);
  // `linkDeliveries` copies each delivery, so a held steer keeps its `shown` mark.
  return claudeHookOutput(hook, await linkDeliveries(routed, cwd), { note, links, mark });
}
