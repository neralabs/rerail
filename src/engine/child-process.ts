/**
 * Every child process Rerail starts goes through this module, which starts it with `windowsHide: true`, unless it is
 * to share this process's console, which has no window ({@link sharesConsole}).
 *
 * Node defaults `windowsHide` to false. On Windows, a console program (git, node, npm, PowerShell) started by a
 * process that has no console of its own then gets a new console with a visible window, and Windows 11 hands each one
 * to Windows Terminal, which takes focus. With `windowsHide: true` libuv starts the child with SW_HIDE and, when no
 * stdio is inherited, with CREATE_NO_WINDOW: a console of its own with no window, so it opens none, but Windows starts
 * a console host (conhost.exe) for it. A child started without it in a process that has a console shares that console
 * and starts no console host; the daemon's console has no window, so its Git runs that way. The option does nothing on
 * macOS and Linux. `npm run check:owned-builtins` fails on any other `node:child_process` import in `src/`.
 *
 * On Windows it also refuses a program named by anything but an absolute path (`assertProgramPath`).
 */
import * as childProcess from "node:child_process";
import { closeSync, openSync } from "node:fs";
import { win32 } from "node:path";
import { promisify } from "node:util";
import { windowsSystemProgram } from "./system-programs.js";

/**
 * Set to `1` in the environment of a process that runs in a console with no window: `launchDetached` sets it for the
 * process it starts through the native spawn on Windows, the daemon and the `rerail daemon start` a session's
 * autostart runs. A process this module starts inherits it, and its console has no window either: it shares this one,
 * or has one of its own with none.
 */
export const WINDOWLESS_CONSOLE = "RERAIL_WINDOWLESS_CONSOLE";

/**
 * Whether this process has a console. Windows opens `CONOUT$`, the screen buffer of the process's console, only in a
 * process that has one; in one started with DETACHED_PROCESS the open fails with EBADF.
 */
export function hasConsole(): boolean {
  try {
    closeSync(openSync(String.raw`\\.\CONOUT$`, "r+"));
    return true;
  } catch {
    return false;
  }
}

/**
 * Whether this process's children share its console instead of each getting one of its own: on Windows, when `env`
 * says the console has no window ({@link WINDOWLESS_CONSOLE}) and the process has a console (`attached`). A child that
 * shares a console opens no window and starts no console host. Everywhere else children keep `windowsHide`. A process
 * with no console can still carry the variable, when a program outside Rerail started it with DETACHED_PROCESS, and a
 * child started there without `windowsHide` would get a new console with a window. In a process whose console a person
 * sees, such as the CLI in a terminal or an agent host, a child sharing it could write there, as PowerShell draws its
 * progress, and read what the person types.
 */
export function sharesConsole(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  attached: () => boolean = hasConsole,
): boolean {
  return platform === "win32" && env[WINDOWLESS_CONSOLE] === "1" && attached();
}

let shared: boolean | undefined;

/** {@link sharesConsole} for this process, asked at its first child: a process keeps the console it started with. */
function childrenShareConsole(): boolean {
  shared ??= sharesConsole();
  return shared;
}

/**
 * Where a child_process call's options are, as Node reads them: after the argument array, or after an argument list
 * given as undefined or null when more follows (`spawn(command, undefined, options)`), else after the command.
 */
function optionsAt(args: readonly unknown[]): 1 | 2 {
  return Array.isArray(args[1]) || (args[1] == null && args.length > 2) ? 2 : 1;
}

/**
 * `args`, the arguments of a child_process call, with `windowsHide` set in its options object, where Node reads it
 * ({@link optionsAt}): true, or false when `hide` is, for a child that shares this process's console. A callback in
 * that place moves one along.
 */
export function withHiddenWindow(args: readonly unknown[], hide = true): unknown[] {
  const window = { windowsHide: hide };
  const at = optionsAt(args);
  const options = args[at];
  if (typeof options === "function") return [...args.slice(0, at), window, ...args.slice(at)];
  // Options are an object, or absent (undefined or null); spreading either keeps what the caller set.
  return [...args.slice(0, at), { ...(options as object | null | undefined), ...window }, ...args.slice(at + 1)];
}

/**
 * Refuses, on Windows, to start a program named by a bare name or a relative path: Windows looks for one in the current
 * folder before PATH, and Rerail runs inside repositories it does not trust, so a `git.exe`, `node.exe` or
 * `powershell.exe` committed to one would run in Rerail's place and get what Rerail hands it. Rerail names every program
 * by absolute path there, found as `system-programs.ts` finds it. A command line run with `shell: true` is the shell's
 * to read, and is let through.
 */
