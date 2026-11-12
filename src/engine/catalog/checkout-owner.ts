/**
 * Which Git checkout holds a file: the nearest folder above it that holds a `.git` entry and that Git
 * confirms as a checkout's top level, found the same way from the file's lexical and real paths. Whether that
 * checkout judges the file with its own rules, or leaves it to the checkout around it, is the router's decision.
 */
import { existsSync, lstatSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { physicalPath } from "./checkout-control.js";
import { runGit } from "../git.js";
import { repositoryIdentityLocation } from "./repository-identity.js";

export interface CheckoutOwner {
  /** Physical (symlink-resolved) top-level folder of the owning checkout. */
  readonly checkoutRoot: string;
  /** The file's real path relative to `checkoutRoot`, POSIX; `""` for the root itself. */
  readonly path: string;
}

/** Whether `directory` holds a `.git` file or folder: a candidate checkout top level, until Git confirms it. */
function holdsGitEntry(directory: string): boolean {
  try {
    lstatSync(join(directory, ".git"));
    return true;
  } catch {
    return false;
  }
}

/** Git's confirmation that a folder is a checkout's top level, with what Git read to give it (`gitBasis`). */
interface Confirmation {
  readonly root: string;
  readonly basis: string;
}

/**
 * What Git reads to take `directory` for a checkout's top level, as one string to compare later: the Git directory its
 * `.git` entry leads to (the entry itself, or the folder a `.git` file's `gitdir:` line names, relative to `directory`),
 * the common directory, the owner of the folder, the entry and the Git directory (Git refuses a folder another user
 * owns), and the repository's configuration files, which hold its format version, extensions and `core.worktree`.
 * Undefined when Git would not take the folder, as for a linked worktree whose entry under the main checkout's
 * `.git/worktrees` was removed or emptied, or a folder whose `.git` folder was emptied (`isGitDirectory`).
 */
function gitBasis(directory: string): string | undefined {
  const entry = join(directory, ".git");
  try {
    const gitDirectory = statSync(entry).isDirectory() ? entry : gitDirectoryNamedBy(directory, entry);
    if (gitDirectory === undefined) return undefined;
    const common = commonDirectoryOf(gitDirectory);
    if (!isGitDirectory(gitDirectory, common)) return undefined;
    const owners = [directory, entry, gitDirectory].map(path => statSync(path).uid);
    return [gitDirectory, common, ...owners, changeStamp(join(common, "config")), changeStamp(join(gitDirectory, "config.worktree"))].join("\0");
  } catch {
    return undefined;
  }
}

/** The Git directory a `.git` file names on its `gitdir:` line, relative to the folder that holds it. */
function gitDirectoryNamedBy(directory: string, entry: string): string | undefined {
  const line = readFileSync(entry, "utf8").split("\n", 1)[0]!.trimEnd();
  return line.startsWith("gitdir: ") ? resolve(directory, line.slice("gitdir: ".length)) : undefined;
}

/** The folder a Git directory's `commondir` file names, relative to it, else the Git directory itself. */
function commonDirectoryOf(gitDirectory: string): string {
  const commondir = join(gitDirectory, "commondir");
  return existsSync(commondir) ? resolve(gitDirectory, readFileSync(commondir, "utf8").replace(/[\r\n]+$/u, "")) : gitDirectory;
}

/** A HEAD Git accepts (`validate_headref`): a symbolic ref under `refs/`, or an object name. */
const GIT_HEAD = /^(?:ref:\s*refs\/|[0-9a-f]{40})/iu;

/**
 * Whether `folder` is a Git directory by the test Git applies before it takes a folder for one (`is_git_directory` in
 * Git's setup.c): its HEAD names a ref or an object, and its common directory holds `objects` and `refs`. Throws when
 * HEAD cannot be read.
 */
function isGitDirectory(folder: string, common: string): boolean {
  return GIT_HEAD.test(readFileSync(join(folder, "HEAD"), "utf8")) && existsSync(join(common, "objects")) && existsSync(join(common, "refs"));
}

/** `path`'s modified time and size, or `-` when it does not exist: changes whenever the file is written. */
function changeStamp(path: string): string {
  try {
    const info = statSync(path);
    return `${info.mtimeMs}:${info.size}`;
  } catch {
    return "-";
  }
}

/**
 * The change stamps of the user's Git configuration files, where `safe.directory` (which other users' folders Git
 * takes) is set: `GIT_CONFIG_GLOBAL` when set, else `~/.gitconfig` and the XDG `git/config`.
 */
function userGitConfigStamp(): string {
  const home = process.env.HOME ?? homedir();
  const files = process.env.GIT_CONFIG_GLOBAL === undefined
    ? [join(home, ".gitconfig"), join(process.env.XDG_CONFIG_HOME ?? join(home, ".config"), "git", "config")]
    : [process.env.GIT_CONFIG_GLOBAL];
  return files.map(changeStamp).join("\0");
}

/** Git's confirmation of `directory` as a checkout's top level, or undefined when Git does not take it for one. */
function gitConfirmation(directory: string): Confirmation | undefined {
  // Read before Git is asked: a change between the two then makes the next check ask Git again.
  const basis = gitBasis(directory) ?? "";
  const location = repositoryIdentityLocation(directory);
  const top = location.git ? physicalPath(location.workspaceRoot) : undefined;
  return top !== undefined && top === physicalPath(directory) ? { root: top, basis } : undefined;
}

function within(root: string, candidate: string): boolean {
  const path = relative(root, candidate);
  return path === "" || (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`));
}

/**
 * Resolves owning checkouts for one runtime. Each folder's `.git` entry is looked at once, and each candidate root
 * is confirmed once with Git, so a lookup costs a map access per file once its folders are known. When
 * `recordedVersion` is given, every answer is forgotten once it changes: a checkout Rerail records later (a folder that
 * became a checkout) then owns its files from the next lookup, without a restart.
 */
export class CheckoutOwners {
  readonly #recordedVersion: (() => string) | undefined;
  #version: string | undefined;
  /** Lexical folder → physical root of the checkout that owns it (undefined: no Git checkout above it). */
  readonly #roots = new Map<string, string | undefined>();
  /** Lexical folder → Git's confirmation of it as a top level when it holds `.git` (undefined: Git does not confirm it). */
  readonly #confirmed = new Map<string, Confirmation | undefined>();
  /** `userGitConfigStamp` at the last check of the confirmations. */
  #userConfig: string | undefined;
  /** Physical checkout root → whether the checkout around it declares it as a submodule (a gitlink). */
  readonly #submodules = new Map<string, boolean>();
  /** Recorded checkouts outside Git (identity under `.rerail`), physical. */
  readonly #plainRoots = new Set<string>();

  /** `recordedVersion` changes whenever the set of recorded checkouts does (see `CheckoutCatalogs.recordedCheckoutsVersion`). */
  constructor(recordedVersion?: () => string) {
    this.#recordedVersion = recordedVersion;
  }

  /** A recorded checkout outside Git owns the files below it that no Git checkout owns. */
  addPlainRoot(root: string): void {
    this.#plainRoots.add(physicalPath(root));
  }

  /**
   * The checkout holding `absolute`, found from its lexical path and again from its real path: undefined when the two
   * differ (a symlink into another checkout) or when the real path leaves the owner (a symlink escaping it).
   */
  ownerOf(absolute: string): CheckoutOwner | undefined {
    this.#revalidate();
    const lexical = resolve(absolute);
    const real = physicalPath(lexical);
    const root = this.#rootOf(lexical, real);
    if (root === undefined || !within(root, real)) return undefined;
    if (real !== lexical && this.#rootOf(real, real) !== root) return undefined;
    return { checkoutRoot: root, path: relative(root, real).split(sep).join("/") };
  }

  /**
   * Whether the checkout at `root` (a physical checkout top level) is a submodule the checkout around it declares: a
   * gitlink in that checkout's index. Git runs only in the enclosing checkout, once per root.
   */
  isDeclaredSubmodule(root: string): boolean {
    this.#revalidate();
    let declared = this.#submodules.get(root);
    if (declared === undefined) {
      const enclosing = this.ownerOf(dirname(root));
      declared = enclosing !== undefined && this.declaredSubmodules(enclosing.checkoutRoot).includes(root);
      this.#submodules.set(root, declared);
    }
    return declared;
  }

  /**
   * The submodules the checkout at `root` declares that are checked out: each `.gitmodules` path that its index holds
   * as a gitlink and that Git confirms as a checkout top level. Physical roots.
   */
  declaredSubmodules(root: string): string[] {
    // Git reads the file from the checkout's folder; without it there is nothing to ask, and a Git process on every tool
    // call (the session's ignore rules ask this of its checkout at each one) costs tens of milliseconds on Windows.
    if (!existsSync(join(root, ".gitmodules"))) return [];
    const configured = runGit(root, ["config", "--file", ".gitmodules", "--null", "--get-regexp", String.raw`^submodule\..*\.path$`]);
    if (configured.status !== 0) return [];
    const paths = configured.stdout.split("\0").filter(Boolean).map(entry => entry.slice(entry.indexOf("\n") + 1));
    if (paths.length === 0) return [];
    const index = runGit(root, ["--literal-pathspecs", "ls-files", "--stage", "-z", "--", ...paths]);
    if (index.status !== 0) return [];
    return index.stdout.split("\0")
      .filter(entry => entry.startsWith("160000 "))
      .map(entry => join(root, entry.slice(entry.indexOf("\t") + 1)))
      .filter(folder => within(root, folder) && this.#confirm(folder) === physicalPath(folder));
  }

  /** The checkout root above `path` (the path itself when it is one), searched from `path`; `real` finds plain roots. */
  #rootOf(path: string, real: string): string | undefined {
    // Only folders are remembered: the path itself is a checkout root only when it holds `.git`.
    const own = holdsGitEntry(path) ? this.#confirm(path) : undefined;
    return own ?? this.#gitRoot(dirname(path)) ?? this.#plainRoot(real);
  }

  /**
   * Forget the cached answers when the recorded checkouts changed since the last lookup, but for each folder Git
   * confirmed as a top level while what Git read to confirm it reads the same (`gitBasis`, and the user's Git
   * configuration): that is Git's answer, which no record changes, and asking it again costs a Git process per checkout
   * the session knows each time any checkout is recorded or forgotten. The check costs a few reads and stats per folder
   * kept, still no Git process, and any change asks Git again.
   */
  #revalidate(): void {
    if (this.#recordedVersion === undefined) return;
    const version = this.#recordedVersion();
    if (version === this.#version) return;
    this.#version = version;
    this.#roots.clear();
    this.#submodules.clear();
    this.#forgetChangedConfirmations();
  }

  #forgetChangedConfirmations(): void {
    const userConfig = userGitConfigStamp();
    if (userConfig !== this.#userConfig) this.#confirmed.clear();
    this.#userConfig = userConfig;
    for (const [directory, confirmed] of this.#confirmed) {
      if (confirmed === undefined || gitBasis(directory) !== confirmed.basis) this.#confirmed.delete(directory);
    }
  }

  #plainRoot(real: string): string | undefined {
    let owner: string | undefined;
    for (const root of this.#plainRoots) {
      if (within(root, real) && (owner === undefined || root.length > owner.length)) owner = root;
    }
    return owner;
  }

  #gitRoot(lexical: string): string | undefined {
    const unresolved: string[] = [];
    let directory = lexical;
    let root: string | undefined;
    for (;;) {
      if (this.#roots.has(directory)) {
        root = this.#roots.get(directory);
        break;
      }
      unresolved.push(directory);
      root = this.#confirm(directory);
      if (root !== undefined) break;
      const parent = dirname(directory);
      if (parent === directory) break;
      directory = parent;
    }
    for (const folder of unresolved) this.#roots.set(folder, root);
    return root;
  }

  #confirm(directory: string): string | undefined {
    if (this.#confirmed.has(directory)) return this.#confirmed.get(directory)?.root;
    const confirmation = holdsGitEntry(directory) ? gitConfirmation(directory) : undefined;
    this.#confirmed.set(directory, confirmation);
    return confirmation?.root;
  }
}
