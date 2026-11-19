import type { ChildProcess } from "node:child_process";
import { accessSync, constants, existsSync, realpathSync, statSync } from "node:fs";
import { access, realpath, stat } from "node:fs/promises";
import { homedir, userInfo } from "node:os";
import { dirname, join, posix, resolve, win32 } from "node:path";
import { promisify } from "node:util";
import { MessageChannel, type MessagePort, receiveMessageOnPort, Worker } from "node:worker_threads";
import { endProcessTree, execFile, spawn, spawnSync } from "./child-process.js";
import { fileStamp } from "./file-stamp.js";
import {
  CONFIG_LISTING, configEntryVariables, driverOverrides, listedEntries, oldGitNotice, readDriverConfig, worktreeRead, type ListedEntry,
} from "./git-filter-drivers.js";
import {
  nameDiff, outsideSubmodules, withChangedSubmodules, type DiffRepository, type GitFailure, type GuardedRepository, type Places,
  type SubmoduleOpener,
} from "./git-submodule-state.js";

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

/** What {@link GitProbe} and {@link GitPathProbe} ask, asked off this process's thread: each answer is a promise. */
export interface GitProbeLater {
  exists(path: string): Promise<boolean>;
  runnable(path: string): Promise<boolean>;
  realpath(path: string): Promise<string>;
  developerDir(): Promise<string | undefined>;
}

/** {@link machine}'s questions, each asked through Node's thread pool or another process, so none waits in this thread. */
const machineLater: GitProbeLater = {
  async exists(path) {
    return await access(path).then(() => true, () => false);
  },
  async runnable(path) {
    try {
      if (!(await stat(path)).isFile()) return false;
      await access(path, constants.X_OK);
      return true;
    } catch {
      return false;
    }
  },
  async realpath(path) {
    return await realpath(path).catch(() => path);
  },
  async developerDir() {
    const selected = promisify(execFile)("/usr/bin/xcode-select", ["-p"], { encoding: "utf8", env: { PATH: "/usr/bin:/bin" } });
    return await selected.then(({ stdout }) => stdout.trim(), () => undefined);
  },
};

/** Thrown by a disk {@link askedInThread} gives at a question asked once its time ran out. */
class OutOfTime extends Error {}

/**
 * The code of the thread a look that must answer within a time limit reads the disk in ({@link askedInThread}): each
 * question asked as {@link machine} asks it, its answer posted back, then counted in `answered` and the waiting thread
 * woken.
 */
const DISK_THREAD = String.raw`
const { workerData } = require("node:worker_threads");
const { accessSync, constants, existsSync, realpathSync, statSync } = require("node:fs");
const { spawnSync } = require("node:child_process");
const { answered, port } = workerData;
const questions = {
  exists: path => existsSync(path),
  runnable(path) {
    try {
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
    // Hidden, as every child of Rerail's is (child-process.ts); only macOS asks it.
    const selected = spawnSync("/usr/bin/xcode-select", ["-p"], { encoding: "utf8", env: { PATH: "/usr/bin:/bin" }, windowsHide: true });
    return selected.status === 0 ? selected.stdout.trim() : undefined;
  },
};
// Counted once posted, so the count never runs ahead of the answers the waiting thread can read.
port.on("message", ({ question, path }) => {
  port.postMessage(questions[question](path));
  Atomics.add(answered, 0, 1);
  Atomics.notify(answered, 0);
});
`;

/** A thread that answers the disk questions of {@link GitProbe} and {@link GitPathProbe} ({@link DISK_THREAD}). */
interface DiskThread {
  /** The answer to `question` about `path`; throws {@link OutOfTime} once `deadline` (on `performance.now()`) passes. */
  ask(question: "exists" | "runnable" | "realpath" | "developerDir", path: string, deadline: number): unknown;
  close(): void;
}

/** A {@link DiskThread} started now, or undefined when this process cannot start one. */
function diskThread(): DiskThread | undefined {
  const answered = new Int32Array(new SharedArrayBuffer(4));
  const { port1, port2 } = new MessageChannel();
  let thread: Worker;
  try {
    thread = new Worker(DISK_THREAD, { eval: true, workerData: { answered, port: port2 }, transferList: [port2] });
  } catch {
    port1.close();
    return undefined;
  }
  // A question on a folder that does not answer holds the thread past its use; it then keeps no process running.
  thread.unref();
  let asked = 0;
  return {
    ask(question, path, deadline) {
      asked += 1;
      port1.postMessage({ question, path });
      return reply(port1, { answered, asked }, deadline);
    },
    close() {
      port1.close();
      thread.terminate().catch(() => undefined);
    },
  };
}

/** How many questions a {@link DiskThread} was asked, and the count of answers its thread posted (`answered[0]`). */
interface Answers {
  readonly answered: Int32Array;
  readonly asked: number;
}

/**
 * The answer to the last question asked on `port`, waited for in this thread until `deadline`, past which it throws
 * {@link OutOfTime}. It is read once the thread's count of answers reaches the questions asked, and each wait is for
 * that count to move from what was read: an answer counted before the wait began ends the wait at once, and no count
 * of an earlier answer can wake it with nothing to read, so the waiting thread sleeps until its answer comes.
 */
function reply(port: MessagePort, { answered, asked }: Answers, deadline: number): unknown {
  for (;;) {
    const count = Atomics.load(answered, 0);
    const received = count === asked ? receiveMessageOnPort(port) : undefined;
    if (received !== undefined) return received.message;
    const left = deadline - performance.now();
    if (left <= 0) throw new OutOfTime();
    Atomics.wait(answered, 0, count, left);
  }
}

/**
 * What `decide`, which asks the disk only through the probe it is given, answers when each question is asked in another
 * thread and waited for in this one, all within `timeoutMs`; undefined past it, or when no thread can be started. So a
 * call that blocks this thread can look for Git: a folder that does not answer (a share off the network: Windows waits
 * about 21 s for its server) holds it to its own time limit, as a Git start that waits on that folder is held.
 */
export function askedInThread<T>(timeoutMs: number, decide: (disk: GitProbe & GitPathProbe) => T): T | undefined {
  const deadline = performance.now() + timeoutMs;
  const thread = diskThread();
  if (thread === undefined) return undefined;
  try {
    return decide(threadDisk(thread, deadline));
  } catch (error) {
    if (error instanceof OutOfTime) return undefined;
    throw error;
  } finally {
    thread.close();
  }
}

/** {@link machine}'s questions, each asked of `thread` and answered by the time `deadline` passes. */
function threadDisk(thread: DiskThread, deadline: number): GitProbe & GitPathProbe {
  return {
    exists: path => thread.ask("exists", path, deadline) === true,
    runnable: path => thread.ask("runnable", path, deadline) === true,
    realpath(path) {
      const real = thread.ask("realpath", path, deadline);
      return typeof real === "string" ? real : path;
    },
    developerDir() {
      const folder = thread.ask("developerDir", "", deadline);
      return typeof folder === "string" ? folder : undefined;
    },
  };
}

/** Thrown by a {@link replayedProbe} at a question it has no answer for yet; `answer` asks it off the thread. */
class Unanswered extends Error {
  readonly answer: () => Promise<void>;

  constructor(answer: () => Promise<void>) {
    super("a look for Git asked the disk a question not answered yet");
    this.answer = answer;
  }
}

/**
 * A probe that answers each question from the answers `later` gave it, and throws {@link Unanswered} at a question it
 * has none for, carrying how to ask it.
 */
function replayedProbe(later: GitProbeLater): GitProbe & GitPathProbe {
  const answered = <T>(ask: (key: string) => Promise<T>): ((key: string) => T) => {
    const answers = new Map<string, { readonly value: T }>();
    return key => {
      const known = answers.get(key);
      if (known !== undefined) return known.value;
      throw new Unanswered(async () => {
        answers.set(key, { value: await ask(key) });
      });
    };
  };
  const developerDir = answered(async () => await later.developerDir());
  return {
    exists: answered(async path => await later.exists(path)),
    runnable: answered(async path => await later.runnable(path)),
    realpath: answered(async path => await later.realpath(path)),
    developerDir: () => developerDir(""),
  };
}

/**
 * What `decide`, which asks the disk only through the probe it is given, answers when every question it asks is asked
 * off this thread: `probe` is a {@link replayedProbe}. It runs with the answers so far; at a question it has none for,
 * that question is asked, and it runs again from the start. So it asks each question once, in its own order: a folder it
 * never reaches is never asked about, and one that does not answer (a share off the network: Windows waits about 21 s
 * for its server) delays this answer, never the thread.
 */
