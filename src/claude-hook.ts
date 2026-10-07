import { hookText } from "./adapter-contract.js";
import { claudeWorkspace } from "./claude-event-adapter.js";
import { handleClaudeEvent, supportsClaudeHook } from "./claude-hook-adapter.js";
import { hookRerailStarted } from "./repository-start.js";
import { launchedDirectly, serveHook, type HookInput, type HookOutput } from "./hook-adapter.js";
import { runHostHook, type HookOptions } from "./hook-host.js";

export { claudeHookOutput, handleClaudeEvent } from "./claude-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Claude");

export async function runClaudeHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsClaudeHook(input)) return {};
  const workspace = claudeWorkspace(input.cwd);
  if (!await hookRerailStarted("claude-code", workspace, options.databasePath)) return {};
  return runHostHook("claude-code", workspace, requiredText(input, "session_id"), options, (runtime, storageRoot) =>
    handleClaudeEvent(input, runtime, storageRoot));
}

if (launchedDirectly(import.meta.url)) await serveHook(runClaudeHook);
