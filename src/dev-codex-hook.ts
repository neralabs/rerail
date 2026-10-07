import { hookText } from "./adapter-contract.js";
import { handleCodexEvent, supportsCodexHook } from "./codex-hook-adapter.js";
import { isRerailDisabled } from "./checkout-control.js";
import { runDevHostHook } from "./dev-hook-runtime.js";
import type { HookInput, HookOutput } from "./hook-adapter.js";

export { serveHook } from "./hook-adapter.js";

const { required: requiredText } = hookText("Codex");

export async function runCodexHook(input: HookInput): Promise<HookOutput> {
  if (!supportsCodexHook(input)) return {};
  const workspace = requiredText(input, "cwd");
  if (isRerailDisabled(workspace)) return {};
  return runDevHostHook("codex", workspace, requiredText(input, "session_id"), (runtime, storageRoot) =>
    handleCodexEvent(input, runtime, storageRoot));
}
