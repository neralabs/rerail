import { hookText } from "./adapter-contract.js";
import { claudeWorkspace } from "./claude-event-adapter.js";
import { handleClaudeEvent, supportsClaudeHook } from "./claude-hook-adapter.js";
import { runDevHostHook } from "./dev-hook-runtime.js";
import type { HookInput, HookOutput } from "./hook-adapter.js";

export { serveHook } from "./hook-adapter.js";

const { required: requiredText } = hookText("Claude");

export async function runClaudeHook(input: HookInput): Promise<HookOutput> {
  if (!supportsClaudeHook(input)) return {};
  const workspace = claudeWorkspace(input.cwd);
  return runDevHostHook("claude-code", workspace, requiredText(input, "session_id"), (runtime, storageRoot) =>
    handleClaudeEvent(input, runtime, storageRoot));
}
