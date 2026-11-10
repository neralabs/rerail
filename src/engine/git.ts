import type { ChildProcess } from "node:child_process";
import { accessSync, constants, existsSync, realpathSync, statSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { dirname, join, posix, resolve, win32 } from "node:path";
import { execFile, spawn, spawnSync } from "./child-process.js";

/** What Rerail looks for Git with: the system, the account's home, `PATH`, and the folder it runs in. */
export interface GitSearch {
  platform: NodeJS.Platform;
  home: string;
  /** The `PATH` value, undefined when unset. */
  path: string | undefined;
  /** The folder Rerail runs in, normally inside the repository a coding agent works on; undefined once deleted. */
  cwd: string | undefined;
}

/**
 * Where Git is looked for, in order: each `PATH` entry Rerail can trust, then the usual install locations, so a Git
 * the person put first on `PATH` (MacPorts, mise, a build of their own) is the one Rerail runs. Rerail runs Git from
 * hooks in whatever environment the coding agent started in, inside repositories it does not trust, so a `PATH` entry
 * a repository could control is skipped: a relative one (which includes the current folder), one inside the
 * repository Rerail runs in (direnv's `PATH_add`), and one inside a `node_modules` folder (`node_modules/.bin`). Rerail
 * resolves the absolute path itself and runs that, so Windows never looks in the current folder first. "The repository
 * Rerail runs in" is the nearest folder with a `.git` above the current one, or above the folder {@link searchGitFrom}
 * named (the daemon runs in the Rerail home and names the repository it serves): an entry inside an outer checkout
 * (around a nested worktree or submodule), or any repository when Rerail runs from outside it, is not skipped. The Git
 * found first is kept for the process. The fixed locations come after `PATH`: the system ones, then the per-user installs
 * whose path only the person's account decides (Git for Windows installed for one user, scoop's Git, Linuxbrew's, a
 * Nix profile's), built from the account's home, never from the environment: `LOCALAPPDATA` can be set by a
 * repository as easily as PATH.
 */
export function gitCandidates(search: GitSearch = currentSearch(), probe: GitPathProbe = machine): string[] {
  return [...candidateOrder(search, gitTrust(search, probe))];
}

/** Where this process looks for Git from: the platform, the account's home, `PATH` and the folder it runs in. */
function currentSearch(): GitSearch {
  return { platform: process.platform, home: accountHome(), path: process.env.PATH, cwd: currentFolder() };
}

/**
 * {@link gitCandidates} one at a time, each once, so a search can stop at the first Git that runs. Windows paths ignore
 * case, so there `C:\Program Files` and `c:\program files` are one candidate, kept as first spelled.
 */
function* candidateOrder(search: GitSearch, trust: GitTrust): Generator<string> {
  const seen = new Set<string>();
  for (const candidates of [pathCandidates(search, trust), installLocations(search.platform, search.home)]) {
    for (const candidate of candidates) {
      const key = trust.windows ? candidate.toLowerCase() : candidate;
      if (seen.has(key)) continue;
      seen.add(key);
      yield candidate;
    }
  }
}

let searchFolder: string | undefined;

/**
 * Has Git looked for as if Rerail ran in `folder` (undefined: the current folder again). The daemon runs in the Rerail
 * home, not the repository it serves, and calls this before its first Git call, so a `PATH` entry inside that repository
 * is still skipped and a home that is itself a Git repository (dotfiles) does not hide the person's own tools.
 */
export function searchGitFrom(folder: string | undefined): void {
  searchFolder = folder;
}

/**
 * The folder Git is looked for from: the one {@link searchGitFrom} named, else the current folder, or undefined when
 * that was deleted under the process (`process.cwd()` throws `ENOENT`).
 */
function currentFolder(): string | undefined {
  if (searchFolder !== undefined) return searchFolder;
  try {
    return process.cwd();
  } catch {
    return undefined;
  }
}

/** What {@link gitCandidates} asks of the disk; tests pass their own. */
export interface GitPathProbe {
  exists(path: string): boolean;
  /** The path with every symlink resolved, or the path itself when it cannot be resolved. */
  realpath(path: string): string;
}

function installLocations(platform: NodeJS.Platform, home: string): string[] {
  if (platform === "win32") {
    // Windows installed on another drive keeps Program Files there; the account's home is on that drive too.
    const drive = win32.parse(home).root || "C:\\";
    return [
      String.raw`C:\Program Files\Git\cmd\git.exe`,
      String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
      win32.join(drive, "Program Files", "Git", "cmd", "git.exe"),
      win32.join(drive, "Program Files (x86)", "Git", "cmd", "git.exe"),
      win32.join(home, "AppData", "Local", "Programs", "Git", "cmd", "git.exe"),
      win32.join(home, "scoop", "apps", "git", "current", "cmd", "git.exe"),
    ];
  }
  return [
    "/usr/bin/git",
    "/usr/local/bin/git",
    "/opt/homebrew/bin/git",
    "/home/linuxbrew/.linuxbrew/bin/git",
    "/run/current-system/sw/bin/git",
    posix.join(home, ".nix-profile", "bin", "git"),
  ];
}

/** What decides whether Rerail may run a program from a folder: the repository it runs in, and the platform's paths. */
interface GitTrust {
  readonly windows: boolean;
  readonly paths: typeof posix;
  /**
   * The repository Rerail runs in by the path as written and with symlinks resolved, both sides: macOS's /var is
   * /private/var, and a link in the repository can point out of it. Empty outside any repository.
   */
  readonly repository: readonly string[];
  readonly probe: GitPathProbe;
}

function gitTrust(search: GitSearch, probe: GitPathProbe): GitTrust {
  const windows = search.platform === "win32";
  const paths = windows ? win32 : posix;
  const repository = search.cwd === undefined ? undefined : enclosingRepository(paths.resolve(search.cwd), paths, probe);
  return { windows, paths, repository: repository === undefined ? [] : [repository, probe.realpath(repository)], probe };
}

/**
 * Whether Rerail may run a program from `folder`: an absolute folder outside every `node_modules`, and outside the
 * repository it runs in by its path and by its real path.
 */
function trustedFolder(folder: string, { windows, paths, repository, probe }: GitTrust): boolean {
  if (!(windows ? /^(?:[A-Za-z]:[\\/]|\\\\)/.test(folder) : folder.startsWith("/"))) return false;
  if (folder.split(/[\\/]/).some(part => (windows ? part.toLowerCase() : part) === "node_modules")) return false;
  if (repository.length === 0) return true;
  const absolute = paths.resolve(folder);
  return ![absolute, probe.realpath(absolute)].some(form => repository.some(root => within(root, form, paths)));
}

/** The Git on each `PATH` entry Rerail may run a program from, in order; each entry is resolved only when reached. */
function* pathCandidates(search: GitSearch, trust: GitTrust): Generator<string> {
  if (search.path === undefined) return;
  for (const written of search.path.split(trust.windows ? ";" : ":")) {
    // Windows allows a quoted entry, which cmd.exe and CreateProcess unquote.
    const entry = trust.windows ? written.replace(/^"(.*)"$/, "$1") : written;
    if (trustedFolder(entry, trust)) yield trust.paths.join(entry, trust.windows ? "git.exe" : "git");
  }
}

/** The nearest folder at or above `folder` that holds a `.git` entry, read from disk without running Git. */
function enclosingRepository(folder: string, paths: typeof posix, probe: GitPathProbe): string | undefined {
  for (let current = folder; ; current = paths.dirname(current)) {
    if (probe.exists(paths.join(current, ".git"))) return current;
    if (paths.dirname(current) === current) return undefined;
  }
}

function within(folder: string, path: string, paths: typeof posix): boolean {
  const relative = paths.relative(folder, path);
  return relative === "" || (relative !== ".." && !relative.startsWith(`..${paths.sep}`) && !paths.isAbsolute(relative));
}

/**
 * The account's own home, not `HOME`: the install's health check runs Rerail with a scratch `HOME`, and it must find
 * the same Git as the person's sessions do. An account without a user database entry (a container's arbitrary uid)
 * falls back to `HOME`.
 */
function accountHome(): string {
  try {
    return userInfo().homedir;
  } catch {
    return homedir();
  }
}

export class GitUnavailableError extends Error {
  /** Every path Rerail looked for Git at. */
  readonly looked: readonly string[];

  constructor(looked: readonly string[]) {
    super(`Git is not installed where Rerail looks for it: ${looked.join(", ")}`);
    this.name = "GitUnavailableError";
    this.looked = looked;
  }
}

let probed: FoundGit | undefined;

/** What {@link gitInstalled} asks of the machine; tests pass their own. */
export interface GitProbe {
  exists(path: string): boolean;
  /** Whether `path` is a regular file (through symlinks) this process may execute, as a shell requires. */
  runnable(path: string): boolean;
  /** The developer folder `xcode-select` points at, or undefined when none is set. */
  developerDir(): string | undefined;
}

const machine: GitProbe & GitPathProbe = {
  exists: existsSync,
  runnable(path) {
    try {
      // Windows has no execute bit (X_OK checks existence there); a git.exe that is a file is what it runs.
      if (!statSync(path).isFile()) return false;
      accessSync(path, constants.X_OK);
      return true;
    } catch {
      return false;
    }
  },
  realpath(path) {
    try {
      return realpathSync.native(path);
    } catch {
      return path;
    }
  },
  developerDir() {
    // Without DEVELOPER_DIR, which overrides the answer and which a repository's environment could set.
    const selected = spawnSync("/usr/bin/xcode-select", ["-p"], { encoding: "utf8", env: { PATH: "/usr/bin:/bin" } });
    return selected.status === 0 ? selected.stdout.trim() : undefined;
  },
};

/**
 * Whether `path` is a Git that runs: a regular file this process may execute, so a folder named `git` or a `git`
 * without its execute bit is passed over, as a shell passes over them. On macOS `/usr/bin/git` is always there, a stub
 * that runs the Git of Apple's developer tools, and without them opens the Command Line Tools installer instead; so it
 * counts only when the Command Line Tools are installed, or an Xcode `xcode-select` points at carries Git.
 */
export function gitInstalled(path: string, platform: NodeJS.Platform = process.platform, probe: GitProbe = machine): boolean {
  if (!probe.runnable(path)) return false;
  if (platform !== "darwin" || path !== "/usr/bin/git") return true;
  if (probe.exists("/Library/Developer/CommandLineTools/usr/bin/git")) return true;
  const developerDir = probe.developerDir();
  return developerDir !== undefined && probe.exists(posix.join(developerDir, "usr", "bin", "git"));
}

/** The folders Git for Windows keeps its own git.exe in: x64, ARM64 and 32-bit builds. */
const GIT_FOR_WINDOWS_BUILDS = ["mingw64", "clangarm64", "mingw32"];

/**
 * The Git program behind Git for Windows' launcher at `path`, or `path` itself. `<Git>\cmd\git.exe` and
 * `<Git>\bin\git.exe` set up the environment and run `<Git>\mingw64\bin\git.exe` (`clangarm64` on ARM64, `mingw32` on
 * 32-bit) as their own child. A timeout kills only the process Rerail started, and on Windows killing a process leaves
 * its children running, so the real Git ran on in the repository after Rerail gave up on it. The real one runs the
 * commands Rerail gives it without the launcher: it finds its own DLLs, helpers and system configuration from its folder.
 *
 * A launcher is only a `git.exe` in a `cmd` or `bin` folder, which anyone can put on PATH beside a git.exe of their own,
 * so Rerail goes behind it only into a Git for Windows install: `<Git>\cmd\git.exe` runs, and `<Git>\etc\gitconfig` or
 * the build's `share\git` is there. The git.exe it then runs must pass what every PATH entry passes (`trustedFolder`):
 * outside `node_modules`, and outside the repository Rerail runs in by its path and its real path, which a junction
 * from `<Git>\mingw64` into the repository would reach.
 */
export function gitBehindLauncher(path: string, search: GitSearch = currentSearch(), probe: GitProbe & GitPathProbe = machine): string {
  return behindLauncher(path, gitTrust(search, probe), probe);
}

function behindLauncher(path: string, trust: GitTrust, probe: GitProbe): string {
  const folder = win32.dirname(path);
  const launcherFolder = win32.basename(folder).toLowerCase();
  if (win32.basename(path).toLowerCase() !== "git.exe" || (launcherFolder !== "cmd" && launcherFolder !== "bin")) return path;
  const root = win32.dirname(folder);
  if (!probe.runnable(win32.join(root, "cmd", "git.exe"))) return path;
  const real = GIT_FOR_WINDOWS_BUILDS.find(build => probe.runnable(win32.join(root, build, "bin", "git.exe"))
    && (probe.exists(win32.join(root, "etc", "gitconfig")) || probe.exists(win32.join(root, build, "share", "git")))
    && trustedFolder(win32.join(root, build, "bin"), trust));
  return real === undefined ? path : win32.join(root, real, "bin", "git.exe");
}

/** The Git a search found, undefined when none runs, and every place it looked, in order. */
export interface FoundGit {
  path: string | undefined;
  looked: readonly string[];
}

/**
 * The first of {@link gitCandidates} that {@link gitInstalled} takes, on Windows the Git behind Git for Windows'
 * launcher ({@link gitBehindLauncher}). Candidates are tried one at a time, so the `PATH` entries after the Git found
 * are never resolved: the status line, a process of its own every few seconds, pays for the entries up to its Git only.
 */
export function findGit(search: GitSearch = currentSearch(), probe: GitProbe & GitPathProbe = machine): FoundGit {
  const trust = gitTrust(search, probe);
  const looked: string[] = [];
  for (const candidate of candidateOrder(search, trust)) {
    looked.push(candidate);
    if (gitInstalled(candidate, search.platform, probe)) {
      return { path: search.platform === "win32" ? behindLauncher(candidate, trust, probe) : candidate, looked };
    }
  }
  return { path: undefined, looked };
}

/** The system Git ({@link findGit}), probed once per process; throws {@link GitUnavailableError} when none is installed. */
export function gitExecutable(): string {
  probed ??= findGit();
  if (probed.path === undefined) throw new GitUnavailableError(probed.looked);
  return probed.path;
}

export interface GitResult {
  /** Null when Git could not be started or was killed. */
  status: number | null;
  stdout: string;
  stderr: string;
  /** Set when Git could not be started, timed out or overflowed `maxBuffer`. */
  error?: Error;
}

export interface RunGitOptions {
  maxBuffer?: number;
  timeout?: number;
}

/** A repository's own config can't run programs through Rerail's git calls: no fsmonitor, no hooks. */
const SAFE_CONFIG: readonly string[] = ["-c", "core.fsmonitor=false", "-c", `core.hooksPath=${process.platform === "win32" ? "NUL" : "/dev/null"}`];
const DEFAULT_TIMEOUT_MS = 10_000;

/**
 * The environment of every Git process Rerail starts: no optional locks, and no lazy fetch of a partial clone's missing
 * objects. Git honours `GIT_NO_LAZY_FETCH` from 2.45.0 and in the 2024-04-19 security releases 2.39.4, 2.40.2, 2.41.1,
 * 2.42.2, 2.43.4 and 2.44.1; an older Git ignores it and may fetch from the remote.
 */
function gitEnvironment(): NodeJS.ProcessEnv {
  return { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1" };
}

/**
 * `git -C <cwd> <args>` with fsmonitor and hooks disabled, in {@link gitEnvironment} (no optional locks, no lazy fetch
 * of a partial clone's missing objects), and a 10 s default timeout; throws {@link GitUnavailableError} when Git is
 * missing, never on a non-zero exit.
 */
export function runGit(cwd: string, args: readonly string[], options: RunGitOptions = {}): GitResult {
  const result = spawnSync(gitExecutable(), [...SAFE_CONFIG, "-C", cwd, ...args], {
    encoding: "utf8",
    timeout: DEFAULT_TIMEOUT_MS,
    ...options,
    env: gitEnvironment(),
  });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "", ...(result.error === undefined ? {} : { error: result.error }) };
}

/** Trimmed stdout of a successful `git -C <cwd> <args>`, or undefined when it fails. */
export function gitText(cwd: string, args: readonly string[]): string | undefined {
  const result = runGit(cwd, args);
  return result.status === 0 ? result.stdout.trim() : undefined;
}

/** Trimmed stdout of a successful Git command run under `runGit`'s safe configuration without blocking; else undefined. */
async function gitOutput(cwd: string, args: readonly string[]): Promise<string | undefined> {
  let executable: string;
  try {
    executable = gitExecutable();
  } catch {
    return undefined;
  }
  const { promise, resolve: settle } = Promise.withResolvers<string | undefined>();
  execFile(
    executable,
    [...SAFE_CONFIG, "-C", cwd, ...args],
    { encoding: "utf8", timeout: DEFAULT_TIMEOUT_MS, env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1", GIT_TERMINAL_PROMPT: "0" } },
    (error, stdout) => settle(error === null ? stdout.trim() : undefined),
  );
  return await promise;
}

/** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
export async function gitHead(cwd: string): Promise<string | undefined> {
  return await gitOutput(cwd, ["rev-parse", "--verify", "--quiet", "HEAD"]);
}

/**
 * The exclude files of the Git checkout at `cwd` that no diff shows, absolute: its `info/exclude` (the common Git
 * folder's, which every worktree shares) and the file `core.excludesFile` names, else Git's default, `git/ignore` in
 * `XDG_CONFIG_HOME` or `~/.config`. Empty outside a Git checkout, or without Git.
 */
export async function gitExcludeFiles(cwd: string): Promise<string[]> {
  // Git prints the path relative to `cwd` unless it is elsewhere; `--path-format` would need Git 2.31.
  const exclude = await gitOutput(cwd, ["rev-parse", "--git-path", "info/exclude"]);
  if (exclude === undefined || exclude === "") return [];
  const configured = await gitOutput(cwd, ["config", "--path", "--get", "core.excludesFile"]);
  return [resolve(cwd, exclude), resolve(cwd, configured || defaultExcludesFile())];
}

/**
 * The home folder Git reads `~/.gitconfig` and `~/.config` from: `HOME` when set. Without it Git for Windows takes
 * `HOMEDRIVE` and `HOMEPATH` when that folder exists, which on a machine with a home share is not the profile folder
 * `homedir()` returns, and then the profile folder (`USERPROFILE`). Elsewhere Git takes the account's home.
 */
export function gitHome(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  isFolder: (path: string) => boolean = folderExists,
): string {
  if (env.HOME !== undefined && env.HOME !== "") return env.HOME;
  if (platform === "win32" && env.HOMEDRIVE && env.HOMEPATH) {
    const share = win32.join(env.HOMEDRIVE, env.HOMEPATH);
    if (isFolder(share)) return share;
  }
  return homedir();
}

function folderExists(path: string): boolean {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

/**
 * The file Git reads as `core.excludesFile` when that names none: `git/ignore` in `XDG_CONFIG_HOME`, else in `.config`
 * in Git's home ({@link gitHome}).
 */
function defaultExcludesFile(env: NodeJS.ProcessEnv = process.env, platform: NodeJS.Platform = process.platform): string {
  const xdg = env.XDG_CONFIG_HOME;
  return join(xdg === undefined || xdg === "" ? join(gitHome(env, platform), ".config") : xdg, "git", "ignore");
}

/**
 * The configuration files that can name the excludes file of the Git checkout at `cwd`, absolute: its repository's
 * `config` and its worktree's `config.worktree`, and the global file, `GIT_CONFIG_GLOBAL` when set, else `.gitconfig`
 * in Git's home ({@link gitHome}, from `env` on `platform`) and `git/config` in `XDG_CONFIG_HOME` or that home's
 * `.config`. The system file, which only an administrator writes, and files `include.path` pulls in are left out. Empty
 * outside a Git checkout, or without Git.
 */
export async function gitConfigFiles(cwd: string, env: NodeJS.ProcessEnv = process.env, platform: NodeJS.Platform = process.platform): Promise<string[]> {
  const local = await gitOutput(cwd, ["rev-parse", "--git-path", "config", "--git-path", "config.worktree"]);
  if (local === undefined || local === "") return [];
  const global = env.GIT_CONFIG_GLOBAL;
  const globals = global === undefined || global === ""
    ? [join(gitHome(env, platform), ".gitconfig"), join(dirname(defaultExcludesFile(env, platform)), "config")]
    : [global];
  return [...local.split("\n").map(path => resolve(cwd, path)), ...globals.map(path => resolve(cwd, path))];
}

/** How far back {@link headMovedOnlyByCommits} looks: a window longer than this is no agent's one command. */
const MAX_REFLOG_ENTRIES = 256;
/** The reflog subject of `git commit` in all its forms (`commit (amend): …`, `commit (merge): …`). */
const COMMIT_SUBJECT = /^commit(?: \([^)]*\))?: /u;

/**
 * Whether everything that moved `HEAD` in `cwd` since it named `from` was a commit, which leaves the working tree as it
 * found it, by `HEAD`'s reflog. Anything else there (a checkout, pull, merge, rebase, reset, cherry-pick) wrote files, and
 * so did a move the reflog cannot show (`from` is not in it). With no `from`, `HEAD`'s newest move is all there is to go by.
 */
export async function headMovedOnlyByCommits(cwd: string, from: string | undefined): Promise<boolean> {
  const reflog = await gitOutput(cwd, ["reflog", "show", "-n", String(MAX_REFLOG_ENTRIES), "--format=%H %gs", "HEAD"]);
  if (reflog === undefined) return false;
  const entries = reflog.split("\n");
  const moves = from === undefined ? 1 : entries.findIndex(entry => entry.startsWith(`${from} `));
  return moves > 0 && entries.slice(0, moves).every(entry => COMMIT_SUBJECT.test(entry.slice(entry.indexOf(" ") + 1)));
}

/** The distinct commits `cwd`'s branches and remote-tracking branches point at; empty without Git or outside a repository. */
export async function gitBranchTips(cwd: string): Promise<string[]> {
  const tips = await gitOutput(cwd, ["for-each-ref", "--format=%(objectname)", "refs/heads", "refs/remotes"]);
  return tips === undefined || tips === "" ? [] : [...new Set(tips.split("\n"))];
}

/** The commits reachable from `to` but not from `from` (`git rev-list from..to`); empty when Git cannot say. */
export async function gitCommitsBetween(cwd: string, from: string, to: string): Promise<string[]> {
  const commits = await gitOutput(cwd, ["rev-list", `${from}..${to}`]);
  return commits === undefined || commits === "" ? [] : commits.split("\n");
}

/** A Git boolean that is true. */
const GIT_TRUE = /^(?:true|yes|on|1)$/iu;

/** The line-ending settings Git reads for one path: its `text` and `eol` attributes, and `core.autocrlf` and `core.eol`. */
interface LineEndingSettings {
  /** `set`, `unset`, `unspecified` or a value such as `auto`, as `git check-attr` prints it. */
  readonly text: string;
  readonly eol: string;
  readonly autocrlf: string | undefined;
  readonly coreEol: string | undefined;
}

/**
 * Whether a checkout writes a text file's line breaks as CRLF under `settings`, as Git decides it: `-text` never; an
 * `eol` attribute as it says; a file with no `text` attribute only under `core.autocrlf=true`; a `text` or `text=auto`
 * file under `core.autocrlf=true`, never under `input`, else as `core.eol` says, whose default (`native`) is CRLF on
 * Windows alone.
 */
function writesCrlf(settings: LineEndingSettings, platform: NodeJS.Platform): boolean {
  const { text, eol, autocrlf, coreEol } = settings;
  if (text === "unset") return false;
  if (eol === "crlf" || eol === "lf") return eol === "crlf";
  const autoCrlf = autocrlf !== undefined && GIT_TRUE.test(autocrlf);
  if (text === "unspecified" || autoCrlf) return autoCrlf;
  if (autocrlf === "input") return false;
  return coreEol === "crlf" || ((coreEol === undefined || coreEol === "native") && platform === "win32");
}

/**
 * The `text` and `eol` attributes of each of `paths` (relative to `cwd`), by one `git check-attr --stdin`; empty without
 * Git, outside a repository or past the timeout.
 */
async function lineEndingAttributes(cwd: string, paths: readonly string[]): Promise<Map<string, { text: string; eol: string }>> {
  const attributes = new Map<string, { text: string; eol: string }>();
  let executable: string;
  try {
    executable = gitExecutable();
  } catch {
    return attributes;
  }
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, "check-attr", "-z", "--stdin", "text", "eol"], {
    env: { ...gitEnvironment(), GIT_TERMINAL_PROMPT: "0" },
    stdio: ["pipe", "pipe", "ignore"],
    timeout: DEFAULT_TIMEOUT_MS,
  });
  const chunks: Buffer[] = [];
  child.stdout.on("data", (chunk: Buffer) => chunks.push(chunk));
  settleWhenDone(child, settle);
  child.stdin.on("error", () => undefined);
  child.stdin.end(paths.map(path => `${path}\0`).join(""));
  await promise;
  // `<path> NUL <attribute> NUL <value> NUL`, once per path and attribute.
  const fields = Buffer.concat(chunks).toString("utf8").split("\0");
  for (let at = 0; at + 2 < fields.length; at += 3) {
    const [path, name, value] = [fields[at]!, fields[at + 1]!, fields[at + 2]!];
    const entry = attributes.get(path) ?? { text: "unspecified", eol: "unspecified" };
    if (name === "text") entry.text = value;
    else if (name === "eol") entry.eol = value;
    attributes.set(path, entry);
  }
  return attributes;
}

