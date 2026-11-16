import { rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
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
  type SessionLock,
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

/** A drive-letter path in each form Cursor may send it on Windows: `C:\repo`, `C:/repo`, `/c:/repo` (a URI's path), `/c/repo`. */
const WINDOWS_DRIVE_PATH = /^(?:\/([A-Za-z]):?|([A-Za-z]):)(?=[\\/]|$)/u;

/** `value` decoded when it is a `file:` URI, as a path for the platform; a URI Node cannot read stays as sent. */
function fileUrlPath(value: string, windows: boolean): string {
  if (!value.startsWith("file:")) return value;
  try {
    return fileURLToPath(value, { windows });
  } catch {
    return value;
  }
}

/**
 * A path from a Cursor payload as a path on this machine: a `file:` URI decoded and, on Windows, a drive-letter path in
 * any of the forms of `WINDOWS_DRIVE_PATH` written `C:\repo`. Any other path stays as sent.
 */
function cursorPath(value: string, platform: NodeJS.Platform): string {
  const path = fileUrlPath(value, platform === "win32");
  const drive = platform === "win32" ? WINDOWS_DRIVE_PATH.exec(path) : null;
  if (drive === null) return path;
  const rest = path.slice(drive[0].length).replaceAll("/", "\\");
  return `${(drive[1] ?? drive[2] ?? "").toUpperCase()}:${rest === "" ? "\\" : rest}`;
}

export function cursorWorkspace(input: CursorHookInput, platform: NodeJS.Platform = process.platform): string {
  const cwd = optionalText(input, "cwd");
  if (cwd !== undefined) return cursorPath(cwd, platform);
  const roots = input.workspace_roots;
  if (Array.isArray(roots)) {
    const root = roots.find(candidate => typeof candidate === "string" && candidate.trim() !== "");
    if (typeof root === "string") return cursorPath(root, platform);
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
  switch (hook) {
    case "sessionStart":
      return { kind: "session-start", eventId: eventId(input, "session-start"), session: descriptor };
    case "sessionEnd":
      return {
        kind: "session-end",
        eventId: eventId(input, "session-end"),
        sessionId: descriptor.sessionId,
        session: descriptor,
      };
    case "beforeSubmitPrompt":
      return {
        kind: "user-prompt",
        eventId: eventId(input, "prompt"),
        sessionId: descriptor.sessionId,
        session: descriptor,
        text: requiredText(input, "prompt"),
      };
    case "afterAgentResponse":
      return {
        kind: "assistant-message",
        eventId: eventId(input, "assistant"),
        sessionId: descriptor.sessionId,
        session: descriptor,
        messageId: optionalText(input, "generation_id") ?? eventId(input, "assistant-message"),
        text: requiredText(input, "text"),
      };
    case "preToolUse": {
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
    case "postToolUse":
    case "postToolUseFailure": {
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
    case "stop":
      return {
        kind: "turn-end",
        eventId: eventId(input, "turn"),
        sessionId: descriptor.sessionId,
        session: descriptor,
        text: "",
      };
    default:
      throw new Error(`Unsupported Cursor hook event: ${hook}`);
  }
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
  const content = deliveryText(deliveries);
  switch (hook) {
    case "beforeSubmitPrompt":
      return { continue: true };
    case "preToolUse":
      return { permission: "allow" };
    case "sessionStart":
    case "postToolUse": {
      const context = [note, content === undefined ? undefined : relayedSteer(content, deliveryRules(deliveries))].filter(text => text !== undefined).join("\n\n");
      if (context === "") return {};
      return content === undefined ? { additional_context: context } : prepareGuidanceOutput({ additional_context: context }, deliveryRuleIds(deliveries));
    }
    case "stop":
      // Cursor submits a stop hook's follow-up as the person's next message, so the agent acts on the steers before it yields.
      return content === undefined ? {} : prepareGuidanceOutput({ followup_message: relayedSteer(content, deliveryRules(deliveries)) }, deliveryRuleIds(deliveries));
    default:
      return {};
  }
}

/**
 * `lock` is the session's lock the hook holds, which a test that runs no hook leaves out. The transcript's sidecar is
 * read before its lines are checked and written after, so the hook keeps the lock through those checks: another hook of
 * the session writing the sidecar meanwhile would have its write undone.
 */
export async function handleCursorEvent(
  input: CursorHookInput,
  eventLoop: RerailEventLoop,
  storageRoot: string,
  lock?: SessionLock,
): Promise<CursorHookOutput> {
  const hook = requiredText(input, "hook_event_name");
  const event = cursorHookEvent(input);
  const descriptor = event.session ?? cursorSession(input);
  const path = pendingPath(storageRoot, "cursor", descriptor.sessionId);
  const transcript = () => ingestCursorTranscript(input, descriptor, eventLoop, storageRoot);
  const transcriptOwnsText = hook !== "sessionStart" && await keptBy(lock, transcript);
  const deliveries = transcriptOwnsText && (hook === "afterAgentResponse" || hook === "stop")
    ? []
    : await eventLoop.ingest(event);
  switch (hook) {
    case "beforeSubmitPrompt":
      await persistPending(path, deliveries.filter(delivery => steers(delivery.requested)));
      return cursorHookOutput(hook, []);
    case "sessionEnd":
      await rm(path, { force: true });
      return {};
    default:
      return cursorAnswer(input, { hook, descriptor, path }, deliveries);
  }
}

/** Runs `work` while `lock` is kept, or as it is when there is no lock. */
async function keptBy<T>(lock: SessionLock | undefined, work: () => Promise<T>): Promise<T> {
  return await (lock === undefined ? work() : lock.kept(work));
}

/**
 * What a hook answers Cursor with: the steers waiting for an event that can carry them, each linked to this checkout's
 * dashboard, and at a session's start the note only the model reads.
 */
async function cursorAnswer(
  input: CursorHookInput, { hook, descriptor, path }: { hook: string; descriptor: AdapterSession; path: string },
  deliveries: readonly AdapterDelivery[],
): Promise<CursorHookOutput> {
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
