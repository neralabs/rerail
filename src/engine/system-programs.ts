import { accessSync, constants, existsSync } from "node:fs";
import { delimiter, isAbsolute, join, win32 } from "node:path";

/**
 * Where Rerail finds the programs it starts, by absolute path. Windows looks for a bare program name in the current
 * folder before PATH, and Rerail runs inside repositories it does not trust, so a `git.exe`, `node.exe` or
 * `powershell.exe` committed to one would run in Rerail's place; the child-process helper refuses a bare name there
 * (`assertProgramPath`). Programs Windows ships come from System32 ({@link windowsSystemProgram}), others from PATH
 * ({@link programOnPath}), and Git from its own search (`gitExecutable` in `git.ts`).
 */

/**
 * The absolute path of a program Windows ships in System32. Rerail starts these by absolute path: Windows looks for a
 * bare program name in the current folder before PATH, and Rerail runs inside repositories it does not trust, so a
 * `powershell.exe` or `rundll32.exe` committed to one would run instead, and could read what Rerail hands it.
 */
export function windowsSystemProgram(name: string, env: NodeJS.ProcessEnv = process.env): string {
  return win32.join(env.SystemRoot || String.raw`C:\Windows`, "System32", name);
}

/** Windows PowerShell 5.1, which every Windows 10 and 11 install carries, by absolute path. */
export function windowsPowerShell(env: NodeJS.ProcessEnv = process.env): string {
  return windowsSystemProgram(win32.join("WindowsPowerShell", "v1.0", "powershell.exe"), env);
}

/** Where {@link programOnPath} looks: the environment whose PATH (and on Windows PATHEXT) it reads, and the platform. */
export interface ProgramSearch {
  env?: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform;
}

/** The name `env` holds PATH under: Windows keeps its own case (`Path`), and any spelling reads the same there. */
export function pathKey(env: NodeJS.ProcessEnv): string {
  return Object.keys(env).find(name => name.toUpperCase() === "PATH") ?? "PATH";
}

/** The extensions CreateProcess starts without a shell; a `.cmd`, `.bat` or `.js` needs one. */
const BINARY_EXTENSION = /^\.(?:exe|com)$/iu;

/**
 * The absolute path of the program PATH finds for `name`, as a spawn without a shell can start it, or undefined when
 * nothing on PATH is. An absolute `name` is returned as it is. On Windows only a `.exe` or `.com` counts, in PATHEXT's
 * order: Node refuses a `.cmd` or `.bat` there (EINVAL), and a version manager's `node.cmd` shim can come first on
 * PATH. Only absolute entries are read there, unquoted as cmd.exe reads them: a relative one names the current folder.
 * Elsewhere the first file this process may execute counts.
 */
export function programOnPath(name: string, { env = process.env, platform = process.platform }: ProgramSearch = {}): string | undefined {
  if (isAbsolute(name)) return name;
  const windows = platform === "win32";
  const entries = (env[pathKey(env)] ?? "").split(windows ? ";" : delimiter)
    .map(entry => (windows ? entry.replace(/^"(.*)"$/u, "$1") : entry))
    .filter(entry => (windows ? win32.isAbsolute(entry) : entry.length > 0));
  const extensions = windows ? binaryExtensions(env) : [""];
  for (const entry of entries) {
    for (const extension of extensions) {
      const candidate = join(entry, `${name}${extension}`);
      if (windows ? existsSync(candidate) : runnable(candidate)) return candidate;
    }
  }
  return undefined;
}

/** The extensions of PATHEXT, in its order, that a spawn without a shell starts: `.exe` and `.com`. */
function binaryExtensions(env: NodeJS.ProcessEnv): string[] {
  return (env.PATHEXT ?? env.Pathext ?? ".EXE;.CMD;.BAT;.COM").split(";").filter(extension => BINARY_EXTENSION.test(extension));
}

function runnable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