async function answeredLater<T>(decide: (probe: GitProbe & GitPathProbe) => T, probe: GitProbe & GitPathProbe): Promise<T> {
  try {
    return decide(probe);
  } catch (error) {
    if (!(error instanceof Unanswered)) throw error;
    await error.answer();
    return await answeredLater(decide, probe);
  }
}

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

/**
 * The build folders Git for Windows keeps its own git.exe in, in the order they are tried: `mingw64` (x64),
 * `clangarm64` (ARM64), `mingw32` (32-bit) and `ucrt64` (the x64 build of Git for Windows 2.56, whose install has no
 * `mingw64`). `ucrt64` comes last, so an install that also holds one of the other folders keeps running that folder's
 * git.exe.
 */
const GIT_FOR_WINDOWS_BUILDS = ["mingw64", "clangarm64", "mingw32", "ucrt64"];

/**
 * The Git program behind Git for Windows' launcher at `path`, or `path` itself. `<Git>\cmd\git.exe` and
 * `<Git>\bin\git.exe` set up the environment and run the build's own `<Git>\<build>\bin\git.exe` (a folder of
 * {@link GIT_FOR_WINDOWS_BUILDS}) as their own child. A timeout kills only the process Rerail started, and on Windows
 * killing a process leaves its children running, so the real Git ran on in the repository after Rerail gave up on it.
 * The real one runs the commands Rerail gives it without the launcher: it finds its own DLLs, helpers and system
 * configuration from its folder.
 *
 * A launcher is only a `git.exe` in a `cmd` or `bin` folder, which anyone can put on PATH beside a git.exe of their own,
 * so Rerail goes behind it only into a Git for Windows install: `<Git>\cmd\git.exe` runs, and `<Git>\etc\gitconfig` or
 * the build's `share\git` is there. The git.exe it then runs must pass what every PATH entry passes (`trustedFolder`):
 * outside `node_modules`, and outside the repository Rerail runs in by its path and its real path, which a junction
 * from `<Git>\<build>` into the repository would reach.
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
    && gitForWindowsFiles(root, build, probe)
    && trustedFolder(win32.join(root, build, "bin"), trust));
  return real === undefined ? path : win32.join(root, real, "bin", "git.exe");
}

/** Whether `root` holds what a Git for Windows install of `build` holds beside its git.exe: `etc\gitconfig` or `<build>\share\git`. */
function gitForWindowsFiles(root: string, build: string, probe: GitProbe): boolean {
  return probe.exists(win32.join(root, "etc", "gitconfig")) || probe.exists(win32.join(root, build, "share", "git"));
}

/** A Git for Windows install and the build folder ({@link GIT_FOR_WINDOWS_BUILDS}) whose own git.exe Rerail starts. */
interface GitForWindowsBuild {
  readonly root: string;
  readonly build: string;
}

/**
 * The install and build of `path` when it is a Git for Windows build's own git.exe, `<Git>\<build>\bin\git.exe`, in an
 * install whose launcher `<Git>\cmd\git.exe` runs ({@link behindLauncher}'s checks); else undefined.
 */
function gitForWindowsBuild(path: string, probe: GitProbe): GitForWindowsBuild | undefined {
  const bin = win32.dirname(path);
  const root = win32.dirname(win32.dirname(bin));
  const build = win32.basename(win32.dirname(bin)).toLowerCase();
  if (win32.basename(path).toLowerCase() !== "git.exe" || win32.basename(bin).toLowerCase() !== "bin" || !GIT_FOR_WINDOWS_BUILDS.includes(build)) return undefined;
  return probe.runnable(win32.join(root, "cmd", "git.exe")) && gitForWindowsFiles(root, build, probe) ? { root, build } : undefined;
}

/**
 * The environment a start of the Git at `path` gets from `env`: as it is, unless `path` is a Git for Windows build's own
 * git.exe ({@link gitForWindowsBuild}). Rerail starts that one without the launcher that would start it
 * ({@link gitBehindLauncher}), so it gets what the launcher gives it, `git-wrapper.c`'s `setup_environment`:
 * `PLINK_PROTOCOL` set to `ssh` when unset, `MSYSTEM` named for the build whatever it was, and `PATH` led by the build's
 * `bin`, the install's `usr\bin` and the home's `bin` ({@link launcherHomes}, by `shareExists`), in that order and spelled
 * as the launcher spells them. `HOME`, which the launcher sets when unset, git.exe sets itself by the same rule when
 * started without it (`compat/mingw.c`), so Rerail leaves it to git.exe. Git looks on that `PATH` for every program it runs: `sh.exe` for
 * a command it runs through its shell (a filter, a textconv, an alias, a credential helper), then the programs that
 * command names. Started without it, git.exe with `MSYSTEM` set and no `sh.exe` on `PATH` crashes there, and with
 * `MSYSTEM` unset puts the home's `bin` before its own folders, so a program of the person's of the same name runs in
 * place of Git's.
 *
 * Where the launcher would put a folder on `PATH` that Windows finds from the folder Git runs in, the repository, Rerail
 * puts none: the home's `bin` goes first only for a fully qualified home (`C:\Users\me`, `\\server\share\me`), never
 * an empty, relative or drive-relative one, and an empty or unset `PATH` adds no empty entry after Git's folders.
 */
export function gitStartEnvironment(path: string, env: NodeJS.ProcessEnv = process.env, probe: GitProbe = machine, shareExists?: boolean): NodeJS.ProcessEnv {
  const install = gitForWindowsBuild(path, probe);
  return install === undefined ? env : launcherEnvironment(install, env, shareExists);
}

/**
 * {@link gitStartEnvironment}'s environment for `install`, from `env` and what is known of the home share
 * (`shareExists`) alone: it reads no file, so a start waits on none.
 */
function launcherEnvironment({ root, build }: GitForWindowsBuild, env: NodeJS.ProcessEnv, shareExists: boolean | undefined): NodeJS.ProcessEnv {
  const first = [win32.join(root, build), win32.join(root, "usr"), ...launcherHomes(env, shareExists)].map(folder => win32.join(folder, "bin"));
  return withVariables(withoutEmpty(env, "HOME"), {
    MSYSTEM: build.toUpperCase(),
    PLINK_PROTOCOL: variable(env, "PLINK_PROTOCOL") ?? "ssh",
    PATH: [...first, ...listed(setVariable(env, "PATH"))].join(";"),
  });
}

/**
 * The homes whose `bin` goes on `PATH` after Git's folders, where the launcher puts its `HOME`'s: `HOME` when set, else
 * the launcher's choice ({@link unsetHomes}), each once whatever its case. An empty `HOME`, `HOMEPATH` or `USERPROFILE`
 * counts as unset, as {@link gitHome} reads them, and only a fully qualified home is kept.
 */
function launcherHomes(env: NodeJS.ProcessEnv, shareExists: boolean | undefined): string[] {
  const home = setVariable(env, "HOME");
  const homes = (home === undefined ? unsetHomes(env, shareExists) : [home]).filter(fullyQualified);
  return homes.filter((folder, index) => homes.findIndex(other => other.toLowerCase() === folder.toLowerCase()) === index);
}

/**
 * With `HOME` unset, the launcher takes the folder `HOMEDRIVE` and `HOMEPATH` name ({@link homeShare}) when it exists,
 * else `USERPROFILE`, and git.exe started without `HOME` takes the same in its own process. Only a look at the folder
 * tells which, and on a home share that does not answer (a domain account's home folder on a laptop off the network)
 * Windows waits about 21 s for the server: a look in Rerail's thread stopped every call the daemon serves for that
 * long. So a start in the daemon takes the answer of the last look off the thread (`shareExists`,
 * {@link ProcessGit.livesOn}): the share, or the profile. In any other process, and in the daemon before the
 * first answer, both, the share's first, so the launcher's is among them.
 */
function unsetHomes(env: NodeJS.ProcessEnv, shareExists: boolean | undefined): string[] {
  const share = homeShare(env);
  const profile = listed(setVariable(env, "USERPROFILE"));
  if (share === undefined || shareExists === false) return profile;
  return shareExists === true ? [share] : [share, ...profile];
}

