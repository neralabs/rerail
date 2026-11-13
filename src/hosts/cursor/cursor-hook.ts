import {
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { serveHookEntry } from "../../harness/hook-adapter.js";
import { hookRerailStarted, runHostHook, withDaemonDeadline, type HookOptions } from "../../harness/hook-host.js";

export { CURSOR_FOLLOWUP_LOOP_LIMIT, cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  const waiting = withDaemonDeadline(options);
  const workspace = cursorWorkspace(input);
  if (!await hookRerailStarted("cursor", workspace, options.databasePath)) return {};
  return runHostHook("cursor", workspace, cursorSession(input).sessionId, input.hook_event_name === "sessionStart", waiting, (runtime, storageRoot, lock) =>
    handleCursorEvent(input, runtime, storageRoot, lock), String(input.hook_event_name));
}

// `serveHook` answers `{}` and exits 0 whatever fails, as Rerail does where it is not started, so Cursor carries on: a
// failed hook would put an error on every tool call (design N11). The reason goes to stderr, out of the chat.
await serveHookEntry(import.meta.url, input => runCursorHook(input as CursorHookInput));
