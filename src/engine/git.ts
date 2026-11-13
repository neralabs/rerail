import type { ChildProcess } from "node:child_process";
import { accessSync, constants, existsSync, realpathSync, statSync } from "node:fs";
import { stat } from "node:fs/promises";
import { homedir, userInfo } from "node:os";
import { dirname, join, posix, resolve, win32 } from "node:path";
import { endProcessTree, execFile, spawn, spawnSync } from "./child-process.js";
import { fileStamp } from "./file-stamp.js";

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

/**
 * Git Bash's `bash.exe` in the Git for Windows install Rerail runs Git from ({@link gitExecutable}), or undefined
 * without Git or a Git Bash beside it. `<Git>\bin\bash.exe` sits a folder or two above its git.exe: `cmd\`, `bin\` or
 * `mingw64\bin\`.
 */
export function gitBash(exists: (path: string) => boolean = existsSync): string | undefined {
  let folder: string;
  try {
    folder = win32.dirname(gitExecutable());
  } catch {
    return undefined;
  }
  for (let level = 0; level < 3; level += 1, folder = win32.dirname(folder)) {
    const bash = win32.join(folder, "bin", "bash.exe");
    if (exists(bash)) return bash;
  }
  return undefined;
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

/**
 * How a Git read that does not block can be stopped. Every such read settles only once the Git it started has exited:
 * on Windows a running process holds the folder it runs in, so a read that answered before its Git exited would leave
 * the checkout impossible to remove or rename for a moment.
 */
export interface GitReadOptions {
  /**
   * Once aborted, the read ends its Git with every process that Git started, as its timeout does ({@link settleWhenDone}),
   * or starts none, and answers as it does past its timeout.
   */
  signal?: AbortSignal | undefined;
}

/**
 * The Git reads one owner started without waiting for each, such as a session's source walks: {@link stop} kills those
 * still running and settles once each has exited, so the owner can end with no Git left running in its checkouts.
 */
export class GitReads {
  readonly #stopping = new AbortController();
  readonly #running = new Set<Promise<unknown>>();

  /** Runs `read` with the signal {@link stop} aborts; once stopped, a read starts no Git and answers as without Git. */
  run<T>(read: (signal: AbortSignal) => Promise<T>): Promise<T> {
    const running = read(this.#stopping.signal);
    this.#running.add(running);
    const forget = (): void => {
      this.#running.delete(running);
    };
    running.then(forget, forget);
    return running;
  }

  /** Kills every read still running and settles once the Git each started has exited. */
  async stop(): Promise<void> {
    this.#stopping.abort();
    await Promise.allSettled([...this.#running]);
  }
}

/** The Git to start a read with; undefined without Git, or once `signal` is aborted. */
function readerGit(signal: AbortSignal | undefined): string | undefined {
  if (signal?.aborted === true) return undefined;
  try {
    return gitExecutable();
  } catch {
    return undefined;
  }
}

/** Resolves once `child` has exited, or at once when it never started. */
async function exited(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  child.once("exit", () => settle());
  child.once("error", () => {
    if (child.pid === undefined) settle();
  });
  await promise;
}

/**
 * Trimmed stdout of a successful Git command run under `runGit`'s safe configuration without blocking; else undefined.
 * Its timeout and `signal` end it as they end a batch read ({@link settleWhenDone}); it settles once that Git has exited
 * ({@link GitReadOptions}).
 */
async function gitOutput(cwd: string, args: readonly string[], signal: AbortSignal | undefined): Promise<string | undefined> {
  return (await gitStdout(cwd, args, signal))?.trim();
}

/** Stdout, as Git wrote it, of a successful Git command run as `gitOutput` runs one; undefined past `maxBuffer` bytes. */
async function gitStdout(cwd: string, args: readonly string[], signal: AbortSignal | undefined, maxBuffer = 1024 * 1024): Promise<string | undefined> {
  const read = await readGit(cwd, args, signal, maxBuffer);
  return read.ended === "printed" ? read.stdout : undefined;
}

/**
 * How a Git read ended: with what Git printed, failing (no Git, an error, past `maxBuffer`), or ended past its time limit;
 * one its `signal` stopped reads the same ({@link GitReadOptions}).
 */
type GitRead = { readonly ended: "printed"; readonly stdout: string } | { readonly ended: "failed" | "timed-out" };

/** Runs a Git command as `gitOutput` does, and says how it ended. */
async function readGit(cwd: string, args: readonly string[], signal: AbortSignal | undefined, maxBuffer: number): Promise<GitRead> {
  const executable = readerGit(signal);
  if (executable === undefined) return { ended: signal?.aborted === true ? "timed-out" : "failed" };
  const { promise, resolve: settle } = Promise.withResolvers<GitRead>();
  const child = execFile(
    executable,
    [...SAFE_CONFIG, "-C", cwd, ...args],
    { encoding: "utf8", maxBuffer, env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1", GIT_TERMINAL_PROMPT: "0" } },
    (error, stdout) => {
      if (error === null && signal?.aborted !== true) settle({ ended: "printed", stdout });
    },
  );
  releaseStderrOnceDone(child);
  // execFile's own `close` listener comes first and answers with the output. This one answers every other end, saying
  // whether the time limit or `signal` ended the read, and a Git that was ended and whose output stays open.
  settleWhenDone(child, ended => settle({ ended: ended ? "timed-out" : "failed" }), { timeoutMs: DEFAULT_TIMEOUT_MS, signal });
  const [read] = await Promise.all([promise, exited(child)]);
  return read;
}

/** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
export async function gitHead(cwd: string, options: GitReadOptions = {}): Promise<string | undefined> {
  return await gitOutput(cwd, ["rev-parse", "--verify", "--quiet", "HEAD"], options.signal);
}

/**
 * The exclude files of the Git checkout at `cwd` that no diff shows, absolute: its `info/exclude` (the common Git
 * folder's, which every worktree shares) and the file `core.excludesFile` names, else Git's default, `git/ignore` in
 * `XDG_CONFIG_HOME` or `~/.config`. Empty outside a Git checkout, or without Git.
 */
export async function gitExcludeFiles(cwd: string, options: GitReadOptions = {}): Promise<string[]> {
  // Git prints the path relative to `cwd` unless it is elsewhere; `--path-format` would need Git 2.31.
  const exclude = await gitOutput(cwd, ["rev-parse", "--git-path", "info/exclude"], options.signal);
  if (exclude === undefined || exclude === "") return [];
  const configured = await gitOutput(cwd, ["config", "--path", "--get", "core.excludesFile"], options.signal);
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
 * in Git's home ({@link gitHome}, from `env` on `platform`, this process's by default) and `git/config` in
 * `XDG_CONFIG_HOME` or that home's `.config`. The system file, which only an administrator writes, and files
 * `include.path` pulls in are left out. Empty outside a Git checkout, or without Git.
 */
export async function gitConfigFiles(
  cwd: string,
  options: GitReadOptions & { env?: NodeJS.ProcessEnv; platform?: NodeJS.Platform } = {},
): Promise<string[]> {
  const { env = process.env, platform = process.platform } = options;
  const local = await gitOutput(cwd, ["rev-parse", "--git-path", "config", "--git-path", "config.worktree"], options.signal);
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
export async function headMovedOnlyByCommits(cwd: string, from: string | undefined, options: GitReadOptions = {}): Promise<boolean> {
  const reflog = await gitOutput(cwd, ["reflog", "show", "-n", String(MAX_REFLOG_ENTRIES), "--format=%H %gs", "HEAD"], options.signal);
  if (reflog === undefined) return false;
  const entries = reflog.split("\n");
  const moves = from === undefined ? 1 : entries.findIndex(entry => entry.startsWith(`${from} `));
  return moves > 0 && entries.slice(0, moves).every(entry => COMMIT_SUBJECT.test(entry.slice(entry.indexOf(" ") + 1)));
}

/** The distinct commits `cwd`'s branches and remote-tracking branches point at; empty without Git or outside a repository. */
export async function gitBranchTips(cwd: string, options: GitReadOptions = {}): Promise<string[]> {
  const tips = await gitOutput(cwd, ["for-each-ref", "--format=%(objectname)", "refs/heads", "refs/remotes"], options.signal);
  return tips === undefined || tips === "" ? [] : [...new Set(tips.split("\n"))];
}

/** The commits reachable from `to` but not from `from` (`git rev-list from..to`); empty when Git cannot say. */
export async function gitCommitsBetween(cwd: string, from: string, to: string, options: GitReadOptions = {}): Promise<string[]> {
  const commits = await gitOutput(cwd, ["rev-list", `${from}..${to}`], options.signal);
  return commits === undefined || commits === "" ? [] : commits.split("\n");
}

/** A Git boolean that is true. */
const GIT_TRUE = /^(?:true|yes|on|1)$/iu;

/**
 * The rule by which a checkout writes a file's LF line breaks as CRLF, as Git's convert.c applies it (line-endings.ts
 * writes the bytes). `auto` (`core.autocrlf=true`, `text=auto`) writes a blob that already holds a CR, or that Git takes
 * for binary data, as it is. `text` (a `text` attribute, or `eol=crlf` without `text=auto`) writes every LF that follows
 * no CR as CRLF, whatever else the blob holds.
 */
export type CrlfRule = "auto" | "text";

/** The line-ending attributes Git reads for one path, each as `git check-attr` prints it: `set`, `unset`, `unspecified` or a value. */
interface LineEndingAttributes {
  readonly text: string;
  /** The older name of `text`, which Git reads where `text` declares nothing. */
  readonly crlf: string;
  readonly eol: string;
}

/** `core.autocrlf` and `core.eol`, lowercased; undefined where unset. */
interface LineEndingConfig {
  readonly autocrlf: string | undefined;
  readonly coreEol: string | undefined;
}

/** What a `text` or `crlf` attribute declares a file: text, binary, text stored and written with LF, or text if Git guesses so. */
function declaredText(value: string): "text" | "binary" | "input" | "auto" | undefined {
  if (value === "set") return "text";
  if (value === "unset") return "binary";
  return value === "input" || value === "auto" ? value : undefined;
}

/**
 * Whether a text file with no `eol` attribute gets CRLF: under `core.autocrlf=true`, never under `input`, else as
 * `core.eol` says, whose default (`native`) is CRLF on Windows alone.
 */
function textGetsCrlf({ autocrlf, coreEol }: LineEndingConfig, platform: NodeJS.Platform): boolean {
  if (autocrlf === "input") return false;
  if (autocrlf !== undefined && GIT_TRUE.test(autocrlf)) return true;
  return coreEol === "crlf" || (coreEol !== "lf" && platform === "win32");
}

/**
 * The rule by which a checkout writes a file's line breaks as CRLF, or undefined where it writes them as stored, as Git's
 * `convert_attrs` and `output_eol` decide it. The `text` attribute, else `crlf`, declares the file; a binary one is
 * never converted. `eol=crlf` then makes a `text=auto` file `auto` and any other `text`, and `eol=lf` writes LF. With no
 * `eol`, an `input` file gets LF, a `text` or `auto` file gets CRLF where a text file does ({@link textGetsCrlf}), and a
 * file that declares nothing is `auto` under `core.autocrlf=true` alone.
 */
function crlfRule(attributes: LineEndingAttributes, config: LineEndingConfig, platform: NodeJS.Platform): CrlfRule | undefined {
  const declared = declaredText(attributes.text) ?? declaredText(attributes.crlf);
  if (declared === "binary") return undefined;
  if (attributes.eol === "crlf") return declared === "auto" ? "auto" : "text";
  if (attributes.eol === "lf" || declared === "input") return undefined;
  if (declared === undefined) return config.autocrlf !== undefined && GIT_TRUE.test(config.autocrlf) ? "auto" : undefined;
  return textGetsCrlf(config, platform) ? declared : undefined;
}

/**
 * The `text`, `crlf` and `eol` attributes of each of `paths` (relative to `cwd`), by one `git check-attr --stdin`; empty
 * without Git, outside a repository or past the timeout.
 */
async function lineEndingAttributes(cwd: string, paths: readonly string[], signal: AbortSignal | undefined): Promise<Map<string, LineEndingAttributes>> {
  const attributes = new Map<string, { text: string; crlf: string; eol: string }>();
  const chunks: Buffer[] = [];
  await streamGit(cwd, ["check-attr", "-z", "--stdin", "text", "crlf", "eol"], {
    input: paths.map(path => `${path}\0`).join(""), onData: chunk => chunks.push(chunk), signal,
  });
  // `<path> NUL <attribute> NUL <value> NUL`, once per path and attribute.
  const fields = Buffer.concat(chunks).toString("utf8").split("\0");
  for (let at = 0; at + 2 < fields.length; at += 3) {
    const [path, name, value] = [fields[at]!, fields[at + 1]!, fields[at + 2]!];
    const entry = attributes.get(path) ?? { text: "unspecified", crlf: "unspecified", eol: "unspecified" };
    if (name === "text" || name === "crlf" || name === "eol") entry[name] = value;
    attributes.set(path, entry);
  }
  return attributes;
}

/**
 * The paths among `paths` (relative to `cwd`, `/`-separated) that a checkout of `cwd` writes with CRLF line breaks, each
 * with the rule it writes them by ({@link crlfRule}), read from the repository's attributes and configuration. Empty
 * without Git, outside a repository or past the timeout, so a file with CRLF line breaks is then never taken for Git's
 * writing.
 */
export async function gitCrlfPaths(
  cwd: string,
  paths: readonly string[],
  options: GitReadOptions & { platform?: NodeJS.Platform } = {},
): Promise<Map<string, CrlfRule>> {
  const { platform = process.platform, signal } = options;
  const asked = paths.filter(path => !path.includes("\0"));
  const rules = new Map<string, CrlfRule>();
  if (asked.length === 0) return rules;
  const [autocrlf, coreEol, attributes] = await Promise.all([
    gitOutput(cwd, ["config", "--get", "core.autocrlf"], signal),
    gitOutput(cwd, ["config", "--get", "core.eol"], signal),
    lineEndingAttributes(cwd, asked, signal),
  ]);
  const config = { autocrlf: autocrlf?.toLowerCase(), coreEol: coreEol?.toLowerCase() };
  for (const path of asked) {
    const attribute = attributes.get(path);
    const rule = attribute === undefined ? undefined : crlfRule(attribute, config, platform);
    if (rule !== undefined) rules.set(path, rule);
  }
  return rules;
}

/** The most bytes of a listing `gitModifiedOrUntrackedPaths` and `unmergedEntries` read: past it, the first lists none and the second cannot say. */
const MAX_LISTED_PATH_BYTES = 64 * 1024 * 1024;

/**
 * The files of the checkout at `cwd` whose text its index may not hold: modified since staged (or deleted), and untracked
 * but not ignored; relative to `cwd`. Empty without Git, outside a repository, or past `MAX_LISTED_PATH_BYTES` of paths.
 * Undefined when Git ran past its time limit, or `options.signal` stopped it: it did not say which files changed, and
 * none is not its answer.
 */
export async function gitModifiedOrUntrackedPaths(cwd: string, options: GitReadOptions = {}): Promise<string[] | undefined> {
  const read = await readGit(cwd, ["ls-files", "-z", "--modified", "--others", "--exclude-standard"], options.signal, MAX_LISTED_PATH_BYTES);
  if (read.ended === "timed-out") return undefined;
  return read.ended === "printed" ? read.stdout.split("\0").filter(path => path !== "") : [];
}

/**
 * The object id of each of `revisions` (as for {@link readGitBlobs}) that names a blob, by one `git cat-file
 * --batch-check`: no blob is read. A revision Git does not resolve to a blob is absent, and so is every revision
 * without Git, outside a repository, or past the timeout; one containing a newline cannot be asked for and is skipped.
 */
export async function readGitBlobIds(cwd: string, revisions: readonly string[], options: GitReadOptions = {}): Promise<Map<string, string>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const ids = new Map<string, string>();
  if (asked.length === 0) return ids;
  const chunks: Buffer[] = [];
  await streamGit(cwd, ["cat-file", "--batch-check=%(objectname) %(objecttype)"], {
    input: asked.map(revision => `${revision}\n`).join(""), onData: chunk => chunks.push(chunk), signal: options.signal,
  });
  // One line per revision, in order: `<oid> <type>`, or `<revision> missing` (or `ambiguous`), which repeats the
  // revision and so may hold any text, spaces included; only a whole line of an object id and `blob` names one.
  const lines = Buffer.concat(chunks).toString("utf8").split("\n");
  asked.forEach((revision, index) => {
    const id = /^([0-9a-f]{40}|[0-9a-f]{64}) blob$/u.exec(lines[index] ?? "")?.[1];
    if (id !== undefined) ids.set(revision, id);
  });
  return ids;
}

/** How long a read waits for Git's output to close once the Git it ended has exited. */
const EXIT_DRAIN_MS = 500;

/**
 * Stops reading `child`'s stderr, which no answer uses, once Git has exited and its stdout has ended: a process Git
 * started that outlives it (a repository's clean filter can leave one) may hold stderr open, and the read would wait for
 * it until its time limit, then answer that Git did not finish. Git's whole output is read by then, so it answers at once.
 */
function releaseStderrOnceDone(child: ChildProcess): void {
  let waiting = 2;
  const done = (): void => {
    waiting -= 1;
    if (waiting === 0) child.stderr?.destroy();
  };
  child.once("exit", done);
  child.stdout?.once("end", done);
}

/**
 * How often a Git read looks at its time limit. Each look counts this long however late it runs, so the count is the
 * time the event loop ran, not the time it stalled.
 */
const TIME_LIMIT_TICK_MS = 1_000;

/** What a Git read has had so far: how many of Git's exit and its output's end. */
interface ReadProgress {
  ends: number;
}

/**
 * A Git read's time limit, asked at each look ({@link lookPastTimeLimit}) with what the read has had so far: whether it
 * may go on. It may while its looks count less than `limitMs` and it has run less than `limitMs` on the clock, as
 * execFile's `timeout` allowed; past that, only once more after Git exited or its output ended since the last look,
 * since the read closes then. A Git still printing at that look ends there, as execFile's `timeout` ended it, and the
 * read answers that it ran late: a walk asks such a listing once more off its path (`CodeRuleBridge`).
 */
function timeLimit(limitMs: number, tickMs: number): (progress: Readonly<ReadProgress>) => boolean {
  const startedAt = performance.now();
  let counted = 0;
  let endsBefore = 0;
  return progress => {
    counted += tickMs;
    const ending = progress.ends > endsBefore;
    endsBefore = progress.ends;
    return ending || (counted < limitMs && performance.now() - startedAt < limitMs);
  };
}

/** What `child`'s read has had so far ({@link ReadProgress}), kept up to date. */
function readProgress(child: ChildProcess): Readonly<ReadProgress> {
  const progress: ReadProgress = { ends: 0 };
  const ended = (): void => {
    progress.ends += 1;
  };
  child.stdout?.once("end", ended);
  child.once("exit", ended);
  return progress;
}

/**
 * Calls `end` once `child`'s read is past its time limit (`limitMs`, {@link timeLimit}), and returns what stops it. It
 * looks one loop turn after each tick: after a stall, Node runs its timers before it reads what a Git that finished
 * during the stall printed and handles its exit, and the look sees both.
 */
function lookPastTimeLimit(child: ChildProcess, limitMs: number, end: () => void): () => void {
  const tickMs = Math.min(TIME_LIMIT_TICK_MS, limitMs);
  const mayGoOn = timeLimit(limitMs, tickMs);
  const progress = readProgress(child);
  let look: NodeJS.Immediate | undefined;
  const stop = (): void => {
    clearInterval(ticks);
    clearImmediate(look);
  };
  const ticks = setInterval(() => {
    look ??= setImmediate(() => {
      look = undefined;
      if (mayGoOn(progress)) return;
      stop();
      end();
    });
  }, tickMs);
  return stop;
}

/** What ends a Git read before its Git is done: its time limit, and the signal that stops the reads it belongs to. */
interface ReadLimits {
  readonly timeoutMs: number;
  readonly signal: AbortSignal | undefined;
}

/**
 * Calls `settle` once `child` fails to start, its output closes, or {@link EXIT_DRAIN_MS} after it exits once ended,
 * whichever comes first. `timeoutMs`, or `signal` once aborted, ends the whole process tree (`endProcessTree`), never
 * the child alone: on Windows the process Rerail starts can still be a launcher that runs the real Git as its own child
 * (a scoop shim, or Git for Windows' `cmd\git.exe` without its own git.exe beside it; {@link gitBehindLauncher}), and
 * killing only the launcher left the real Git running in the repository. Node's own `signal` option kills the child
 * alone, so it is not used. The real Git holds the output too, so the output closes once the whole tree has exited;
 * should it stay open anyway, the answer does not wait for it. A Git that exits by itself is read to the end of its
 * output; one a signal from elsewhere ended is waited for as one this ended. `timeoutMs` counts event-loop time, and a
 * read past it on the clock goes on only once more after Git exited or its output ended ({@link timeLimit}). `settle` is
 * told whether `timeoutMs` or `signal` ended the read.
 */
function settleWhenDone(child: ChildProcess, settle: (ended: boolean) => void, { timeoutMs, signal }: ReadLimits): void {
  let ended = false;
  const forget = (): void => {
    stopLooking();
    signal?.removeEventListener("abort", end);
  };
  const drain = (): void => {
    forget();
    setTimeout(() => {
      child.stdout?.destroy();
      settle(ended);
    }, EXIT_DRAIN_MS).unref();
  };
  const end = (): void => {
    stopLooking();
    ended = true;
    // A child that has exited has no tree left to reach, and its pid may name another process by now.
    if (child.exitCode === null && child.signalCode === null) endProcessTree(child);
    else drain();
  };
  const stopLooking = lookPastTimeLimit(child, timeoutMs, end);
  signal?.addEventListener("abort", end, { once: true });
  const done = (): void => {
    forget();
    settle(ended);
  };
  child.on("error", done);
  child.on("close", done);
  child.on("exit", (_code, killedBy) => {
    if (ended || killedBy !== null) drain();
  });
}

/** What {@link streamGit} writes to Git and does with what Git prints. */
interface GitStream extends GitReadOptions {
  readonly input: string;
  readonly onData: (chunk: Buffer) => void;
  /** `DEFAULT_TIMEOUT_MS` by default. */
  readonly timeout?: number;
}

/**
 * Runs `git -C <cwd> <args>` under `runGit`'s safe configuration without blocking, writes `stream.input` to it and hands
 * each chunk it prints to `stream.onData`. Settles once Git is done ({@link settleWhenDone}) and has exited
 * ({@link GitReadOptions}); without Git, or once `stream.signal` is aborted, it starts nothing.
 */
async function streamGit(cwd: string, args: readonly string[], stream: GitStream): Promise<void> {
  const executable = readerGit(stream.signal);
  if (executable === undefined) return;
  const { promise, resolve: settle } = Promise.withResolvers<void>();
  const child = spawn(executable, [...SAFE_CONFIG, "-C", cwd, ...args], {
    env: { ...gitEnvironment(), GIT_TERMINAL_PROMPT: "0" },
    stdio: ["pipe", "pipe", "ignore"],
  });
  child.stdout.on("data", stream.onData);
  settleWhenDone(child, () => settle(), { timeoutMs: stream.timeout ?? DEFAULT_TIMEOUT_MS, signal: stream.signal });
  // Git exits before reading every line when it cannot start in `cwd`; what it answered before that is kept.
  child.stdin.on("error", () => undefined);
  child.stdin.end(stream.input);
  await Promise.all([promise, exited(child)]);
}

/**
 * The index's unmerged entries under `cwd`, by path relative to it. Each value holds the path's modes, blobs and stages,
 * so a later conflict on the same path reads differently. Undefined when Git cannot say: outside a repository, past the
 * timeout, past `MAX_LISTED_PATH_BYTES` of entries, or once `signal` is aborted.
 */
async function unmergedEntries(cwd: string, signal: AbortSignal | undefined): Promise<ReadonlyMap<string, string> | undefined> {
  const listed = await gitStdout(cwd, ["ls-files", "--unmerged", "-z"], signal, MAX_LISTED_PATH_BYTES);
  if (listed === undefined) return undefined;
  const entries = new Map<string, string>();
  for (const record of listed.split("\0")) {
    const tab = record.indexOf("\t");
    if (tab < 0) continue;
    const path = record.slice(tab + 1);
    entries.set(path, `${entries.get(path) ?? ""}${record.slice(0, tab)}\n`);
  }
  return entries;
}

/** The file `cwd`'s checkout keeps its index in (a linked worktree's is under the main `.git`); undefined when Git cannot say. */
async function gitIndexPath(cwd: string, signal: AbortSignal | undefined): Promise<string | undefined> {
  const path = await gitOutput(cwd, ["rev-parse", "--git-path", "index"], signal);
  return path === undefined ? undefined : resolve(cwd, path);
}

/**
 * Reads the index's unmerged entries under a folder ({@link unmergedEntries}), for a caller that asks every pass: it
 * starts a Git process only when the folder's index file changed since the last read, and otherwise costs one `stat`
 * (plus one Git process the first time, to find the file). Git replaces that file with a new one on every write, so a
 * write changes its inode even inside one clock tick. A folder whose file cannot be found or stamped is read every time,
 * so this can cost more, never answer with stale entries. When Git cannot list them (it failed, timed out or was
 * stopped), the answer is undefined and nothing is kept, so the next read asks Git again.
 */
export function gitUnmergedReader(): (cwd: string, options?: GitReadOptions) => Promise<ReadonlyMap<string, string> | undefined> {
  const indexes = new Map<string, string>();
  const held = new Map<string, { readonly stamp: string; readonly entries: ReadonlyMap<string, string> }>();
  /** The stamp of `cwd`'s index file, found once and remembered; undefined when it cannot be found or stamped. */
  const indexStamp = async (cwd: string, signal: AbortSignal | undefined): Promise<string | undefined> => {
    const index = indexes.get(cwd) ?? await gitIndexPath(cwd, signal);
    if (index === undefined) return undefined;
    indexes.set(cwd, index);
    try {
      return fileStamp(await stat(index));
    } catch {
      return undefined;
    }
  };
  return async (cwd, options = {}) => {
    // Stamped before the read: an index written between the two reads as changed next time.
    const stamp = await indexStamp(cwd, options.signal);
    const last = held.get(cwd);
    if (last !== undefined && last.stamp === stamp) return last.entries;
    held.delete(cwd);
    const entries = await unmergedEntries(cwd, options.signal);
    if (entries !== undefined && stamp !== undefined) held.set(cwd, { stamp, entries });
    return entries;
  };
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
  options: GitReadOptions & { maxBlobBytes: number; timeout?: number },
): Promise<Map<string, Buffer>> {
  const asked = revisions.filter(revision => !/[\r\n]/u.test(revision));
  const blobs = new Map<string, Buffer>();
  if (asked.length === 0) return blobs;
  await streamGit(cwd, ["cat-file", "--batch"], {
    input: asked.map(revision => `${revision}\n`).join(""),
    onData: batchReader(asked, options.maxBlobBytes, blobs),
    ...(options.timeout === undefined ? {} : { timeout: options.timeout }),
    signal: options.signal,
  });
  return blobs;
}

/**
 * Reads `git cat-file --batch` output for `asked`, chunk by chunk, into `blobs`: each blob of at most `maxBlobBytes`,
 * under the revision that asked for it.
 */
function batchReader(asked: readonly string[], maxBlobBytes: number, blobs: Map<string, Buffer>): (chunk: Buffer) => void {
  let next = 0;
  let header: Buffer[] = [];
  let body: BatchBody | undefined;
  return (chunk: Buffer) => {
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
      const kept = fields[1] === "blob" && size <= maxBlobBytes ? [] : undefined;
      body = { revision, size, kept, remaining: size + 1 };
    }
  };
}