/** The folder `HOMEDRIVE` and `HOMEPATH` name when `HOME` is unset and `HOMEPATH` set: the launcher's first choice of home. */
function homeShare(env: NodeJS.ProcessEnv): string | undefined {
  const homePath = setVariable(env, "HOMEPATH");
  if (setVariable(env, "HOME") !== undefined || homePath === undefined) return undefined;
  return `${variable(env, "HOMEDRIVE") ?? ""}${homePath}`;
}

/** Whether `folder` is a fully qualified Windows path: a drive's root or a share, and what is under it. */
function fullyQualified(folder: string): boolean {
  return /^(?:[A-Za-z]:[\\/]|\\\\)/.test(folder);
}

/** `env` without `name` when it is set to "": git.exe then sets it as when unset, as {@link gitHome} reads an empty `HOME`. */
function withoutEmpty(env: NodeJS.ProcessEnv, name: string): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(env).filter(([key, value]) => value !== "" || key.toUpperCase() !== name));
}

/** `[value]`, or `[]` when it is undefined. */
function listed(value: string | undefined): string[] {
  return value === undefined ? [] : [value];
}

/** The value of `name` in `env`, whatever case `env` spells it in, as Windows reads it. */
function variable(env: NodeJS.ProcessEnv, name: string): string | undefined {
  return Object.entries(env).find(([key]) => key.toUpperCase() === name)?.[1];
}

/** {@link variable}, undefined when it is empty. */
function setVariable(env: NodeJS.ProcessEnv, name: string): string | undefined {
  const value = variable(env, name);
  return value === "" ? undefined : value;
}

/** `env` with each of `values` set, under the spelling `env` already has for it (Windows keeps `Path` as `Path`). */
function withVariables(env: NodeJS.ProcessEnv, values: Record<string, string>): NodeJS.ProcessEnv {
  const spelled = new Map(Object.keys(env).map(key => [key.toUpperCase(), key]));
  return { ...env, ...Object.fromEntries(Object.entries(values).map(([name, value]) => [spelled.get(name) ?? name, value])) };
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
  return gitLook(search, probe)();
}

/**
 * A look for Git ({@link findGit}) by inputs read once ({@link gitLook}); each call looks again, reading the disk
 * through `probe`, by default the one its inputs were read through.
 */
export type GitLook = (probe?: GitProbe & GitPathProbe) => FoundGit;

/**
 * {@link findGit} with its inputs read once, now: `search` (the folder, `PATH` and the account's home) and the
 * repository above that folder ({@link gitTrust}). Each call of the look it returns decides by those inputs and reads
 * only each candidate on disk: whether it is a Git that runs, and the real path of its folder. So `PATH` and the
 * repository its entries are checked against stay those of the first look, whatever has happened since to that
 * repository's `.git`, the process's folder or its `PATH`.
 */
export function gitLook(search: GitSearch = currentSearch(), probe: GitProbe & GitPathProbe = machine): GitLook {
  const trust = gitTrust(search, probe);
  return (disk = probe) => firstInstalled(search, { ...trust, probe: disk }, disk);
}

/** The first candidate ({@link candidateOrder}) that {@link gitInstalled} takes, behind its launcher on Windows. */
function firstInstalled(search: GitSearch, trust: GitTrust, probe: GitProbe & GitPathProbe): FoundGit {
  const looked: string[] = [];
  for (const candidate of candidateOrder(search, trust)) {
    looked.push(candidate);
    if (gitInstalled(candidate, search.platform, probe)) {
      return { path: search.platform === "win32" ? behindLauncher(candidate, trust, probe) : candidate, looked };
    }
  }
  return { path: undefined, looked };
}

/**
 * The look a process keeps ({@link gitLook}), the Git it found last, and that Git's Git for Windows build when it is
 * one ({@link gitForWindowsBuild}), read by the same look.
 */
interface KeptGit {
  readonly look: GitLook;
  readonly found: FoundGit;
  readonly build: GitForWindowsBuild | undefined;
}

/** What `look` finds when it reads the disk through `disk`, with the Git for Windows build of that Git, read there too. */
function keptBy(look: GitLook, disk: GitProbe & GitPathProbe): KeptGit {
  const found = look(disk);
  return { look, found, build: found.path === undefined ? undefined : gitForWindowsBuild(found.path, disk) };
}

/**
 * How a {@link ProcessGit} reads the disk: `now` in its thread, for the first look, which a process makes before its
 * first Git call; `later` off its thread, for each look after a Git it kept is gone; `within`, in another thread with
 * each answer waited for in this one ({@link askedInThread}), for such a look made by a call that blocks; and `folder`,
 * off its thread, for whether the home share exists, in a process that follows it ({@link ProcessGit.livesOn}).
 */
export interface GitDisks {
  readonly now: GitProbe & GitPathProbe;
  readonly later: GitProbeLater;
  within<T>(timeoutMs: number, decide: (disk: GitProbe & GitPathProbe) => T): T | undefined;
  folder(path: string): Promise<boolean>;
}

/** This machine's disk, read as {@link GitDisks} says. */
const machineDisks: GitDisks = {
  now: machine,
  later: machineLater,
  within: askedInThread,
  async folder(path) {
    return await stat(path).then(found => found.isDirectory(), () => false);
  },
};

/**
 * Whether the folder `HOMEDRIVE` and `HOMEPATH` name exists, which the launcher asks before each start
 * ({@link unsetHomes}): the answer of the last look, which each start asks again off this thread, one look at a time.
 */
class HomeShare {
  readonly #folder: (path: string) => Promise<boolean>;
  #answer: { readonly share: string; readonly exists: boolean } | undefined;
  #asking = false;

  constructor(folder: (path: string) => Promise<boolean>) {
    this.#folder = folder;
  }

  /** For a start in `env`: the last answer for its share, undefined before the first; and has the share looked at again. */
  exists(env: NodeJS.ProcessEnv): boolean | undefined {
    const share = homeShare(env);
    // A share that is not fully qualified puts no folder on PATH (launcherHomes), so no answer is needed for it.
    if (share === undefined || !fullyQualified(share)) return undefined;
    const answer = this.#answer;
    this.#ask(share).catch(() => undefined);
    return answer?.share.toLowerCase() === share.toLowerCase() ? answer.exists : undefined;
  }

  async #ask(share: string): Promise<void> {
    if (this.#asking) return;
    this.#asking = true;
    try {
      this.#answer = { share, exists: await this.#folder(share) };
    } finally {
      this.#asking = false;
    }
  }
}

/**
 * The Git a process runs: looked for at the first ask and kept, and looked for again only when starting it found no
 * program at its path ({@link afterMissing}), by the first look's own inputs. A daemon outlives an upgrade of Git, and
 * Git for Windows 2.56 removed the `mingw64\bin\git.exe` that a daemon started on 2.55 runs (it moved to `ucrt64\bin`):
 * every Git call it made then failed until it restarted.
 */
export class ProcessGit {
  readonly #decide: () => GitLook;
  readonly #disks: GitDisks;
  #kept: KeptGit | undefined;
  /** The Git for Windows build of every Git kept so far, by its path, as the look that found it read it. */
  readonly #builds = new Map<string, GitForWindowsBuild | undefined>();
  /** The look after the Git kept was found gone, while it runs: every start that failed on that Git waits for it. */
  #looking: Promise<string | undefined> | undefined;
  /** The home share's last answer, once {@link livesOn} has the process follow it. */
  #share: HomeShare | undefined;
  /** Whether this process lives on past each call ({@link livesOn}), so a look for Git may outlast the call. */
  #livesOn = false;

  /** `decide` makes the look at the first ask, after the daemon has named its repository (`searchGitFrom`). */
  constructor(decide: () => GitLook, disks: GitDisks = machineDisks) {
    this.#decide = decide;
    this.#disks = disks;
  }

  /**
   * Has this process work as one that lives on past each call, the daemon, from now on: each start puts the launcher's
   * home's bin on `PATH` by the home share's last answer, asked again off this thread at each start ({@link HomeShare}),
   * and a start that finds no program at the Git kept looks for Git again ({@link afterMissing}, {@link replacedNow}).
   * Only the daemon calls this. Each leaves a question to the disk pending past the call that asked it: on a folder that
   * does not answer (a share off the network: Windows waits about 21 s for its server) a process cannot end until that
   * question returns, so a hook or a command, which ends after its answer, would live that long. Without it a start
   * puts both homes' bin on `PATH`, the share's first ({@link unsetHomes}), and asks the disk nothing; and a start that
   * finds no program at the Git kept fails as the start did, with no look, so the next process looks afresh at its
   * first ask.
   */
  livesOn(): void {
    this.#share ??= new HomeShare(path => this.#disks.folder(path));
    this.#livesOn = true;
  }