/**
 * The paths among `paths` (relative to `cwd`, `/`-separated) that a checkout of `cwd` writes with CRLF line breaks
 * ({@link writesCrlf}), read from the repository's attributes and configuration. Empty without Git, outside a
 * repository or past the timeout, so a file with CRLF line breaks is then never taken for Git's writing.
 */
export async function gitCrlfPaths(cwd: string, paths: readonly string[], platform: NodeJS.Platform = process.platform): Promise<Set<string>> {
  const asked = paths.filter(path => !path.includes("\0"));
  const crlf = new Set<string>();
  if (asked.length === 0) return crlf;
  const [autocrlf, coreEol, attributes] = await Promise.all([
    gitOutput(cwd, ["config", "--get", "core.autocrlf"]),
    gitOutput(cwd, ["config", "--get", "core.eol"]),
    lineEndingAttributes(cwd, asked),
  ]);
  for (const path of asked) {
    const attribute = attributes.get(path);
    if (attribute !== undefined && writesCrlf({ ...attribute, autocrlf: autocrlf?.toLowerCase(), coreEol: coreEol?.toLowerCase() }, platform)) crlf.add(path);
  }
  return crlf;
}

/**
 * The object id of each of `revisions` (as for {@link readGitBlobs}) that names a blob, by one `git cat-file
 * --batch-check`: no blob is read. A revision Git does not resolve to a blob is absent, and so is every revision
 * without Git, outside a repository, or past the timeout; one containing a newline cannot be asked for and is skipped.
 */
