import {
  cursorSession,
  cursorWorkspace,
  handleCursorEvent,
  type CursorHookInput,
  type CursorHookOutput,
} from "./cursor-hook-adapter.js";
import { runDevHostHook } from "./dev-hook-runtime.js";
import { emitHookOutput } from "./hook-adapter.js";

export async function runCursorHook(input: CursorHookInput): Promise<CursorHookOutput> {
  const workspace = cursorWorkspace(input);
  return runDevHostHook("cursor", workspace, cursorSession(input).sessionId, (runtime, storageRoot) =>
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
