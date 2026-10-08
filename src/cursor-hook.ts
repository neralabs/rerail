import { hookRerailStarted } from "./repository-start.js";
import {
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { emitHookOutput, launchedDirectly, readStdin } from "./hook-adapter.js";
import { runHostHook, type HookOptions } from "./hook-host.js";

export { cursorHookEvent, cursorHookOutput, handleCursorEvent } from "./cursor-hook-adapter.js";

export async function runCursorHook(
  input: CursorHookInput,
  options: HookOptions = {},
): Promise<CursorHookOutput> {
  const workspace = cursorWorkspace(input);
  if (!await hookRerailStarted("cursor", workspace, options.databasePath)) return {};
  return runHostHook("cursor", workspace, cursorSession(input).sessionId, options, (runtime, storageRoot) =>
    handleCursorEvent(input, runtime, storageRoot));
}

export async function serveCursorHook(input: string): Promise<void> {
  try {
    await emitHookOutput(await runCursorHook(JSON.parse(input) as CursorHookInput));
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}

if (launchedDirectly(import.meta.url)) await serveCursorHook(await readStdin());
