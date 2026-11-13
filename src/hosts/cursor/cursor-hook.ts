import {
  cursorRelayedLine,
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { serveHookEntry } from "../../harness/hook-adapter.js";
import { answerAutostartOff, hookRerailStarted, runHostHook, withDaemonDeadline, type HookOptions } from "../../harness/hook-host.js";

export { CURSOR_FOLLOWUP_LOOP_LIMIT, cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

/**
 * A session that starts with no daemon to record it, because `RERAIL_AUTOSTART` kept one from starting, says so through
 * the model, which Cursor asks to relay the line (`answerAutostartOff`, `cursorRelayedLine`).
 */
export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  const waiting = withDaemonDeadline(options);
  const workspace = cursorWorkspace(input);
  if (!await hookRerailStarted("cursor", workspace, options.databasePath)) return {};
  return await runHostHook("cursor", workspace, cursorSession(input).sessionId, input.hook_event_name === "sessionStart", waiting, (runtime, storageRoot, lock) =>
    handleCursorEvent(input, runtime, storageRoot, lock), String(input.hook_event_name))
    .catch((error: unknown) => answerAutostartOff(error, cursorRelayedLine));
}

// `serveHook` answers `{}` and exits 0 whatever fails, as Rerail does where it is not started, so Cursor carries on: a
// failed hook would put an error on every tool call (design N11). The reason goes to stderr, out of the chat.
await serveHookEntry(import.meta.url, input => runCursorHook(input as CursorHookInput));
