/**
 * Ends a hook process on Windows once the process that started it is gone.
 *
 * A coding agent that gives up on a slow hook kills the process it started. On macOS and Linux that is node itself,
 * since the `sh` launcher replaces itself with node (`exec`). On Windows it is the cmd.exe (Cursor) or PowerShell
 * (Codex) that runs node, or the hook launcher (Claude Code), and killing a parent there never kills its children: the
 * hook ran on, holding its session's lock, and the session's next hook waited up to 30 s for it. The agent has dropped
 * the hook by then, so nothing reads what it would answer.
 */

import { processState } from "../engine/process-liveness.js";

/** How often the parent is looked for. */
const PARENT_POLL_MS = 1_000;

export interface ParentWatch {
  platform?: NodeJS.Platform;
  /** The process that started this one; its pid stays the same after it exits. */
  parentPid?: number;
  /** Whether `pid` is a live process. */
  alive?: (pid: number) => boolean;
  exit?: (code: number) => void;
  intervalMs?: number;
}

/** Whether `pid` is a live process of any user. */
function processLives(pid: number): boolean {
  return processState(pid) !== "gone";
}

/**
 * On Windows, checks every second that the parent still runs and exits this process once it does not; elsewhere does
 * nothing. The timer never keeps the process alive. Returns a function that stops the watch.
 */
export function exitWhenParentGone(watch: ParentWatch = {}): () => void {
  if ((watch.platform ?? process.platform) !== "win32") return () => undefined;
  const parent = watch.parentPid ?? process.ppid;
  const alive = watch.alive ?? processLives;
  const exit = watch.exit ?? ((code: number) => process.exit(code));
  const timer = setInterval(() => {
    if (alive(parent)) return;
    clearInterval(timer);
    exit(0);
  }, watch.intervalMs ?? PARENT_POLL_MS);
  timer.unref();
  return () => clearInterval(timer);
}