  /** The Git's path; throws {@link GitUnavailableError} when none is installed. */
  path(): string {
    this.#kept ??= this.#keep(keptBy(this.#decide(), this.#disks.now));
    const { found } = this.#kept;
    if (found.path === undefined) throw new GitUnavailableError(found.looked);
    return found.path;
  }

  /**
   * The environment a start of `executable`, a Git this process kept, gets from `env`: as {@link gitStartEnvironment}
   * gives it, by what the look that found that Git read and, in a process that follows it ({@link livesOn}),
   * the last answer about the home share. It reads no file, so no start waits on a folder.
   */
  startEnvironment(executable: string, env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
    const install = this.#builds.get(executable);
    return install === undefined ? env : launcherEnvironment(install, env, this.#share?.exists(env));
  }

  /**
   * The Git to start once more after a start of `failed`, the Git kept, found no program there (`ENOENT`): Git looked
   * for again by the look the first ask made, with the folder, `PATH` and repository it read then ({@link gitLook}),
   * and kept from then on. The look reads the disk off this thread, so a folder that does not answer delays the starts
   * waiting for it and no other work. A start that failed on a Git another start has already replaced gets that one
   * without a look, and every start that failed on the Git kept waits for the same look. Undefined when the look finds
   * no Git, or `failed` again: that start's failure stands and the Git kept stays, so the next start that fails looks
   * again, and no call starts the same missing program twice. With `limits`, the start waits for the look no longer
   * than its time limit and only until its signal is aborted: past either it gets undefined, while the look goes on and
   * keeps what it finds for the starts after it. Undefined with no look in a process that does not live on
   * ({@link livesOn}).
   */
  async afterMissing(failed: string, limits?: ReadLimits): Promise<string | undefined> {
    const kept = this.#kept;
    if (kept === undefined || kept.found.path !== failed) return kept?.found.path;
    if (!this.#livesOn) return undefined;
    this.#looking ??= this.#lookAgain(kept);
    return await (limits === undefined ? this.#looking : withinLimits(this.#looking, limits));
  }

  /**
   * For a start that blocks this thread ({@link runGit}): the Git kept when another start has already replaced `failed`;
   * else the Git a look finds and keeps, a look that reads the disk in another thread and is waited for here no longer
   * than `timeoutMs`, the call's own time limit ({@link GitDisks}). Undefined when that look finds no Git or `failed`
   * again, or runs out of time; then Git is looked for again off this thread ({@link afterMissing}) for the calls that
   * follow, and this one fails as its start did. Undefined with no look in a process that does not live on
   * ({@link livesOn}).
   */
  replacedNow(failed: string, timeoutMs: number): string | undefined {
    const kept = this.#kept;
    if (kept === undefined || kept.found.path !== failed) return kept?.found.path;
    if (!this.#livesOn) return undefined;
    const again = this.#disks.within(timeoutMs, disk => keptBy(kept.look, disk));
    // A look that fails leaves the Git kept, so the next start that fails looks again.
    if (again === undefined) this.#lookLater(failed);
    return again === undefined ? undefined : this.#replace(kept, again);
  }

  /** The look {@link afterMissing} makes, for the calls after this one; a failure of it is written to stderr. */
  #lookLater(failed: string): void {
    this.afterMissing(failed).catch((error: unknown) => {
      process.stderr.write(`rerail: looking for Git again after ${failed} was found gone failed: ${error instanceof Error ? error.message : String(error)}\n`);
    });
  }

  async #lookAgain(kept: KeptGit): Promise<string | undefined> {
    try {
      return this.#replace(kept, await answeredLater(disk => keptBy(kept.look, disk), replayedProbe(this.#disks.later)));
    } finally {
      this.#looking = undefined;
    }
  }

  /** The Git `again` found, kept from now on, when it is not `kept`'s; else undefined, and the Git kept stays. */
  #replace(kept: KeptGit, again: KeptGit): string | undefined {
    if (again.found.path === undefined || again.found.path === kept.found.path) return undefined;
    this.#kept = this.#keep(again);
    return again.found.path;
  }

  #keep(kept: KeptGit): KeptGit {
    if (kept.found.path !== undefined) this.#builds.set(kept.found.path, kept.build);
    return kept;
  }
}

/** The Git this process runs, looked for by {@link gitLook} from this process's folder and `PATH` at its first ask. */
const processGit = new ProcessGit(() => gitLook());

/**
 * The system Git ({@link findGit}), kept for the process and looked for again only once it is gone ({@link ProcessGit});
 * throws {@link GitUnavailableError} when none is installed.
 */
export function gitExecutable(): string {
  return processGit.path();
}

/**
 * Has this process's Git work as one that lives on past each call ({@link ProcessGit.livesOn}): each start following
 * the home share's last answer, and Git looked for again once the Git kept is gone. Only a process that lives on calls
 * it: the daemon ({@link prepareDaemonGit}), and tests that look for Git again.
 */
export function gitLivesOn(): void {
  processGit.livesOn();
}

/**
 * Has this process's Git run as the daemon's, which serves the checkout in `folder` and lives on: looked for from that
 * folder ({@link searchGitFrom}), and working as a process that lives on does ({@link gitLivesOn}). The daemon calls it
 * as it starts, before anything runs Git.
 */
export function prepareDaemonGit(folder: string): void {
  searchGitFrom(folder);
  gitLivesOn();
}

/** Whether a Git start failed because no program was at the path it named: Node's `ENOENT` from the start itself. */
function programMissing(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}

/**
 * The Git to start once more after a start of `executable`, the Git kept, found no program there (`missing`): the Git
 * found again ({@link ProcessGit.afterMissing}), waited for within the call's time limit and until its signal is aborted
 * (`limits`). Undefined when that start found a program, past either limit, or once the signal is aborted: a stopped
 * call starts no other Git and looks for none, and its first answer stands, as when it is aborted during the look.
 */
async function gitAgain(executable: string, missing: boolean, limits: ReadLimits): Promise<string | undefined> {
  if (!missing || stopped(limits.signal)) return undefined;
  const again = await processGit.afterMissing(executable, limits);
  return stopped(limits.signal) ? undefined : again;
}

/**
 * `look`'s answer, or undefined once `timeoutMs` has passed or `signal` is aborted, should that come first; the look
 * goes on either way.
 */
async function withinLimits<T>(look: Promise<T>, { timeoutMs, signal }: ReadLimits): Promise<T | undefined> {
  const { promise: ended, resolve: end } = Promise.withResolvers<undefined>();
  const stop = (): void => end(undefined);
  const timer = setTimeout(stop, timeoutMs);
  signal?.addEventListener("abort", stop, { once: true });
  // The look can stop the call as it starts, before this listens: a read's owner stops it while the disk is asked.
  if (signal?.aborted === true) stop();
  try {
    return await Promise.race([look, ended]);
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", stop);
  }
}

/** Whether `signal` is aborted, read anew at each call: the look {@link gitAgain} waits for can outlast it. */
function stopped(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

/**
 * Git Bash's `bash.exe` in the Git for Windows install Rerail runs Git from ({@link gitExecutable}), or undefined
 * without Git or a Git Bash beside it. `<Git>\bin\bash.exe` sits a folder or two above its git.exe: `cmd\`, `bin\` or
 * `<build>\bin\` ({@link GIT_FOR_WINDOWS_BUILDS}).
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
  /** Set when Git could not be started, timed out, was stopped by a signal, or overflowed `maxBuffer`. */
  error?: Error;
}

export interface RunGitOptions {
  maxBuffer?: number;
  timeout?: number;
  /** Variables set over the inherited environment; an undefined value removes one. Rerail's fixed ones ({@link gitEnvironment}) still win. */
  env?: Readonly<Record<string, string | undefined>>;
}

/**
 * What every Git call sets so that a repository's own config starts no program through it: no fsmonitor, no hooks, and
 * no signature check, which `git reflog` and `git log` make under `log.showSignature` by starting `gpg.program` for each
 * signed commit they list. A call that reads worktree files also needs the filter drivers a repository's config
 * defines set to no program, which {@link runGit} and the reads that do not block do on Git 2.31 or later
 * ({@link guardedSync}); `runGitAsync` and `streamGit` do not, and of their calls only Merge Assist's diff of its base
 * against the working files (internal testing) reads a worktree file.
 */
