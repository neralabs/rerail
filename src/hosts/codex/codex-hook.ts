import { hookText } from "../../engine/adapter-contract.js";
import { handleCodexEvent, supportsCodexHook } from "./codex-hook-adapter.js";
import { serveHook, serveHookEntry, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import {
  answerAutostartOff,
  DaemonUnreachableError,
  hookRerailStarted,
  runHostHook,
  withDaemonDeadline,
  type HookOptions,
} from "../../harness/hook-host.js";

export { codexHookOutput, handleCodexEvent } from "./codex-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Codex");

/**
 * Codex has no footer, so a session that starts while Rerail's daemon cannot run says so once, in the one line Codex
 * shows the person (design N11, D8). Every later hook stays silent.
 */
export const CODEX_NOT_RUNNING = "Rerail ✕ not running · start it with: rerail daemon start";

/**
 * A session that starts with no daemon says so in that line: as in Claude Code, why and that nothing is recorded where
 * `RERAIL_AUTOSTART` kept one from starting (`answerAutostartOff`), else `CODEX_NOT_RUNNING`.
 */
export async function runCodexHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsCodexHook(input)) return {};
  const waiting = withDaemonDeadline(options);
  const workspace = requiredText(input, "cwd");
  if (!await hookRerailStarted("codex", workspace, options.databasePath)) return {};
  const event = requiredText(input, "hook_event_name");
  const startsSession = event === "SessionStart";
  try {
    return await runHostHook("codex", workspace, requiredText(input, "session_id"), startsSession, waiting, (runtime, storageRoot) =>
      handleCodexEvent(input, runtime, storageRoot), event)
      .catch((error: unknown) => answerAutostartOff(error, line => ({ systemMessage: line })));
  } catch (error) {
    if (!startsSession || !(error instanceof DaemonUnreachableError)) throw error;
    process.stderr.write(`rerail hook: ${error.message}\n`);
    return { systemMessage: CODEX_NOT_RUNNING };
  }
}

await serveHookEntry(import.meta.url, runCodexHook);