export async function readGitBlobIds(cwd: string, revisions: readonly string[]): Promise<Map<string, string>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const ids = new Map<string, string>();
  if (asked.length === 0) return ids;
  let executable: string;
  try {
    executable = gitExecutable();
  } catch {
    return ids;
  }
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, "cat-file", "--batch-check=%(objectname) %(objecttype)"], {
    env: { ...gitEnvironment(), GIT_TERMINAL_PROMPT: "0" },
    stdio: ["pipe", "pipe", "ignore"],
    timeout: DEFAULT_TIMEOUT_MS,
  });
  const chunks: Buffer[] = [];
  child.stdout.on("data", (chunk: Buffer) => chunks.push(chunk));
  settleWhenDone(child, settle);
  child.stdin.on("error", () => undefined);
  child.stdin.end(asked.map(revision => `${revision}\n`).join(""));
  await promise;
  // One line per revision, in order: `<oid> <type>`, or `<revision> missing` (or `ambiguous`), which repeats the
  // revision and so may hold any text, spaces included; only a whole line of an object id and `blob` names one.
  const lines = Buffer.concat(chunks).toString("utf8").split("\n");
  asked.forEach((revision, index) => {
    const id = /^([0-9a-f]{40}|[0-9a-f]{64}) blob$/u.exec(lines[index] ?? "")?.[1];
    if (id !== undefined) ids.set(revision, id);
  });
  return ids;
}

