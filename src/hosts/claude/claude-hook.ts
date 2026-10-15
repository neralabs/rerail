import { hookText } from "../../engine/adapter-contract.js";
import { claudeWorkspace } from "./claude-event-adapter.js";
import { handleClaudeEvent, supportsClaudeHook } from "./claude-hook-adapter.js";
import { launchedDirectly, serveHook, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import { hookRerailStarted, runHostHook, type HookOptions } from "../../harness/hook-host.js";

export { claudeHookOutput, handleClaudeEvent } from "./claude-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Claude");

export async function runClaudeHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsClaudeHook(input)) return {};
  const workspace = claudeWorkspace(input.cwd);
  if (!await hookRerailStarted("claude-code", workspace, options.databasePath)) return {};
  return runHostHook("claude-code", workspace, requiredText(input, "session_id"), requiredText(input, "hook_event_name") === "SessionStart", options, (runtime, storageRoot) =>
    handleClaudeEvent(input, runtime, storageRoot));
}

if (launchedDirectly(import.meta.url)) await serveHook(runClaudeHook);
