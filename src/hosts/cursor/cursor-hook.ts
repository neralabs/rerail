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
import { isCursorCloud } from "./cursor-cloud.js";

export { CURSOR_FOLLOWUP_LOOP_LIMIT, cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

/**
 * A session that starts while no daemon it may use runs, because `RERAIL_AUTOSTART` kept the hook from starting or
 * replacing one, says so through the model, which Cursor asks to relay the line (`answerAutostartOff`,
 * `cursorRelayedLine`).
 *
 * On a Cursor cloud agent's VM (`isCursorCloud`, the marker `rerail start --cursor-cloud` leaves beside the database),
 * Cursor fires no `sessionStart`, so every hook waits for the daemon as a session's start does
 * (`HookOptions.waitsForDaemon`), within the same deadline, and no event is lost while the daemon boots; and the
 * session's first hook records the start of the agent's own session, whose note its first `postToolUse` carries
 * (`handleCursorEvent`). Without the marker every hook runs as on a person's machine.
 */
export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  const waiting = withDaemonDeadline(options);
  const workspace = cursorWorkspace(input);
  if (!await hookRerailStarted("cursor", workspace, options.databasePath)) return {};
  const startsSession = input.hook_event_name === "sessionStart";
  const cloud = isCursorCloud(options.databasePath);
  return await runHostHook("cursor", workspace, cursorSession(input).sessionId, startsSession, { ...waiting, waitsForDaemon: cloud }, (runtime, storageRoot, lock) =>
    handleCursorEvent(input, runtime, storageRoot, lock, cloud), String(input.hook_event_name))
    .catch((error: unknown) => answerAutostartOff(error, startsSession, cursorRelayedLine));
}

// `serveHook` answers `{}` and exits 0 whatever fails, as Rerail does where it is not started, so Cursor carries on: a
// failed hook would put an error on every tool call (design N11). The reason goes to stderr, out of the chat.
await serveHookEntry(import.meta.url, input => runCursorHook(input as CursorHookInput));