export function assertProgramPath(args: readonly unknown[], platform: NodeJS.Platform = process.platform): void {
  if (platform !== "win32") return;
  const [command] = args;
  const options = args[optionsAt(args)];
  const shell = typeof options === "object" && options !== null && "shell" in options && Boolean(options.shell);
  if (typeof command === "string" && !shell && !win32.isAbsolute(command)) {
    throw new Error(`Rerail starts a program on Windows by its absolute path, never from the current folder; it was asked for ${command}`);
  }
}

type Starter = "spawn" | "spawnSync" | "execFile" | "execFileSync";

/** `name` of node:child_process, read at each call so a test's module mock is the one called. */
function hidden<Name extends Starter>(name: Name): (typeof childProcess)[Name] {
  // Reflect.apply returns any; the cast restores the overloads of the function it calls.
  return ((...args: unknown[]) => {
    assertProgramPath(args);
    return Reflect.apply(childProcess[name], undefined, withHiddenWindow(args, !childrenShareConsole()));
  }) as (typeof childProcess)[Name];
}

export const spawn = hidden("spawn");
export const spawnSync = hidden("spawnSync");
export const execFileSync = hidden("execFileSync");

/**
 * `execFile`, which `promisify` turns into the form it gives Node's own: a promise of `{ stdout, stderr }`, rejected
 * with an error that carries both.
 */
export const execFile: typeof childProcess.execFile = Object.assign(hidden("execFile"), {
  [promisify.custom]: async (...args: unknown[]): Promise<{ stdout: unknown; stderr: unknown }> => await new Promise((resolve, reject) => {
    const settle = (error: Error | null, stdout: unknown, stderr: unknown): void => {
      if (error === null) resolve({ stdout, stderr });
      else reject(Object.assign(error, { stdout, stderr }));
    };
    assertProgramPath(args);
    Reflect.apply(childProcess.execFile, undefined, [...withHiddenWindow(args, !childrenShareConsole()), settle]);
  }),
});

/** Ends the process tree under `pid` on Windows, calling `failed` when that could not be done; tests pass their own. */
export type TreeKill = (pid: number, failed: () => void) => void;

/**
 * `taskkill /T /F`, which walks the tree from `pid`: the process must still run for it to find the rest. A taskkill
 * that cannot start reaches `failed` like one that fails, and never throws: this runs from abort listeners and timers,
 * where a throw is an uncaught exception.
 */
function taskkillTree(pid: number, failed: () => void): void {
  const taskkill = startTaskkill(pid);
  if (taskkill === undefined) {
    failed();
    return;
  }
  taskkill.on("error", failed);
  taskkill.on("exit", code => {
    if (code !== 0) failed();
  });
}

/**
 * `taskkill /T /F /PID pid`, started; undefined when spawn threw or answered with no child. Node's spawn always answers
 * with one, so the return type admits what a stand-in for it can do.
 */
function startTaskkill(pid: number): childProcess.ChildProcess | undefined {
  try {
    return spawn(windowsSystemProgram("taskkill.exe"), ["/T", "/F", "/PID", String(pid)], { stdio: "ignore" });
  } catch {
    return undefined;
  }
}

/** How {@link endProcessTree} reaches the processes a child started. */
export interface ProcessTreeOptions {
  /** Whether the child was started `detached` on macOS and Linux, where it then leads a process group of its own. */
  group?: boolean;
  platform?: NodeJS.Platform;
  killTree?: TreeKill;
}

/**
 * Ends `child` and the processes it started. On Windows killing a process leaves its children running (Git for
 * Windows' launcher runs the real Git as its child, Git Bash's launcher the real bash), so `taskkill /T /F` ends the
 * tree from `child` while it still runs, since taskkill finds the tree from a live root, and `child` alone is killed
 * only when taskkill cannot start or fails. On macOS and Linux a child started `detached` leads its own process group,
 * which goes whole; any other child is killed alone.
 */
export function endProcessTree(
  child: Pick<childProcess.ChildProcess, "pid" | "kill">,
  { group = false, platform = process.platform, killTree = taskkillTree }: ProcessTreeOptions = {},
): void {
  const killChild = (): void => {
    try {
      child.kill("SIGKILL");
    } catch { /* It may have exited already. */ }
  };
  if (child.pid === undefined) killChild();
  else if (platform === "win32") killTree(child.pid, killChild);
  else if (!group) killChild();
  else {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      killChild();
    }
  }
}

/**
 * Starts a desktop program whose window the person is meant to see: the browser opener, and nothing else. A Windows
 * GUI program hands the show state it was started with to the window it opens, so `windowsHide` there would hide the
 * browser. Never pass a console program here.
 */
export function spawnShown(command: string, args: readonly string[], options: childProcess.SpawnOptions): childProcess.ChildProcess {
  assertProgramPath([command, args, options]);
  return childProcess.spawn(command, args, { ...options, windowsHide: false });
}
