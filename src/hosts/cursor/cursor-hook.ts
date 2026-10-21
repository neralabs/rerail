import {
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { launchedDirectly, serveHook } from "../../harness/hook-adapter.js";
import { hookRerailStarted, runHostHook, type HookOptions } from "../../harness/hook-host.js";

export { CURSOR_FOLLOWUP_LOOP_LIMIT, cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  const workspace = cursorWorkspace(input);
  if (!await hookRerailStarted("cursor", workspace, options.databasePath)) return {};
  return runHostHook("cursor", workspace, cursorSession(input).sessionId, input.hook_event_name === "sessionStart", options, (runtime, storageRoot) =>
    handleCursorEvent(input, runtime, storageRoot));
}

// `serveHook` answers `{}` and exits 0 whatever fails, as Rerail does where it is not started, so Cursor carries on: a
// failed hook would put an error on every tool call (design N11). The reason goes to stderr, out of the chat.
if (launchedDirectly(import.meta.url)) await serveHook(input => runCursorHook(input as CursorHookInput));