const SAFE_CONFIG: readonly string[] = [
  "-c", "core.fsmonitor=false", "-c", `core.hooksPath=${process.platform === "win32" ? "NUL" : "/dev/null"}`, "-c", "log.showSignature=false",
];
const DEFAULT_TIMEOUT_MS = 10_000;
/** The most bytes the guard's own questions (a repository's config, its index's submodules) take: past it, the call it guards is not made. */
const MAX_LISTING_BYTES = 1024 * 1024 * 1024;

/**
 * The environment of every Git process Rerail starts, a start of the Git at `executable`: this process's with
 * `overrides` set over it, as Git for Windows' launcher would hand it to that Git ({@link gitStartEnvironment}), with
 * no optional locks, and no lazy fetch of a partial clone's missing objects, which no caller's `overrides` can lift.
 * Git honours `GIT_NO_LAZY_FETCH` from 2.45.0 and in the 2024-04-19 security releases 2.39.4, 2.40.2, 2.41.1, 2.42.2,
 * 2.43.4 and 2.44.1; an older Git ignores it and may fetch from the remote.
 */
function gitEnvironment(executable: string, overrides: RunGitOptions["env"] = {}): NodeJS.ProcessEnv {
  return { ...processGit.startEnvironment(executable, { ...process.env, ...overrides }), GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1" };
}

/**
 * What is left of a time limit of `limitMs` that starts now, in whole milliseconds, as each call asks: 0 once it has
 * passed. A call that starts Git once more ({@link ProcessGit.afterMissing}) gives its look and that start only what
 * is left of its own limit, so the call ends within it.
 */
function timeLeft(limitMs: number): () => number {
  const deadline = performance.now() + limitMs;
  return () => Math.max(0, Math.floor(deadline - performance.now()));
}

/**
 * `git -C <cwd> <args>` with fsmonitor, hooks and signature checks disabled ({@link SAFE_CONFIG}), in
 * {@link gitEnvironment} (no optional locks, no lazy fetch of a partial clone's missing objects), and a 10 s default
 * timeout; throws {@link GitUnavailableError} when Git is missing, never on a non-zero exit. A call that reads worktree
 * files ({@link worktreeRead}) runs with each filter driver the repository's config defines set as the person's own
 * config sets it, or to no program ({@link guardedSync}); a `git diff --name-only -z` that would look into submodules
 * runs with Git kept out of them, and Rerail asks each through the same guard (`git-submodule-state.ts`). A call
 * whose config Git cannot list is not made: its result carries the listing's status and error, and none of its output.
 * On a Git older than 2.31 every call runs as it did before the guard, and the process says why once ({@link guardWorks}).
 */
export function runGit(cwd: string, args: readonly string[], options: RunGitOptions = {}): GitResult {
  const read = worktreeRead(args);
  if (read === "none" || !guardWorks(cwd)) return startGitSync(cwd, args, options);
  const guarded = guardedSync(cwd, options.env);
  if (!("entries" in guarded)) return guarded;
  if (read === "files") return startGitSync(cwd, args, { ...options, env: guarded.env });
  return diffOutsideSubmodules(cwd, args, options, guarded);
}

/** The variables a Git call sets over the inherited environment ({@link RunGitOptions.env}). */
type GitVariables = NonNullable<RunGitOptions["env"]>;

/** A repository's config as the guard listed it, and the variables its calls run with: each driver it defines set. */
interface Guarded {
  readonly entries: readonly ListedEntry[];
  readonly env: GitVariables;
}

/**
 * The guard for calls in `cwd`'s repository with `env` set: its config listed once ({@link CONFIG_LISTING}), and `env`
 * with each filter driver the repository's config defines set as the person's own config sets it, or to no program
 * (`git-filter-drivers.ts`). How the listing failed instead (a config Git cannot parse, a timeout), with none of its
 * output. The listing runs without `GIT_CONFIG`, which `git config` alone reads in place of every config file, so a
 * listing in its presence would miss the files the guarded call reads. The listing and the call are two Git
 * processes: a writer that changes the config between them can have its driver started (docs/threat-model.md, T11).
 */
function guardedSync(cwd: string, env: GitVariables = {}): Guarded | GitResult {
  const listing = startGitSync(cwd, CONFIG_LISTING, { env: { ...env, GIT_CONFIG: undefined }, maxBuffer: MAX_LISTING_BYTES });
  return listing.status === 0 && listing.error === undefined ? guardedBy(listing.stdout, env) : unanswered(listing);
}

/** {@link Guarded} from the config `listing` of a repository whose calls run with `env` set. */
function guardedBy(listing: string, env: GitVariables): Guarded {
  const entries = listedEntries(listing);
  const drivers = configEntryVariables({ ...process.env, ...env }, driverOverrides(readDriverConfig(entries)));
  return { entries, env: { ...env, ...drivers } };
}

/** A failed question asked for a call, as that call's result: its status, stderr and error, none of its output. */
function unanswered(failure: GitFailure): GitResult {
  return { status: failure.status, stdout: "", stderr: failure.stderr, ...(failure.error === undefined ? {} : { error: failure.error }) };
}

/**
 * A call that would look into submodules, under `guarded`. A `git diff --name-only -z` is answered as Git answers it
 * with Git looking into none: run with `--ignore-submodules=dirty`, then each submodule it would have looked into asked
 * through the guard, opened as Git opens a submodule (`git-submodule-state.ts`). Any other such call (Rerail makes
 * none) runs as it is, so Git looks into each submodule with that submodule's drivers as they are.
 */
function diffOutsideSubmodules(cwd: string, args: readonly string[], options: RunGitOptions, guarded: Guarded): GitResult {
  const diff = nameDiff(args);
  if (diff === undefined) return startGitSync(cwd, args, { ...options, env: guarded.env });
  const outside = startGitSync(cwd, outsideSubmodules(args), { ...options, env: guarded.env });
  if (outside.status !== 0 || outside.error !== undefined) return outside;
  const answer = withChangedSubmodules(outside.stdout, diff, diffRepository(cwd, guarded), submoduleOpener(options.env));
  return typeof answer === "string" ? { ...outside, stdout: answer } : unanswered(answer);
}

/** Stdout of each question `git -C <cwd> <args>` with `env` set, or how it failed. */
function asker(cwd: string, env: GitVariables): (args: readonly string[]) => string | GitFailure {
  return args => {
    const answer = startGitSync(cwd, args, { env, maxBuffer: MAX_LISTING_BYTES });
    return answer.status === 0 && answer.error === undefined ? answer.stdout : answer;
  };
}

/** The repository a guarded diff runs in, as `git-submodule-state.ts` asks it. */
function diffRepository(cwd: string, guarded: Guarded): DiffRepository {
  const ask = asker(cwd, guarded.env);
  return { entries: guarded.entries, ask, place: () => places(ask) };
}

/** The top of the worktree a question runs in, and its folder below that top; how asking failed instead. */
function places(ask: (args: readonly string[]) => string | GitFailure): Places | GitFailure {
  const answer = ask(["rev-parse", "--show-toplevel", "--show-prefix"]);
  if (typeof answer !== "string") return answer;
  const [top = "", prefix = ""] = answer.split("\n");
  return { top, prefix };
}

/**
 * The variables Git clears before it starts a Git in a submodule (`local_repo_env` in Git's environment.c, less the
 * command-line config that `prepare_other_repo_env` hands on); it then sets `GIT_DIR=.git` in the submodule's folder.
 */
const SUBMODULE_CLEARED = [
  "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG", "GIT_OBJECT_DIRECTORY", "GIT_DIR", "GIT_WORK_TREE", "GIT_IMPLICIT_WORK_TREE",
  "GIT_GRAFT_FILE", "GIT_INDEX_FILE", "GIT_NO_REPLACE_OBJECTS", "GIT_REPLACE_REF_BASE", "GIT_PREFIX", "GIT_SHALLOW_FILE", "GIT_COMMON_DIR",
] as const;

/**
 * Opens each submodule through the guard as Git opens one for the `git status` it runs there: in the submodule's
 * folder with the variables of {@link SUBMODULE_CLEARED} cleared and `GIT_DIR=.git`, which also skips the ownership
 * check, as Git's does; `env` is the variables the diff was asked with.
 */
function submoduleOpener(env: GitVariables = {}): SubmoduleOpener {
  const inSubmodule: GitVariables = { ...env, ...Object.fromEntries(SUBMODULE_CLEARED.map(name => [name, undefined])), GIT_DIR: ".git" };
  return folder => {
    const guarded = guardedSync(folder, inSubmodule);
    if (!("entries" in guarded)) return guarded;
    const repository: GuardedRepository = { entries: guarded.entries, ask: asker(folder, guarded.env), places: { top: folder, prefix: "" } };
    return repository;
  };
}

/** What this process's Git means for the guard: what Rerail says when it is too old ({@link oldGitNotice}), and whether it said it. */
interface GuardGit {
  readonly notice: string | undefined;
  told: boolean;
}

let guardGit: GuardGit | undefined;

/** {@link GuardGit} from `git version` asked in `cwd`; undefined when Git did not answer, so a later call asks again. */
function askedGuardGitSync(cwd: string): GuardGit | undefined {
  const answer = startGitSync(cwd, ["version"]);
  return answer.status === 0 ? { notice: oldGitNotice(answer.stdout), told: false } : undefined;
}

/**
 * Whether the guard covers this process's Git calls: a Git it works with (2.31 or later), asked once. With an older one
 * every call runs as it ran before the guard, so each answers as that Git answers, and the first call the guard would
 * cover says why on the process's error stream, which for the daemon is its log.
 */
function guardWorks(cwd: string): boolean {
  guardGit ??= askedGuardGitSync(cwd);
  return guardCovers(guardGit);
}

/** {@link guardWorks} for a read that does not block. */
async function guardWorksLater(cwd: string, signal: AbortSignal | undefined): Promise<boolean> {
  const asked = guardGit ?? await askedGuardGit(cwd, signal);
  guardGit ??= asked;
  return guardCovers(asked);
}

/** {@link askedGuardGitSync} without blocking. */
async function askedGuardGit(cwd: string, signal: AbortSignal | undefined): Promise<GuardGit | undefined> {
  const answer = await readGitWith(cwd, ["version"], { signal, maxBuffer: GIT_OUTPUT_MAX_BYTES, env: {} });
  return answer.ended === "printed" ? { notice: oldGitNotice(answer.stdout), told: false } : undefined;
}

/** Whether `git` is one the guard works with, saying once why not when it is older. */
function guardCovers(git: GuardGit | undefined): boolean {
  if (git?.notice === undefined) return true;
  if (!git.told) process.stderr.write(`rerail: ${git.notice}\n`);
  git.told = true;
  return false;
}

/**
 * `git -C <cwd> <args>` as {@link runGit} makes every call, with no guard of its own: when the Git kept for the process
 * is gone, it runs once more with the Git found again ({@link ProcessGit.replacedNow}): the look and that second start
 * get what is left of the call's time limit ({@link timeLeft}), since the call blocks the thread and a look may wait on
 * a folder far longer; with none left, the call fails as its start did.
 */
function startGitSync(cwd: string, args: readonly string[], options: RunGitOptions = {}): GitResult {
  const command = [...SAFE_CONFIG, "-C", cwd, ...args];
  const left = timeLeft(options.timeout ?? DEFAULT_TIMEOUT_MS);
  const executable = gitExecutable();
  const result = runGitWith(executable, command, options);
  const again = programMissing(result.error) ? processGit.replacedNow(executable, left()) : undefined;
  const rest = left();
  return again === undefined || rest === 0 ? result : runGitWith(again, command, { ...options, timeout: rest });
}

/** One blocking start of the Git at `executable` with `args`, as {@link runGit} makes it. */
function runGitWith(executable: string, args: readonly string[], options: RunGitOptions): GitResult {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    timeout: DEFAULT_TIMEOUT_MS,
    ...options,
    env: gitEnvironment(executable, options.env),
  });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "", ...(result.error === undefined ? {} : { error: result.error }) };
}

