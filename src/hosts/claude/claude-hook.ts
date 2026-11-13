import { hookText } from "../../engine/adapter-contract.js";
import { claudeWorkspace } from "./claude-event-adapter.js";
import { handleClaudeEvent, supportsClaudeHook } from "./claude-hook-adapter.js";
import { serveHook, serveHookEntry, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import { answerAutostartOff, hookRerailStarted, runHostHook, withDaemonDeadline, type HookOptions } from "../../harness/hook-host.js";

export { claudeHookOutput, handleClaudeEvent } from "./claude-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Claude");

/**
 * A session that starts with no daemon to record it, because `RERAIL_AUTOSTART` kept one from starting, says so in the
 * line Claude Code shows the person under the session's start (`answerAutostartOff`).
 */
export async function runClaudeHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsClaudeHook(input)) return {};
  const waiting = withDaemonDeadline(options);
  const workspace = claudeWorkspace(input.cwd);
  if (!await hookRerailStarted("claude-code", workspace, options.databasePath)) return {};
  const event = requiredText(input, "hook_event_name");
  const startsSession = event === "SessionStart";
  return await runHostHook("claude-code", workspace, requiredText(input, "session_id"), startsSession, waiting, (runtime, storageRoot) =>
    handleClaudeEvent(input, runtime, storageRoot), event)
    .catch((error: unknown) => answerAutostartOff(error, startsSession, line => ({ systemMessage: line })));
}

await serveHookEntry(import.meta.url, runClaudeHook);
