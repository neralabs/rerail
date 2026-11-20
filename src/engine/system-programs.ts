import { accessSync, constants, existsSync } from "node:fs";
import { delimiter, isAbsolute, join, posix, win32 } from "node:path";

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

/**
 * Whether `path` names one file from any current folder, as Windows reads it: it starts with a drive and a separator
 * (`C:\`), or with two separators (a server and share, `\\server\share\`, or a device path, `\\?\C:\`). Node's
 * `win32.isAbsolute` also takes a path from the root of the current drive (`\tools\git.exe`, `/git.exe`), which Windows
 * reads on the drive of the current folder, so it names another file on each drive.
 */
export function isFullyQualifiedWindowsPath(path: string): boolean {
  return /^(?:[A-Za-z]:[\\/]|[\\/]{2})/u.test(path);
}

/**
 * Whether `path` names one file from any current folder, as `platform` reads it (this machine's by default): on Windows
 * a fully qualified path ({@link isFullyQualifiedWindowsPath}), elsewhere an absolute one.
 */
export function namesOneFile(path: string, platform: NodeJS.Platform = process.platform): boolean {
  return platform === "win32" ? isFullyQualifiedWindowsPath(path) : posix.isAbsolute(path);
}

/** Windows PowerShell 5.1, which every Windows 10 and 11 install carries, by absolute path. */
export function windowsPowerShell(env: NodeJS.ProcessEnv = process.env): string {
  return windowsSystemProgram(win32.join("WindowsPowerShell", "v1.0", "powershell.exe"), env);
}

/** Where {@link programOnPath} looks: the environment whose PATH (and on Windows PATHEXT) it reads, and the platform. */
export interface ProgramSearch {
  env?: NodeJS.ProcessEnv;
  /**
   * The platform whose program start takes the answer: how PATH is split and unquoted, and which files count (a `.exe`
   * or `.com` in PATHEXT's order on Windows, a file this process may execute elsewhere). What a name or a PATH entry
   * written from a root names is read by this machine's rule ({@link namesOneFile}, {@link fromCurrentDrive}), since
   * the search reads this machine's disk: a test that plays Windows on Linux keeps its Linux folders on PATH.
   */
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
 * nothing on PATH is. A `name` written from a root is read as a PATH entry is ({@link onThisMachine}): as it is where it
 * names one file, and on Windows with the current folder's drive where it starts at the root of the current drive
 * (`\nodejs\node.exe`), or undefined inside the repository there. On Windows only a `.exe` or `.com` counts, in PATHEXT's
 * order: Node refuses a `.cmd` or `.bat` there (EINVAL), and a version manager's `node.cmd` shim can come first on
 * PATH. Entries are read there unquoted, as cmd.exe reads them, and only those that name one folder on this machine
 * ({@link searchedFolder}): a relative one names the current folder and is skipped. Elsewhere the first file this
 * process may execute counts.
 */
export function programOnPath(name: string, { env = process.env, platform = process.platform }: ProgramSearch = {}): string | undefined {
  if (isAbsolute(name)) return onThisMachine(name);
  const windows = platform === "win32";
  const extensions = windows ? binaryExtensions(env) : [""];
  const found = windows ? existsSync : runnable;
  for (const entry of searchedEntries(env, windows)) {
    const candidate = extensions.map(extension => join(entry, `${name}${extension}`)).find(path => found(path));
    if (candidate !== undefined) return candidate;
  }
  return undefined;
}

/**
 * The PATH folders {@link programOnPath} reads, in order. On Windows each entry is unquoted as cmd.exe reads it, and
 * read as the folder it names on this machine ({@link searchedFolder}); one that names none is skipped.
 */
function searchedEntries(env: NodeJS.ProcessEnv, windows: boolean): string[] {
  const written = env[pathKey(env)] ?? "";
  if (!windows) return written.split(delimiter).filter(entry => entry.length > 0);
  return written.split(";").flatMap(entry => searchedFolder(entry.replace(/^"(.*)"$/u, "$1")));
}

/**
 * The folder a Windows PATH entry names on this machine, as a list of one, or none. One that names one folder
 * ({@link namesOneFile}) is read as written, one from the root of the current drive (`\tools`) as
 * {@link fromCurrentDrive} reads it: a program start reads a path without a drive on the drive of the child's folder,
 * which can be another, so the search would check one file and the start run another.
 */
function searchedFolder(entry: string): string[] {
  const folder = onThisMachine(entry);
  return folder === undefined ? [] : [folder];
}

/**
 * The file or folder `path` names on this machine: `path` where it names one ({@link namesOneFile}), else, for one from
 * the root of the current drive on Windows, what {@link fromCurrentDrive} reads; undefined for one that names none.
 */
function onThisMachine(path: string): string | undefined {
  return namesOneFile(path) ? path : fromCurrentDrive(path);
}

/**
 * `path`, written from the root of the current drive (`\tools`, `/Git/bin/bash.exe`), named with the drive (or server
 * share) of this process's current folder, where Windows and Node read it on a Windows machine. Undefined where
 * `platform` (this machine's by default) is not Windows, where such a path has no drive to take; for a path that does
 * not start at a root; and where the path lies in the
 * repository nearest the current folder, as for a repository cloned to a drive's root: its files are the repository's,
 * not the person's programs, and Git's own search skips such a folder too. Only the nearest repository counts, as in
 * Git's search: from a submodule or a nested clone of a repository at a drive's root, the outer repository's files are
 * read as before.
 */
export function fromCurrentDrive(path: string, platform: NodeJS.Platform = process.platform): string | undefined {
  if (platform !== "win32" || !/^[\\/]/u.test(path)) return undefined;
  const here = process.cwd();
  const named = win32.resolve(here, path);
  const repository = enclosingRepository(here);
  // Both lie on the drive of the current folder, so a path outside the repository is reached from it by going up.
  return repository !== undefined && win32.relative(repository, named).split("\\")[0] !== ".." ? undefined : named;
}

/** The nearest folder at or above `folder` that holds a `.git` entry, read from disk without running Git. */
function enclosingRepository(folder: string): string | undefined {
  for (let current = folder; ; current = win32.dirname(current)) {
    if (existsSync(win32.join(current, ".git"))) return current;
    if (win32.dirname(current) === current) return undefined;
  }
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
