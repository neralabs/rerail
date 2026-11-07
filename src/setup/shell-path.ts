/**
 * Puts the launcher directory (`~/.rerail/bin`) on the person's PATH, so `rerail` runs from a new terminal.
 *
 * POSIX: a marked block in the startup file the login shell (`$SHELL`) reads. The block prepends the directory only
 * when PATH lacks it, so a file sourced twice (`.bash_profile` sourcing `.bashrc`) adds it once. Windows: the user
 * `Path` in `HKCU\Environment`, keeping its registry value kind so `%VAR%` entries stay unexpanded.
 *
 * Nothing is written when the running PATH has the directory already: the person manages it. `RERAIL_MODIFY_PATH=0`
 * turns the whole step off (dotfile managers; the test suite).
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile, rm, stat } from "node:fs/promises";
import { delimiter, join, resolve, win32 } from "node:path";
import { shellQuote } from "../harness/setup-screens.js";
import { writeTextFile } from "./harness-registration.js";

const BLOCK_START = "# >>> rerail >>>";
const BLOCK_END = "# <<< rerail <<<";
/**
 * The block as whole lines, start marker to the first end marker after it, with no other start marker between: a start
 * marker left on its own never reaches into a later block. CRLF files match too. A line starts at the start of the text
 * or after an LF (`(?<![^\n])`), never after a lone CR, which `^` with the `m` flag would also accept.
 */
const BLOCK = /(?<![^\n])# >>> rerail >>>\r?\n(?:(?!# >>> rerail >>>).*\r?\n)*?# <<< rerail <<<(?:\r?\n|$)/u;
/** The block with the blank line `addBlock` puts before it, when that line is still blank. */
const BLOCK_AND_GAP = /(?:(?<![^\n])\r?\n)?(?<![^\n])# >>> rerail >>>\r?\n(?:(?!# >>> rerail >>>).*\r?\n)*?# <<< rerail <<<(?:\r?\n|$)/u;
/** Where `Path` lives on Windows, as reports name it. */
export const WINDOWS_USER_PATH = String.raw`HKCU\Environment Path`;

export interface LauncherPathOptions {
  /** The launcher's directory, `<rerail home>/bin`. */
  binDir: string;
  /** The person's home directory, where shell startup files live. */
  home: string;
  env?: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform;
  dryRun?: boolean;
}

export type LauncherPathChange =
  /** `RERAIL_MODIFY_PATH=0`, or the running PATH has the directory already. */
  | { status: "skipped" }
  /** Every startup file (or the Windows user Path) names the directory already. */
  | { status: "unchanged"; locations: string[] }
  /** Written now, or planned for a dry run. */
  | { status: "changed"; locations: string[] }
  | { status: "failed"; reason: string };

/** What setup prints for `action`'s change, or undefined when there is nothing to say. */
export function describeLauncherPath(change: LauncherPathChange, binDir: string, action: "add" | "remove", dryRun = false): string | undefined {
  if (change.status === "failed") {
    return action === "add"
      ? `Could not add ${binDir} to PATH (${change.reason}); add it yourself to run \`rerail\` directly.`
      : `Could not remove ${binDir} from PATH (${change.reason}); remove it yourself.`;
  }
  if (change.status !== "changed") return undefined;
  const where = change.locations.join(" and ");
  if (action === "remove") return `${dryRun ? "Would remove" : "Removed"} ${binDir} from PATH in ${where}.`;
  if (dryRun) return `Would add ${binDir} to PATH in ${where}.`;
  return `Added ${binDir} to PATH in ${where}; open a new terminal to run \`rerail\` directly.`;
}

function onRunningPath(binDir: string, env: NodeJS.ProcessEnv, platform: NodeJS.Platform): boolean {
  const windows = platform === "win32";
  const key = Object.keys(env).find(name => name.toUpperCase() === "PATH") ?? "PATH";
  const normal = (path: string) => {
    const resolved = resolve(path).replace(/[\\/]+$/u, "");
    return windows ? resolved.toLowerCase() : resolved;
  };
  const target = normal(binDir);
  return (env[key] ?? "").split(windows ? ";" : delimiter).some(entry => entry.trim() !== "" && normal(entry.trim()) === target);
}

