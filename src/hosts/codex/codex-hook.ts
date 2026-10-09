import { hookText } from "../../engine/adapter-contract.js";
import { handleCodexEvent, supportsCodexHook } from "./codex-hook-adapter.js";
import { launchedDirectly, serveHook, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import { hookRerailStarted, runHostHook, type HookOptions } from "../../harness/hook-host.js";

export { codexHookOutput, handleCodexEvent } from "./codex-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Codex");

export async function runCodexHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsCodexHook(input)) return {};
  const workspace = requiredText(input, "cwd");
  if (!await hookRerailStarted("codex", workspace, options.databasePath)) return {};
  return runHostHook("codex", workspace, requiredText(input, "session_id"), options, (runtime, storageRoot) =>
    handleCodexEvent(input, runtime, storageRoot));
}

if (launchedDirectly(import.meta.url)) await serveHook(runCodexHook);
