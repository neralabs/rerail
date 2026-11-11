import { execFile, spawn, spawnSync } from "node:child_process";
import { accessSync, constants, existsSync, realpathSync, statSync } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { homedir, userInfo } from "node:os";
import { dirname, join, posix, resolve, win32 } from "node:path";
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
 * Rerail runs in" is the nearest folder above the current one with a `.git`: an entry inside an outer checkout (around
 * a nested worktree or submodule), or any repository when Rerail runs from outside it (a daemon keeps the Git it found
 * where it started), is not skipped. The fixed locations come after `PATH`: the system ones, then the per-user installs
 * whose path only the person's account decides (Git for Windows installed for one user, scoop's Git, Linuxbrew's, a
 * Nix profile's), built from the account's home, never from the environment: `LOCALAPPDATA` can be set by a
 * repository as easily as PATH.
 */
export function gitCandidates(
  search: GitSearch = { platform: process.platform, home: accountHome(), path: process.env.PATH, cwd: currentFolder() },
  probe: GitPathProbe = machine,
): string[] {
  return [...new Set([...pathCandidates(search, probe), ...installLocations(search.platform, search.home)])];
}

/** The current folder, or undefined when it was deleted under the process (`process.cwd()` throws `ENOENT`). */
function currentFolder(): string | undefined {
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
    return [
      String.raw`C:\Program Files\Git\cmd\git.exe`,
      String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
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

function pathCandidates(search: GitSearch, probe: GitPathProbe): string[] {
  if (search.path === undefined) return [];
  const windows = search.platform === "win32";
  const paths = windows ? win32 : posix;
  const repository = search.cwd === undefined ? undefined : enclosingRepository(paths.resolve(search.cwd), paths, probe);
  // By the path as written and with symlinks resolved, both sides: macOS's /var is /private/var, and a link in the
  // repository can point out of it.
  const repositoryForms = repository === undefined ? [] : [repository, probe.realpath(repository)];
  return search.path
    .split(windows ? ";" : ":")
    // Windows allows a quoted entry, which cmd.exe and CreateProcess unquote.
    .map(entry => windows ? entry.replace(/^"(.*)"$/, "$1") : entry)
    .filter(entry => (windows ? /^(?:[A-Za-z]:[\\/]|\\\\)/.test(entry) : entry.startsWith("/"))
      && !entry.split(/[\\/]/).some(part => (windows ? part.toLowerCase() : part) === "node_modules")
      && ![paths.resolve(entry), probe.realpath(paths.resolve(entry))]
        .some(form => repositoryForms.some(folder => within(folder, form, paths))))
    .map(entry => paths.join(entry, windows ? "git.exe" : "git"));
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

let probed: { path: string | undefined; looked: readonly string[] } | undefined;

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

/** The system Git, probed once per process; throws {@link GitUnavailableError} when none is installed. */
export function gitExecutable(): string {
  if (probed === undefined) {
    const looked = gitCandidates();
    probed = { path: looked.find(path => gitInstalled(path)), looked };
  }
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
  return (await gitStdout(cwd, args))?.trim();
}

/** Stdout, as Git wrote it, of a successful Git command run as `gitOutput` runs one; undefined past `maxBuffer` bytes. */
async function gitStdout(cwd: string, args: readonly string[], maxBuffer = 1024 * 1024): Promise<string | undefined> {
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
    { encoding: "utf8", maxBuffer, timeout: DEFAULT_TIMEOUT_MS, env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1", GIT_TERMINAL_PROMPT: "0" } },
    (error, stdout) => settle(error === null ? stdout : undefined),
  );
  return await promise;
}

/** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
export async function gitHead(cwd: string): Promise<string | undefined> {
  return await gitOutput(cwd, ["rev-parse", "--verify", "--quiet", "HEAD"]);
}

/** How long packed-refs must stand unchanged before {@link gitHeadReader} keeps an answer that its stamp vouches for. */
const PACKED_REFS_SETTLE_MS = 2_000;

/**
 * Where a checkout's top folder keeps `HEAD` and its branches, as Git named them (`kept`), or `asked` where every read
 * asks Git: a folder below the top, a bare repository or Git folder, a reftable repository. `dotGit` is what the
 * folder's `.git` entry held before Git named them ({@link fileText}).
 */
type HeadLayout =
  | { readonly kind: "kept"; readonly dotGit: string; readonly gitDir: string; readonly commonDir: string }
  | { readonly kind: "asked"; readonly dotGit: string };

/** What {@link gitHeadReader} compares between reads, and whether a read may keep the answer that goes with it. */
interface HeadKey {
  readonly key: string;
  /** False while packed-refs, which the key holds by its stamp, is too new for the stamp to vouch for it. */
  readonly settled: boolean;
}

function errorCode(error: unknown): string {
  return error instanceof Error && "code" in error ? String(error.code) : "unknown";
}

/** A file's text, or `!` and the error that kept it unread: one part of a {@link HeadKey}. */
async function fileText(path: string): Promise<string> {
  try {
    return await readFile(path, "utf8");
  } catch (error) {
    return `!${errorCode(error)}`;
  }
}

/**
 * The stamp of packed-refs, and whether it is old enough to vouch for the file. Git writes packed-refs as a new file
 * each time, so a rewrite changes its inode; but a new file can take an inode freed by an earlier rewrite, and within
 * one tick of the file clock it then keeps every part of the stamp. A stamp taken `settleMs` after the file's last
 * change can only be repeated by a write that carries a later time.
 */
async function packedRefsStamp(path: string, settleMs: number): Promise<{ readonly stamp: string; readonly settled: boolean }> {
  try {
    const info = await stat(path);
    return { stamp: fileStamp(info), settled: Date.now() - info.ctimeMs >= settleMs };
  } catch (error) {
    return { stamp: `!${errorCode(error)}`, settled: true };
  }
}

async function present(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

/** Where the folder `cwd` keeps `HEAD`, by one Git process; undefined when Git cannot say, so the next read asks again. */
async function headLayout(cwd: string, dotGit: string): Promise<HeadLayout | undefined> {
  const printed = await gitStdout(cwd, ["rev-parse", "--show-cdup", "--git-dir", "--git-common-dir"]);
  if (printed === undefined) return undefined;
  // An empty line first when `cwd` is the top folder (a bare repository or a Git folder prints none), then the Git
  // folder and the common one, each absolute or relative to `cwd`.
  const top = /^\r?\n([^\r\n]+)\r?\n([^\r\n]+)\r?\n$/u.exec(printed);
  if (top?.[1] === undefined || top[2] === undefined) return { kind: "asked", dotGit };
  const commonDir = resolve(cwd, top[2]);
  // A reftable repository keeps its refs in tables this reader does not follow.
  if (await present(join(commonDir, "reftable"))) return { kind: "asked", dotGit };
  return { kind: "kept", dotGit, gitDir: resolve(cwd, top[1]), commonDir };
}

/**
 * The key of what Git reads `HEAD` from in a `kept` layout: `HEAD` itself, and for a branch its loose ref, or the stamp
 * of packed-refs where it has none. Undefined where Git would read a file the key leaves out: `HEAD` naming a ref other
 * than a branch (or the `refs/heads/.invalid` a reftable repository writes there), or a branch naming another ref.
 */
async function headKey(layout: Extract<HeadLayout, { kind: "kept" }>, settleMs: number): Promise<HeadKey | undefined> {
  const head = await fileText(join(layout.gitDir, "HEAD"));
  if (head.startsWith("!")) return undefined;
  // Detached: `HEAD` holds the commit itself.
  if (!head.startsWith("ref:")) return { key: head, settled: true };
  const branch = head.slice("ref:".length).trim();
  if (!branch.startsWith("refs/heads/") || branch === "refs/heads/.invalid" || branch.split("/").includes("..")) return undefined;
  const loose = await fileText(join(layout.commonDir, ...branch.split("/")));
  if (loose.startsWith("ref:")) return undefined;
  if (!loose.startsWith("!")) return { key: `${head}\0${loose}`, settled: true };
  const packed = await packedRefsStamp(join(layout.commonDir, "packed-refs"), settleMs);
  return { key: `${head}\0${loose}\0${packed.stamp}`, settled: packed.settled };
}

/**
 * Reads `HEAD` ({@link gitHead}) for a caller that asks every pass, as the walk of a repository's worktrees does: it
 * starts a Git process only when a file Git reads `HEAD` from changed since the last read, and otherwise reads two to
 * four small files. A read keeps Git's answer with what those files held before Git read them: the folder's `.git`
 * entry, `HEAD`, and the branch's loose ref, else the stamp of packed-refs. A write between the two reads as a change
 * next time. The first read of a folder with a `.git` entry, and the first after that entry changes, also asks Git where
 * the folder keeps `HEAD` (one more Git process). Where Git would read other files (a folder below the top of its
 * checkout, a branch naming another ref, a reftable repository), where packed-refs is newer than `settleMs`, and where
 * Git named no commit, every read asks Git: this can cost more, never answer differently from Git.
 */
export function gitHeadReader(options: { readonly settleMs?: number } = {}): (cwd: string) => Promise<string | undefined> {
  const settleMs = options.settleMs ?? PACKED_REFS_SETTLE_MS;
  const layouts = new Map<string, HeadLayout>();
  const held = new Map<string, { readonly key: string; readonly head: string }>();
  const layoutOf = async (cwd: string): Promise<HeadLayout | undefined> => {
    const dotGit = await fileText(join(cwd, ".git"));
    const known = layouts.get(cwd);
    if (known?.dotGit === dotGit) return known;
    held.delete(cwd);
    // A folder without a `.git` entry is no checkout's top folder (or is outside any): Git is asked every time.
    const layout: HeadLayout | undefined = dotGit === "!ENOENT" ? { kind: "asked", dotGit } : await headLayout(cwd, dotGit);
    if (layout === undefined) layouts.delete(cwd);
    else layouts.set(cwd, layout);
    return layout;
  };
  return async cwd => {
    const layout = await layoutOf(cwd);
    const key = layout?.kind === "kept" ? await headKey(layout, settleMs) : undefined;
    const last = held.get(cwd);
    if (key !== undefined && last?.key === key.key) return last.head;
    held.delete(cwd);
    const head = await gitHead(cwd);
    if (key?.settled === true && head !== undefined) held.set(cwd, { key: key.key, head });
    return head;
  };
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

/** The file Git reads as `core.excludesFile` when that names none: `git/ignore` in `XDG_CONFIG_HOME`, else in `~/.config`. */
function defaultExcludesFile(): string {
  const xdg = process.env.XDG_CONFIG_HOME;
  return join(xdg === undefined || xdg === "" ? join(process.env.HOME ?? homedir(), ".config") : xdg, "git", "ignore");
}

/**
 * The configuration files that can name the excludes file of the Git checkout at `cwd`, absolute: its repository's
 * `config` and its worktree's `config.worktree`, and the global file, `GIT_CONFIG_GLOBAL` when set, else `~/.gitconfig`
 * and `git/config` in `XDG_CONFIG_HOME` or `~/.config`. The system file, which only an administrator writes, and files
 * `include.path` pulls in are left out. Empty outside a Git checkout, or without Git.
 */
export async function gitConfigFiles(cwd: string): Promise<string[]> {
  const local = await gitOutput(cwd, ["rev-parse", "--git-path", "config", "--git-path", "config.worktree"]);
  if (local === undefined || local === "") return [];
  const global = process.env.GIT_CONFIG_GLOBAL;
  const globals = global === undefined || global === ""
    ? [join(process.env.HOME ?? homedir(), ".gitconfig"), join(dirname(defaultExcludesFile()), "config")]
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

/** The most bytes of a listing `gitModifiedOrUntrackedPaths` and `unmergedEntries` read: past it, the first lists none and the second cannot say. */
const MAX_LISTED_PATH_BYTES = 64 * 1024 * 1024;

/**
 * The files of the checkout at `cwd` whose text its index may not hold: modified since staged (or deleted), and untracked
 * but not ignored; relative to `cwd`. Empty without Git, outside a repository, past the timeout, or past
 * `MAX_LISTED_PATH_BYTES` of paths.
 */
export async function gitModifiedOrUntrackedPaths(cwd: string): Promise<string[]> {
  const listed = await gitStdout(cwd, ["ls-files", "-z", "--modified", "--others", "--exclude-standard"], MAX_LISTED_PATH_BYTES);
  return listed === undefined ? [] : listed.split("\0").filter(path => path !== "");
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
  child.on("error", () => settle());
  child.on("close", () => settle());
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

/**
 * The index's unmerged entries under `cwd`, by path relative to it. Each value holds the path's modes, blobs and stages,
 * so a later conflict on the same path reads differently. Undefined when Git cannot say: outside a repository, past the
 * timeout, or past `MAX_LISTED_PATH_BYTES` of entries.
 */
async function unmergedEntries(cwd: string): Promise<ReadonlyMap<string, string> | undefined> {
  const listed = await gitStdout(cwd, ["ls-files", "--unmerged", "-z"], MAX_LISTED_PATH_BYTES);
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
async function gitIndexPath(cwd: string): Promise<string | undefined> {
  const path = await gitOutput(cwd, ["rev-parse", "--git-path", "index"]);
  return path === undefined ? undefined : resolve(cwd, path);
}

/**
 * Reads the index's unmerged entries under a folder ({@link unmergedEntries}), for a caller that asks every pass: it
 * starts a Git process only when the folder's index file changed since the last read, and otherwise costs one `stat`
 * (plus one Git process the first time, to find the file). Git replaces that file with a new one on every write, so a
 * write changes its inode even inside one clock tick. A folder whose file cannot be found or stamped is read every time,
 * so this can cost more, never answer with stale entries. When Git cannot list them (it failed or timed out), the
 * answer is undefined and nothing is kept, so the next read asks Git again.
 */
export function gitUnmergedReader(): (cwd: string) => Promise<ReadonlyMap<string, string> | undefined> {
  const indexes = new Map<string, string>();
  const held = new Map<string, { readonly stamp: string; readonly entries: ReadonlyMap<string, string> }>();
  /** The stamp of `cwd`'s index file, found once and remembered; undefined when it cannot be found or stamped. */
  const indexStamp = async (cwd: string): Promise<string | undefined> => {
    const index = indexes.get(cwd) ?? await gitIndexPath(cwd);
    if (index === undefined) return undefined;
    indexes.set(cwd, index);
    try {
      return fileStamp(await stat(index));
    } catch {
      return undefined;
    }
  };
  return async cwd => {
    // Stamped before the read: an index written between the two reads as changed next time.
    const stamp = await indexStamp(cwd);
    const last = held.get(cwd);
    if (last !== undefined && last.stamp === stamp) return last.entries;
    held.delete(cwd);
    const entries = await unmergedEntries(cwd);
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
  child.on("error", () => settle());
  child.on("close", () => settle());
  // Git exits before reading every line when it cannot start in `cwd`; what it answered before that is kept.
  child.stdin.on("error", () => undefined);
  child.stdin.end(asked.map(revision => `${revision}\n`).join(""));
  await promise;
  return blobs;
}