/** What {@link runGitAsync} takes: {@link RunGitOptions}, {@link GitReadOptions}, and the text written to Git's stdin. */
type RunGitAsyncOptions = RunGitOptions & GitReadOptions & { input?: string };

/**
 * {@link runGit} without blocking: the same safe configuration, environment and timeout, and `input` written to Git's
 * stdin. Rejects with {@link GitUnavailableError} when Git is missing, never on a non-zero exit; a Git that could not
 * start, timed out, was stopped by `signal` or overflowed `maxBuffer` (1 MiB by default, for stdout and stderr each)
 * answers a null status with its error. The timeout and `signal` end Git as they end a batch read
 * ({@link settleWhenDone}), with every process it started, and the answer comes once that Git has exited
 * ({@link GitReadOptions}); once `signal` is aborted it starts no Git. When the Git kept for the process is gone, it
 * runs once more with the Git found again, looked for off this thread while this call waits, all within its one time
 * limit and unless `signal` is aborted by then ({@link startedWithin}).
 */
export async function runGitAsync(cwd: string, args: readonly string[], options: RunGitAsyncOptions = {}): Promise<GitResult> {
  const command = [...SAFE_CONFIG, "-C", cwd, ...args];
  const limits = { timeoutMs: options.timeout ?? DEFAULT_TIMEOUT_MS, signal: options.signal };
  return await startedWithin(gitExecutable(), limits, async (executable, timeout) => {
    const result = await runGitAsyncWith(executable, command, { ...options, timeout });
    return { answer: result, missing: programMissing(result.error) };
  });
}

