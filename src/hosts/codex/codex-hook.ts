import { hookText } from "../../engine/adapter-contract.js";
import { handleCodexEvent, supportsCodexHook } from "./codex-hook-adapter.js";
import { launchedDirectly, serveHook, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import { DaemonUnreachableError, hookRerailStarted, runHostHook, type HookOptions } from "../../harness/hook-host.js";

export { codexHookOutput, handleCodexEvent } from "./codex-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Codex");

/**
 * Codex has no footer, so a session that starts while Rerail's daemon cannot run says so once, in the one line Codex
 * shows the person (design N11, D8). Every later hook stays silent.
 */
export const CODEX_NOT_RUNNING = "Rerail ✕ not running · start it with: rerail daemon start";

export async function runCodexHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsCodexHook(input)) return {};
  const workspace = requiredText(input, "cwd");
  if (!await hookRerailStarted("codex", workspace, options.databasePath)) return {};
  const startsSession = requiredText(input, "hook_event_name") === "SessionStart";
  try {
    return await runHostHook("codex", workspace, requiredText(input, "session_id"), startsSession, options, (runtime, storageRoot) =>
      handleCodexEvent(input, runtime, storageRoot));
  } catch (error) {
    if (!startsSession || !(error instanceof DaemonUnreachableError)) throw error;
    process.stderr.write(`rerail hook: ${error.message}\n`);
    return { systemMessage: CODEX_NOT_RUNNING };
  }
}

if (launchedDirectly(import.meta.url)) await serveHook(runCodexHook);
