import { hookText } from "../../engine/adapter-contract.js";
import { claudeWorkspace } from "./claude-event-adapter.js";
import { handleClaudeEvent, supportsClaudeHook } from "./claude-hook-adapter.js";
import { serveHook, serveHookEntry, type HookInput, type HookOutput } from "../../harness/hook-adapter.js";
import { runHostHook, type HookOptions, type HookPayload } from "../../harness/hook-host.js";

export { claudeHookOutput, handleClaudeEvent } from "./claude-hook-adapter.js";
export { serveHook };

const { required: requiredText } = hookText("Claude");

/** A Claude Code hook's payload: the event's name, then its session. */
function claudePayload(input: HookInput): HookPayload {
  return {
    workspace: () => claudeWorkspace(input.cwd),
    event: () => {
      const name = requiredText(input, "hook_event_name");
      return { name, sessionId: requiredText(input, "session_id"), startsSession: name === "SessionStart" };
    },
  };
}

export async function runClaudeHook(input: HookInput, options: HookOptions = {}): Promise<HookOutput> {
  if (!supportsClaudeHook(input)) return {};
  return runHostHook("claude-code", claudePayload(input), options, (runtime, storageRoot) => handleClaudeEvent(input, runtime, storageRoot));
}

await serveHookEntry(import.meta.url, runClaudeHook);