/** How long a batch read waits for Git's output once the timeout has killed the process it started. */
const EXIT_DRAIN_MS = 500;

/**
 * Calls `settle` once `child` fails to start, its output closes, or {@link EXIT_DRAIN_MS} after the timeout kills it,
 * whichever comes first. On Windows the process Rerail starts can still be a launcher that runs the real Git as its own
 * child (a scoop shim, or Git for Windows' `cmd\git.exe` without its own git.exe beside it; {@link gitBehindLauncher}).
 * A timeout kills only the launcher, and the real Git keeps the stdout pipe open until it ends on its own, so waiting
 * for `close` alone would hold the answer past the timeout. A Git that exits by itself is read to the end of its output.
 */
function settleWhenDone(child: ChildProcess, settle: () => void): void {
  child.on("error", () => settle());
  child.on("close", () => settle());
  child.on("exit", (_code, signal) => {
    if (signal === null) return;
    setTimeout(() => {
      child.stdout?.destroy();
      settle();
    }, EXIT_DRAIN_MS).unref();
  });
}

/** One object body of `git cat-file --batch` output being read: kept, or skipped as it streams past. */
interface BatchBody {
  readonly revision: string;
  readonly size: number;
  readonly kept: Buffer[] | undefined;
  /** Bytes of the body and its trailing newline still to come. */
  remaining: number;
}

