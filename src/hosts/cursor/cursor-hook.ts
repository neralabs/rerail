import {
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { emitHookOutput, launchedDirectly, readStdin } from "../../harness/hook-adapter.js";
import { hookRerailStarted, runHostHook, type HookOptions } from "../../harness/hook-host.js";

export { cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  const workspace = cursorWorkspace(input);
  if (!await hookRerailStarted("cursor", workspace, options.databasePath)) return {};
  return runHostHook("cursor", workspace, cursorSession(input).sessionId, input.hook_event_name === "sessionStart", options, (runtime, storageRoot) =>
    handleCursorEvent(input, runtime, storageRoot));
}

/**
 * Always exits 0, answering `{}` as Rerail does where it is not started, so Cursor carries on: a failed hook would put
 * an error on every tool call (design N11). The reason goes to stderr, which Cursor keeps out of the chat.
 */
export async function serveCursorHook(input: string): Promise<void> {
  let output: CursorHookOutput = {};
  try {
    output = await runCursorHook(JSON.parse(input) as CursorHookInput);
  } catch (error) {
    process.stderr.write(`rerail hook: ${error instanceof Error ? error.message : String(error)}\n`);
  }
  await emitHookOutput(output);
}

if (launchedDirectly(import.meta.url)) await serveCursorHook(await readStdin());
