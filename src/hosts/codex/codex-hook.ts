import { hookText } from "../../engine/adapter-contract.js";
import { handleCodexEvent, supportsCodexHook } from "./codex-hook-adapter.js";
import { serveHook, serveHookEntry, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import { DaemonUnreachableError, runHostHook, type HookOptions, type HookPayload } from "../../harness/hook-host.js";

export { codexHookOutput, handleCodexEvent } from "./codex-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Codex");

/**
 * Codex has no footer, so a session that starts while Rerail's daemon cannot run says so once, in the one line Codex
 * shows the person (design N11, D8). Every later hook stays silent.
 */
export const CODEX_NOT_RUNNING = "Rerail ✕ not running · start it with: rerail daemon start";

const startsSession = (input: HookInput): boolean => input.hook_event_name === "SessionStart";

/** A Codex hook's payload: the checkout Codex names, then the event's name and its session. */
function codexPayload(input: HookInput): HookPayload {
  return {
    workspace: () => requiredText(input, "cwd"),
    event: () => ({ name: requiredText(input, "hook_event_name"), sessionId: requiredText(input, "session_id"), startsSession: startsSession(input) }),
  };
}

export async function runCodexHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsCodexHook(input)) return {};
  try {
    return await runHostHook("codex", codexPayload(input), options, (runtime, storageRoot) => handleCodexEvent(input, runtime, storageRoot));
  } catch (error) {
    if (!startsSession(input) || !(error instanceof DaemonUnreachableError)) throw error;
    process.stderr.write(`rerail hook: ${error.message}\n`);
    return { systemMessage: CODEX_NOT_RUNNING };
  }
}

await serveHookEntry(import.meta.url, runCodexHook);