/**
 * The contents of `revisions` (revision expressions such as `:./src/a.ts` or `HEAD:./src/a.ts`, relative to `cwd`),
 * read by one `git cat-file --batch` under `runGit`'s safe configuration, parsed as the output streams. A revision Git
 * does not resolve to a blob, or whose blob is larger than `maxBlobBytes`, is absent; so is every revision without Git,
 * outside a repository, or past the timeout (blobs read before it are kept). A revision containing a newline cannot be
 * asked for and is skipped. Git never fetches a partial clone's missing blob or prompts for credentials here: a blob
 * the clone does not hold is absent.
 */
export async function readGitBlobs(
  cwd: string,
  revisions: readonly string[],
  options: { maxBlobBytes: number; timeout?: number },
): Promise<Map<string, Buffer>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const blobs = new Map<string, Buffer>();
  if (asked.length === 0) return blobs;
  let executable: string;
  try {
    executable = gitExecutable();
  } catch {
    return blobs;
  }
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, "cat-file", "--batch"], {
    env: { ...gitEnvironment(), GIT_TERMINAL_PROMPT: "0" },
    stdio: ["pipe", "pipe", "ignore"],
    timeout: options.timeout ?? DEFAULT_TIMEOUT_MS,
  });
  let next = 0;
  let header: Buffer[] = [];
  let body: BatchBody | undefined;
  child.stdout.on("data", (chunk: Buffer) => {
    let offset = 0;
    while (offset < chunk.length) {
      if (body !== undefined) {
        const take = Math.min(body.remaining, chunk.length - offset);
        body.kept?.push(chunk.subarray(offset, offset + take));
        body.remaining -= take;
        offset += take;
        if (body.remaining === 0) {
          if (body.kept !== undefined) blobs.set(body.revision, Buffer.concat(body.kept).subarray(0, body.size));
          body = undefined;
        }
        continue;
      }
      const end = chunk.indexOf(0x0a, offset);
      if (end === -1) {
        header.push(chunk.subarray(offset));
        break;
      }
      header.push(chunk.subarray(offset, end));
      offset = end + 1;
      // `<oid> <type> <size>` then the object and a newline; anything else (`<revision> missing`) has no body.
      const fields = Buffer.concat(header).toString("utf8").split(" ");
      header = [];
      const revision = asked[next++];
      if (revision === undefined || fields.length !== 3 || !/^[0-9a-f]+$/u.test(fields[0]!)) continue;
      const size = Number(fields[2]);
      const kept = fields[1] === "blob" && size <= options.maxBlobBytes ? [] : undefined;
      body = { revision, size, kept, remaining: size + 1 };
    }
  });
  settleWhenDone(child, settle);
  // Git exits before reading every line when it cannot start in `cwd`; what it answered before that is kept.
  child.stdin.on("error", () => undefined);
  child.stdin.end(asked.map(revision => `${revision}\n`).join(""));
  await promise;
  return blobs;
}
