import {
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { serveHookEntry } from "../../harness/hook-adapter.js";
import { runHostHook, type HookOptions, type HookPayload } from "../../harness/hook-host.js";

export { CURSOR_FOLLOWUP_LOOP_LIMIT, cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

/** A Cursor hook's payload: its workspace, then its session and the event's name. */
function cursorPayload(input: CursorHookInput): HookPayload {
  return {
    workspace: () => cursorWorkspace(input),
    event: () => ({
      sessionId: cursorSession(input).sessionId,
      name: String(input.hook_event_name),
      startsSession: input.hook_event_name === "sessionStart",
    }),
  };
}

export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  return runHostHook("cursor", cursorPayload(input), options, (runtime, storageRoot, lock) =>
    handleCursorEvent(input, runtime, storageRoot, lock));
}

// `serveHook` answers `{}` and exits 0 whatever fails, as Rerail does where it is not started, so Cursor carries on: a
// failed hook would put an error on every tool call (design N11). The reason goes to stderr, out of the chat.
await serveHookEntry(import.meta.url, input => runCursorHook(input as CursorHookInput));
