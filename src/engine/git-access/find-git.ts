import { accessSync, constants, existsSync, realpathSync, statSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { posix, win32 } from "node:path";
import { spawnSync } from "../child-process.js";

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
 * Throws {@link GitUnavailableError} when no Git runs here ({@link gitExecutable}), for code that needs Git to be
 * installed and never starts it: only `git-process.ts` takes Git's path.
 */
export function requireGit(): void {
  gitExecutable();
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