/** Add the launcher directory to PATH where the person's shell will find it. Never throws: setup goes on either way. */
export async function addLauncherToPath(options: LauncherPathOptions): Promise<LauncherPathChange> {
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  if (env.RERAIL_MODIFY_PATH === "0" || onRunningPath(options.binDir, env, platform)) return { status: "skipped" };
  try {
    if (platform === "win32") return windowsUserPath(options.binDir, options.dryRun === true ? "check" : "add", env, options.home);
    const files = startupFiles(options.home, env);
    const changed: string[] = [];
    for (const file of files) {
      const existing = await readText(file.path);
      const text = addBlock(existing?.text ?? "", file.block(options.binDir));
      if (text === (existing?.text ?? "")) continue;
      changed.push(file.path);
      if (options.dryRun !== true) await writeTextFile(file.path, text, existing ?? { exists: false, mode: 0o644 });
    }
    return changed.length === 0 ? { status: "unchanged", locations: files.map(file => file.path) } : { status: "changed", locations: changed };
  } catch (error) {
    return { status: "failed", reason: error instanceof Error ? error.message : String(error) };
  }
}

/** Take out what `addLauncherToPath` wrote, from every startup file it may have used. Never throws. */
export async function removeLauncherFromPath(options: LauncherPathOptions): Promise<LauncherPathChange> {
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  if (env.RERAIL_MODIFY_PATH === "0") return { status: "skipped" };
  try {
    if (platform === "win32") return windowsUserPath(options.binDir, options.dryRun === true ? "check-remove" : "remove", env, options.home);
    const changed: string[] = [];
    for (const path of allStartupFiles(options.home, env)) {
      const existing = await readText(path);
      if (existing === undefined || !BLOCK.test(existing.text)) continue;
      changed.push(path);
      if (options.dryRun === true) continue;
      // Only a line that is still blank goes with the block, so the lines around it stay as they are.
      const text = existing.text.replace(BLOCK_AND_GAP, "");
      // The fish file is Rerail's own; a shared startup file stays, however little is left in it.
      if (path.endsWith("rerail.fish") && text.trim() === "") await rm(path, { force: true });
      else await writeTextFile(path, text, existing);
    }
    return changed.length === 0 ? { status: "unchanged", locations: [] } : { status: "changed", locations: changed };
  } catch (error) {
    return { status: "failed", reason: error instanceof Error ? error.message : String(error) };
  }
}

/** `text` with exactly one Rerail block, set to `block`, after a blank line. */
function addBlock(text: string, block: string): string {
  // A function, so a `$` in the launcher's path is never read as a replacement pattern.
  if (BLOCK.test(text)) return text.replace(BLOCK, () => `${block}\n`);
  if (text === "") return `${block}\n`;
  return `${text}${text.endsWith("\n") ? "" : "\n"}\n${block}\n`;
}

interface StartupFile {
  path: string;
  block(binDir: string): string;
}

function posixBlock(binDir: string): string {
  const quoted = shellQuote(binDir, "linux");
  return [
    BLOCK_START,
    "# The `rerail` launcher, added by `rerail setup`; `rerail install --remove` takes it out.",
    `case ":\${PATH}:" in`,
    `  *:${quoted}:*) ;;`,
    `  *) PATH=${quoted}":\${PATH}"; export PATH ;;`,
    "esac",
    BLOCK_END,
  ].join("\n");
}

function fishBlock(binDir: string): string {
  const quoted = `'${binDir.replaceAll("\\", "\\\\").replaceAll("'", "\\'")}'`;
  return [
    BLOCK_START,
    "# The `rerail` launcher, added by `rerail setup`; `rerail install --remove` takes it out.",
    `if not contains -- ${quoted} $PATH`,
    `    set -gx PATH ${quoted} $PATH`,
    "end",
    BLOCK_END,
  ].join("\n");
}

function zshrc(home: string, env: NodeJS.ProcessEnv): string {
  // An empty value is unset (as zsh and the XDG spec read it), never a path relative to the current folder.
  return join(env.ZDOTDIR || home, ".zshrc");
}

function fishConfig(home: string, env: NodeJS.ProcessEnv): string {
  return join(env.XDG_CONFIG_HOME || join(home, ".config"), "fish", "conf.d", "rerail.fish");
}

/**
 * The files `$SHELL` reads for a new terminal. Bash: `.bashrc` (interactive shells) and the login file bash reads
 * first of `.bash_profile`, `.bash_login`, `.profile`; creating `.bash_profile` would hide an existing `.profile`.
 */
