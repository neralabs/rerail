/**
 * Every child process Rerail starts goes through this module, which starts it with `windowsHide: true`.
 *
 * Node defaults `windowsHide` to false. On Windows, a console program (git, node, npm, PowerShell) started by a
 * process that has no console of its own, such as the daemon or the session-start autostart, then gets a new console
 * with a visible window, and Windows 11 hands each one to Windows Terminal, which takes focus. With `windowsHide: true`
 * libuv starts the child with SW_HIDE and, when no stdio is inherited, with CREATE_NO_WINDOW, so it opens no window.
 * The option does nothing on macOS and Linux. `npm run check:child-process` fails on any other `node:child_process`
 * import in `src/`.
 */
import * as childProcess from "node:child_process";
import { promisify } from "node:util";

const HIDDEN = { windowsHide: true } as const;

/**
 * `args`, the arguments of a child_process call, with `windowsHide: true` set in its options object: after the
 * argument array when there is one, else after the command. A callback in that place moves one along.
 */
export function withHiddenWindow(args: readonly unknown[]): unknown[] {
  const at = Array.isArray(args[1]) ? 2 : 1;
  const options = args[at];
  if (typeof options === "function") return [...args.slice(0, at), HIDDEN, ...args.slice(at)];
  // Options are an object, or absent (undefined or null); spreading either keeps what the caller set.
  return [...args.slice(0, at), { ...(options as object | null | undefined), ...HIDDEN }, ...args.slice(at + 1)];
}

type Starter = "spawn" | "spawnSync" | "execFile" | "execFileSync";

/** `name` of node:child_process, read at each call so a test's module mock is the one called. */
function hidden<Name extends Starter>(name: Name): (typeof childProcess)[Name] {
  // Reflect.apply returns any; the cast restores the overloads of the function it calls.
  return ((...args: unknown[]) => Reflect.apply(childProcess[name], undefined, withHiddenWindow(args))) as (typeof childProcess)[Name];
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
    Reflect.apply(childProcess.execFile, undefined, [...withHiddenWindow(args), settle]);
  }),
});

/**
 * Starts a desktop program whose window the person is meant to see: the browser opener, and nothing else. A Windows
 * GUI program hands the show state it was started with to the window it opens, so `windowsHide` there would hide the
 * browser. Never pass a console program here.
 */
export function spawnShown(command: string, args: readonly string[], options: childProcess.SpawnOptions): childProcess.ChildProcess {
  return childProcess.spawn(command, args, { ...options, windowsHide: false });
}