/** One start of the Git at `executable` with `args`, as {@link runGitAsync} makes it. */
async function runGitAsyncWith(executable: string, args: readonly string[], options: RunGitAsyncOptions): Promise<GitResult> {
  const { maxBuffer = 1024 * 1024, timeout = DEFAULT_TIMEOUT_MS, signal } = options;
  if (signal?.aborted === true) return { status: null, stdout: "", stderr: "", error: new Error("Git was not started: the read was stopped") };
  const { promise, resolve: settle } = Promise.withResolvers<GitResult>();
  const child = startGit(() => spawn(executable, args, { env: gitEnvironment(executable), stdio: ["pipe", "pipe", "pipe"] }));
  if (child instanceof Error) return { status: null, stdout: "", stderr: "", error: child };
  const printed = { stdout: [] as string[], stderr: [] as string[] };
  let failure: Error | undefined;
  for (const stream of ["stdout", "stderr"] as const) {
    let bytes = 0;
    child[stream].setEncoding("utf8");
    child[stream].on("data", (chunk: string) => {
      bytes += Buffer.byteLength(chunk);
      if (bytes <= maxBuffer) printed[stream].push(chunk);
      else if (failure === undefined) {
        failure = new Error(`Git printed more than maxBuffer (${maxBuffer} bytes) to ${stream}`);
        endPastMaxBuffer(child);
      }
    });
  }
  // Registered before `settleWhenDone`'s listeners, so the answer below sees why Git ended.
  child.on("error", error => {
    failure ??= error;
  });
  settleWhenDone(child, ended => {
    // Once answered, keeps no pipe open that a process Git started may still hold.
    child.stderr.destroy();
    const output = { stdout: printed.stdout.join(""), stderr: printed.stderr.join("") };
    if (ended) {
      const why = signal?.aborted === true ? "the read was stopped" : `it ran past its time limit of ${timeout} ms`;
      settle({ status: null, ...output, error: new Error(`Git was ended: ${why}`) });
    } else if (failure !== undefined || child.exitCode === null) {
      settle({ status: null, ...output, error: failure ?? new Error(`Git was ended by ${child.signalCode ?? "a signal"}`) });
    } else settle({ status: child.exitCode, ...output });
  }, { timeoutMs: timeout, signal });
  // Git may exit before reading all of its input; the broken pipe is no failure of the read.
  child.stdin.on("error", () => undefined);
  child.stdin.end(options.input ?? "");
  const [result] = await Promise.all([promise, exited(child)]);
  return result;
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

/** What one start of Git answered, and whether it found no program at the path it named ({@link programMissing}). */
interface GitStart<T> {
  readonly answer: T;
  readonly missing: boolean;
}

/**
 * `read` run with the Git to read with ({@link readerGit}), and once more with the Git found again when that start found
 * no program at its path ({@link startedWithin}), within the read's one time limit and unless its signal was aborted by
 * then (`limits`): a stopped read starts no other Git. Undefined without Git, or once the signal is aborted before the
 * first start.
 */
async function readWithGit<T>(limits: ReadLimits, read: (executable: string, limits: ReadLimits) => Promise<GitStart<T>>): Promise<T | undefined> {
  const executable = readerGit(limits.signal);
  if (executable === undefined) return undefined;
  return await startedWithin(executable, limits, async (git, timeoutMs) => await read(git, { ...limits, timeoutMs }));
}

/**
 * `start` with the Git at `executable` and the call's time limit, and once more with the Git found again when that
 * start found no program at its path ({@link gitAgain}), all within that one limit: the look and the second start get
 * what is left of it ({@link timeLeft}), and with none left the first answer stands. So a call that starts Git twice
 * ends within its own time limit, as one that starts it once does.
 */
async function startedWithin<T>(executable: string, limits: ReadLimits, start: (executable: string, timeoutMs: number) => Promise<GitStart<T>>): Promise<T> {
  const left = timeLeft(limits.timeoutMs);
  const first = await start(executable, limits.timeoutMs);
  const again = await gitAgain(executable, first.missing, { ...limits, timeoutMs: left() });
  const rest = left();
  return again === undefined || rest === 0 ? first.answer : (await start(again, rest)).answer;
}

/**
 * Starts a Git with `start` and gives back its process, or the error the start threw. Node tells of a start that failed
 * with ENOENT, EACCES, EAGAIN, EMFILE or ENFILE through the process's `error` event, and throws any other: on Windows
 * EFTYPE for a git.exe that is not a program, such as one an installer is still writing, which a look for Git can find.
 * A call it ends so answers as one whose Git could not start, and only ENOENT looks for Git again.
 */
function startGit<T extends ChildProcess>(start: () => T): T | Error {
  try {
    return start();
  } catch (error) {
    if (startThrew(error)) return error;
    throw error;
  }
}

/** Whether `error` is one Node threw at the start of a program (its `syscall` is `spawn`), not a fault in the call. */
function startThrew(error: unknown): error is Error {
  return error instanceof Error && "syscall" in error && error.syscall === "spawn";
}

/**
 * Whether the start of `child` found no program at its path ({@link programMissing}), as its `error` event tells; asked
 * once the read has settled.
 */
function startMissing(child: ChildProcess): () => boolean {
  let missing = false;
  child.once("error", error => {
    missing = programMissing(error);
  });
  return () => missing;
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

/** The most bytes of stdout `gitStdout` reads by default: past it, the read fails. */
const GIT_OUTPUT_MAX_BYTES = 1024 * 1024;

/** Stdout, as Git wrote it, of a successful Git command run as `gitOutput` runs one; undefined past `maxBuffer` bytes. */
async function gitStdout(cwd: string, args: readonly string[], signal: AbortSignal | undefined, maxBuffer = GIT_OUTPUT_MAX_BYTES): Promise<string | undefined> {
  const read = await readGit(cwd, args, signal, maxBuffer);
  return read.ended === "printed" ? read.stdout : undefined;
}

/**
 * How a Git read ended: with what Git printed, failing (no Git, an error, past `maxBuffer`), or ended past its time limit;
 * one its `signal` stopped reads the same ({@link GitReadOptions}).
 */
type GitRead = { readonly ended: "printed"; readonly stdout: string } | { readonly ended: "failed" | "timed-out" };

/**
 * Runs a Git command as `gitOutput` does, and says how it ended. Git's stderr goes to the null device, as `streamGit`'s
 * does: no answer reads it, and a process Git started that outlives Git (a repository's clean filter can leave one)
 * holds it for as long as it runs and writes there as much as it likes, while the read answers once Git has exited and
 * its output has closed. Run again with the Git found again when the one kept is gone ({@link readWithGit}). A read of
 * worktree files runs under the guard {@link runGit} puts on such a call ({@link readerGuard}).
 */
async function readGit(cwd: string, args: readonly string[], signal: AbortSignal | undefined, maxBuffer: number): Promise<GitRead> {
  const guard = await readerGuard(cwd, args, signal);
  return "ended" in guard ? guard : await readGitWith(cwd, args, { signal, maxBuffer, env: guard.env });
}

/**
 * The variables a read of `args` in `cwd` runs with: for one that reads worktree files, each filter driver the
 * repository's config defines set as {@link guardedSync} sets it, from a listing that does not block; how that listing
 * ended instead. No such read looks into submodules: one that would runs with Git looking into them as it does.
 */
async function readerGuard(cwd: string, args: readonly string[], signal: AbortSignal | undefined): Promise<{ readonly env: GitVariables } | GitReadEnd> {
  const read = worktreeRead(args);
  if (read === "none" || !(await guardWorksLater(cwd, signal))) return { env: {} };
  const listing = await readGitWith(cwd, CONFIG_LISTING, { signal, maxBuffer: MAX_LISTING_BYTES, env: { GIT_CONFIG: undefined } });
  return listing.ended === "printed" ? guardedBy(listing.stdout, {}) : listing;
}

/** How a Git read ended without an answer ({@link GitRead}). */
type GitReadEnd = Exclude<GitRead, { readonly ended: "printed" }>;

/** What one start of a Git read takes: its stop, the most it prints, and the variables it sets. */
interface ReadStart {
  readonly signal: AbortSignal | undefined;
  readonly maxBuffer: number;
  readonly env: GitVariables;
}

/** One Git read as {@link readGit} makes it, with `start.env` set and no guard of its own. */
async function readGitWith(cwd: string, args: readonly string[], { signal, maxBuffer, env }: ReadStart): Promise<GitRead> {
  const read = await readWithGit<GitRead>({ timeoutMs: DEFAULT_TIMEOUT_MS, signal }, async (executable, limits) => {
    const { promise, resolve: settle } = Promise.withResolvers<GitRead>();
    const child = startGit(() => spawn(executable, [...SAFE_CONFIG, "-C", cwd, ...args], {
      env: { ...gitEnvironment(executable, env), GIT_TERMINAL_PROMPT: "0" },
      stdio: ["pipe", "pipe", "ignore"],
    }));
    if (child instanceof Error) return { answer: { ended: "failed" }, missing: false };
    const missing = startMissing(child);
    const printed = gitPrinted(child, maxBuffer);
    // This `close` listener comes first and answers with what a Git that exited with status 0 printed. The next answers
    // every other end, saying whether the time limit or `signal` ended the read, and a Git that was ended and whose
    // output stays open.
    child.on("close", (status: number | null) => {
      const stdout = printed(status);
      if (stdout !== undefined && signal?.aborted !== true) settle({ ended: "printed", stdout });
    });
    settleWhenDone(child, ended => settle({ ended: ended ? "timed-out" : "failed" }), limits);
    const [answer] = await Promise.all([promise, exited(child)]);
    return { answer, missing: missing() };
  });
  return read ?? { ended: signal?.aborted === true ? "timed-out" : "failed" };
}

/**
 * Reads what `child`, the Git of a read, prints: as UTF-8, and at most `maxBuffer` bytes, past which it ends Git
 * ({@link endPastMaxBuffer}). Answers that text for the `status` Git exited with: only for 0, and only within
 * `maxBuffer`.
 */
function gitPrinted(child: ChildProcess, maxBuffer: number): (status: number | null) => string | undefined {
  const chunks: string[] = [];
  let bytes = 0;
  child.stdout?.setEncoding("utf8");
  child.stdout?.on("data", (chunk: string) => {
    bytes += Buffer.byteLength(chunk);
    if (bytes <= maxBuffer) chunks.push(chunk);
    else endPastMaxBuffer(child);
  });
  return status => (status === 0 && bytes <= maxBuffer ? chunks.join("") : undefined);
}

/** Ends a Git that printed more than its read takes: closes its output, then sends it SIGTERM. */
function endPastMaxBuffer(child: ChildProcess): void {
  child.stdout?.destroy();
  child.kill();
}

/** The commit `HEAD` names in `cwd`; undefined outside a repository, or before its first commit. */
export async function gitHead(cwd: string, options: GitReadOptions = {}): Promise<string | undefined> {
  return await gitOutput(cwd, ["rev-parse", "--verify", "--quiet", "HEAD"], options.signal);
}

/**
 * The exclude files of the Git checkout at `cwd` that no diff shows, absolute: its `info/exclude` (the common Git
 * folder's, which every worktree shares) and the file `core.excludesFile` names, else Git's default, `git/ignore` in
 * `XDG_CONFIG_HOME` or `~/.config`. Undefined when Git does not name them (outside a Git checkout, without Git, or a
 * Git that failed, ran past its time limit or was stopped): none is not its answer.
 */
export async function gitExcludeFiles(cwd: string, options: GitReadOptions = {}): Promise<string[] | undefined> {
  // Git prints the path relative to `cwd` unless it is elsewhere; `--path-format` would need Git 2.31.
  const exclude = await gitOutput(cwd, ["rev-parse", "--git-path", "info/exclude"], options.signal);
  if (exclude === undefined) return undefined;
  if (exclude === "") return [];
  // Git fails when `core.excludesFile` is unset: Git's default then holds.
  const configured = await readGit(cwd, ["config", "--path", "--get", "core.excludesFile"], options.signal, GIT_OUTPUT_MAX_BYTES);
  if (configured.ended === "timed-out") return undefined;
  const named = configured.ended === "printed" ? configured.stdout.trim() : "";
  return [resolve(cwd, exclude), resolve(cwd, named || defaultExcludesFile())];
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
 * `include.path` pulls in are left out. Undefined when Git does not name them, as for {@link gitExcludeFiles}.
 */
export async function gitConfigFiles(
  cwd: string,
  options: GitReadOptions & { env?: NodeJS.ProcessEnv; platform?: NodeJS.Platform } = {},
): Promise<string[] | undefined> {
  const { env = process.env, platform = process.platform } = options;
  const local = await gitOutput(cwd, ["rev-parse", "--git-path", "config", "--git-path", "config.worktree"], options.signal);
  if (local === undefined) return undefined;
  if (local === "") return [];
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
 * How a listing of `gitModifiedOrUntrackedListing` ended: with the paths Git listed, failing (no Git, an error, outside a
 * repository, past `MAX_LISTED_PATH_BYTES` of paths), or past its time limit or stopped ({@link GitRead}).
 */
export type ChangedPathsListing = { readonly ended: "printed"; readonly paths: string[] } | { readonly ended: "failed" | "timed-out" };

/** Lists the files `gitModifiedOrUntrackedPaths` answers, and says how the listing ended. */
export async function gitModifiedOrUntrackedListing(cwd: string, options: GitReadOptions = {}): Promise<ChangedPathsListing> {
  const read = await readGit(cwd, ["ls-files", "-z", "--modified", "--others", "--exclude-standard"], options.signal, MAX_LISTED_PATH_BYTES);
  return read.ended === "printed" ? { ended: "printed", paths: read.stdout.split("\0").filter(path => path !== "") } : read;
}

/**
 * The files of the checkout at `cwd` whose text its index may not hold: modified since staged (or deleted), and untracked
 * but not ignored; relative to `cwd`. Empty without Git, outside a repository, or past `MAX_LISTED_PATH_BYTES` of paths.
 * Undefined when Git ran past its time limit, or `options.signal` stopped it: it did not say which files changed, and
 * none is not its answer.
 */
export async function gitModifiedOrUntrackedPaths(cwd: string, options: GitReadOptions = {}): Promise<string[] | undefined> {
  const listing = await gitModifiedOrUntrackedListing(cwd, options);
  if (listing.ended === "timed-out") return undefined;
  return listing.ended === "printed" ? listing.paths : [];
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

/** How often a Git read looks at its time limit on the clock ({@link lookPastTimeLimit}). */
const TIME_LIMIT_TICK_MS = 1_000;

/** What a Git read has had so far: how many of Git's exit and its output's end. */
interface ReadProgress {
  ends: number;
}

/**
 * A Git read's time limit, asked at each look ({@link lookPastTimeLimit}) with what the read has had so far: whether it
 * may go on. It may while it has run less than `limitMs` on the clock, as a timer of `limitMs` allowed, however much of
 * that time the event loop stalled; past that, only once more after Git exited or its output ended since the last look,
 * since the read closes then. A Git still printing at that look ends there, as that timer ended it, and the read
 * answers that it ran late: a walk asks such a listing once more off its path (`CodeRuleBridge`).
 */
function timeLimit(limitMs: number): (progress: Readonly<ReadProgress>) => boolean {
  const startedAt = performance.now();
  let endsBefore = 0;
  return progress => {
    const ending = progress.ends > endsBefore;
    endsBefore = progress.ends;
    return ending || performance.now() - startedAt < limitMs;
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
  const mayGoOn = timeLimit(limitMs);
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
 * output; one a signal from elsewhere ended is waited for as one this ended. `timeoutMs` is time on the clock, however
 * much of it the event loop stalled, and a read past it goes on only once more after Git exited or its output ended
 * ({@link timeLimit}). `settle` is told whether `timeoutMs` or `signal` ended the read.
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
 * ({@link GitReadOptions}); without Git, or once `stream.signal` is aborted, it starts nothing. Run again with the Git
 * found again when the one kept is gone ({@link readWithGit}); a start that found no program printed nothing.
 */
async function streamGit(cwd: string, args: readonly string[], stream: GitStream): Promise<void> {
  await readWithGit({ timeoutMs: stream.timeout ?? DEFAULT_TIMEOUT_MS, signal: stream.signal }, async (executable, limits) => {
    const { promise, resolve: settle } = Promise.withResolvers<void>();
    const child = startGit(() => spawn(executable, [...SAFE_CONFIG, "-C", cwd, ...args], {
      env: { ...gitEnvironment(executable), GIT_TERMINAL_PROMPT: "0" },
      stdio: ["pipe", "pipe", "ignore"],
    }));
    if (child instanceof Error) return { answer: undefined, missing: false };
    const missing = startMissing(child);
    child.stdout.on("data", stream.onData);
    settleWhenDone(child, () => settle(), limits);
    // Git exits before reading every line when it cannot start in `cwd`; what it answered before that is kept.
    child.stdin.on("error", () => undefined);
    child.stdin.end(stream.input);
    await Promise.all([promise, exited(child)]);
    return { answer: undefined, missing: missing() };
  });
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
 * Reads something of the index under a folder, for a caller that asks every pass: it starts a Git process only when the
 * folder's index file changed since the last read, and otherwise costs one `stat` (plus one Git process the first time,
 * to find the file). Git replaces that file with a new one on every write, so a write changes its inode even inside one
 * clock tick. An index rewritten with nothing `same` changed, as `git status` does when it refreshes the files' stat
 * data, answers with the value it held: the very object, so a caller that compares answers by identity sees no change.
 * A folder whose file cannot be found or stamped is read every time, a new answer each time, so this can cost more,
 * never answer stale. When Git cannot answer (it failed, timed out or was stopped), the answer is undefined and nothing
 * is kept, so the next read asks Git again.
 */
function gitIndexReader<T>(
  read: (cwd: string, signal: AbortSignal | undefined) => Promise<T | undefined>,
  same: (held: T, now: T) => boolean,
): (cwd: string, options?: GitReadOptions) => Promise<T | undefined> {
  const indexes = new Map<string, string>();
  const held = new Map<string, { readonly stamp: string; readonly value: T }>();
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
    if (last !== undefined && last.stamp === stamp) return last.value;
    held.delete(cwd);
    const now = await read(cwd, options.signal);
    if (now === undefined || stamp === undefined) return now;
    const value = last !== undefined && same(last.value, now) ? last.value : now;
    held.set(cwd, { stamp, value });
    return value;
  };
}

/** The index's unmerged entries under a folder ({@link unmergedEntries}), read as {@link gitIndexReader} reads. */
export function gitUnmergedReader(): (cwd: string, options?: GitReadOptions) => Promise<ReadonlyMap<string, string> | undefined> {
  return gitIndexReader(unmergedEntries, (held, now) => held.size === now.size && [...now].every(([path, entry]) => held.get(path) === entry));
}

/**
 * The files the index under a folder tracks though Git's ignore rules would ignore them, as `git add --force` makes
 * them, `\0`-separated in Git's order; read as {@link gitIndexReader} reads. A path the index tracks is never ignored, so
 * a walk that left such a file out reads it once this list names it.
 */
export function gitTrackedIgnoredReader(): (cwd: string, options?: GitReadOptions) => Promise<string | undefined> {
  return gitIndexReader(
    (cwd, signal) => gitStdout(cwd, ["ls-files", "--cached", "--ignored", "--exclude-standard", "-z"], signal, MAX_LISTED_PATH_BYTES),
    (held, now) => held === now,
  );
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