function startupFiles(home: string, env: NodeJS.ProcessEnv): StartupFile[] {
  const shell = (env.SHELL ?? "").split("/").pop() ?? "";
  if (shell === "zsh") return [{ path: zshrc(home, env), block: posixBlock }];
  if (shell === "fish") return [{ path: fishConfig(home, env), block: fishBlock }];
  if (shell === "bash") {
    const login = [".bash_profile", ".bash_login", ".profile"].map(name => join(home, name)).find(path => existsSync(path)) ?? join(home, ".profile");
    return [{ path: join(home, ".bashrc"), block: posixBlock }, { path: login, block: posixBlock }];
  }
  return [{ path: join(home, ".profile"), block: posixBlock }];
}

/** Every file `startupFiles` can name, whatever the shell is now. */
function allStartupFiles(home: string, env: NodeJS.ProcessEnv): string[] {
  return [
    zshrc(home, env),
    ...[".bashrc", ".bash_profile", ".bash_login", ".profile"].map(name => join(home, name)),
    fishConfig(home, env),
  ];
}

async function readText(path: string): Promise<{ text: string; exists: true; mode: number } | undefined> {
  try {
    const [text, info] = await Promise.all([readFile(path, "utf8"), stat(path)]);
    return { text, exists: true, mode: info.mode & 0o777 };
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}

/**
 * Reads and writes the raw (unexpanded) user `Path`, keeping its value kind. Setting a throwaway user variable through
 * `SetEnvironmentVariable` broadcasts WM_SETTINGCHANGE, so Explorer hands new terminals the new `Path`.
 */
const WINDOWS_SCRIPT = String.raw`
$ErrorActionPreference = 'Stop'
$entry = $env:RERAIL_PATH_ENTRY
$mode = $env:RERAIL_PATH_MODE
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
$hasPath = $key.GetValueNames() -contains 'Path'
$raw = if ($hasPath) { [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { '' }
$kind = if ($hasPath) { $key.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
function Normal([string]$path) { [Environment]::ExpandEnvironmentVariables($path).Trim().TrimEnd('\').ToLowerInvariant() }
$parts = @($raw -split ';' | Where-Object { $_ -ne '' })
$target = Normal $entry
$kept = @($parts | Where-Object { (Normal $_) -ne $target })
$present = $kept.Count -ne $parts.Count
if ($mode -eq 'check') { if ($present) { 'unchanged' } else { 'changed' }; exit 0 }
if ($mode -eq 'check-remove') { if ($present) { 'changed' } else { 'unchanged' }; exit 0 }
if ($mode -eq 'add') {
  if ($present) { 'unchanged'; exit 0 }
  $next = @($parts + $entry) -join ';'
} else {
  if (-not $present) { 'unchanged'; exit 0 }
  $next = $kept -join ';'
}
$key.SetValue('Path', $next, $kind)
[Environment]::SetEnvironmentVariable('RERAIL_PATH_REFRESH', '1', 'User')
[Environment]::SetEnvironmentVariable('RERAIL_PATH_REFRESH', $null, 'User')
'changed'
`;

function windowsUserPath(binDir: string, mode: "check" | "add" | "check-remove" | "remove", env: NodeJS.ProcessEnv, home: string): LauncherPathChange {
  // By absolute path, from the home folder: Windows looks in the current folder (the repository) before PATH.
  const powershell = win32.join(env.SystemRoot || String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const result = spawnSync(
    powershell,
    ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", Buffer.from(WINDOWS_SCRIPT, "utf16le").toString("base64")],
    { cwd: home, env: { ...env, RERAIL_PATH_ENTRY: binDir, RERAIL_PATH_MODE: mode }, encoding: "utf8", timeout: 30_000, windowsHide: true },
  );
  const answer = (result.stdout ?? "").trim().split(/\r?\n/u).pop();
  if (result.status !== 0 || (answer !== "changed" && answer !== "unchanged")) {
    const detail = (result.error?.message ?? result.stderr ?? "").trim();
    return { status: "failed", reason: `powershell.exe ${detail === "" ? `exited ${result.status ?? "early"}` : detail}` };
  }
  return { status: answer, locations: [WINDOWS_USER_PATH] };
}
