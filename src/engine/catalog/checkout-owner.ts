/**
 * Which Git checkout holds a file: the nearest folder above it that holds a `.git` entry and that Git
 * confirms as a checkout's top level, found the same way from the file's lexical and real paths. Whether that
 * checkout judges the file with its own rules, or leaves it to the checkout around it, is the router's decision.
 */
import { lstatSync } from "node:fs";
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
  /** Lexical folder → its physical root when it holds `.git` and Git confirms it as a top level. */
  readonly #confirmed = new Map<string, string | undefined>();
  /** Physical checkout root → whether the checkout around it declares it as a submodule (a gitlink). */
  readonly #submodules = new Map<string, boolean>();
  /**
   * The folders `#confirm` found holding a `.git` entry that Git did not take for a top level (one naming a Git folder
   * that is gone, say), told apart from those that held none: `lookAgainAt` leaves them be, so such an entry costs one
   * Git process, not one per write below it. `#confirm` keeps it in step for every folder `#confirmed` holds.
   */
  readonly #unconfirmedGitEntries = new Set<string>();
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
   * Forgets every folder looked up so far but the checkout top levels Git confirmed: each of those took a Git process to
   * learn, and stays as long as it would have otherwise. A walk of a repository's other worktrees looks up every folder of
   * every worktree, so a runtime calls this once its last session has closed rather than keep them for as long as it
   * serves its folder.
   */
  forgetFolders(): void {
    this.#roots.clear();
    for (const [directory, root] of this.#confirmed) if (root === undefined) this.#confirmed.delete(directory);
    this.#unconfirmedGitEntries.clear();
  }

  /**
   * Drops the answers kept at and below each folder on the way up from `path` that a lookup found with no `.git` entry
   * and that holds one now, so the next lookup asks the disk and Git again. `git worktree add`, from any repository, or
   * `git init` can make a folder a session already looked up a checkout's top level; the answers from before name the
   * checkout around it, or none, for its files, and would stay until the recorded checkouts change. A lookup of such a
   * folder itself drops them on its own (`#confirm`), but a file's lookup reads the answers kept for the folders above
   * it without looking at them. Costs a stat per folder kept as plain on the way up from `path`, and from its real path
   * when that differs. Returns whether it dropped any.
   */
  lookAgainAt(path: string): boolean {
    this.#revalidate();
    const lexical = resolve(path);
    const real = physicalPath(lexical);
    const outgrown = (real === lexical ? [lexical] : [lexical, real])
      .map(start => this.#outgrownAbove(start))
      .filter((folder): folder is string => folder !== undefined);
    for (const folder of outgrown) this.#forgetWithin(folder);
    return outgrown.length > 0;
  }

  /** How many folders it holds an answer for. */
  cachedFolders(): number {
    return new Set([...this.#roots.keys(), ...this.#confirmed.keys()]).size;
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

  /** Forget every cached answer when the recorded checkouts changed since the last lookup. */
  #revalidate(): void {
    if (this.#recordedVersion === undefined) return;
    const version = this.#recordedVersion();
    if (version === this.#version) return;
    this.#version = version;
    this.#roots.clear();
    this.#confirmed.clear();
    this.#submodules.clear();
    this.#unconfirmedGitEntries.clear();
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

  /**
   * Whether `directory` holds a `.git` entry that Git confirms as a checkout's top level: its physical root, or
   * undefined. A folder kept as plain that holds a `.git` entry now became a checkout after a lookup passed it; the
   * answers kept at and below it predate that, so they go and Git is asked. A lookup asks this of the folder it starts
   * at only when that folder holds a `.git` entry, so a walk, which asks for each folder before the files in it, finds
   * such a checkout at its folder without a stat more.
   */
  #confirm(directory: string): string | undefined {
    if (this.#confirmed.has(directory)) {
      if (!this.#outgrown(directory)) return this.#confirmed.get(directory);
      this.#forgetWithin(directory);
    }
    const gitEntry = holdsGitEntry(directory);
    let root: string | undefined;
    if (gitEntry) {
      const location = repositoryIdentityLocation(directory);
      const top = location.git ? physicalPath(location.workspaceRoot) : undefined;
      if (top !== undefined && top === physicalPath(directory)) root = top;
    }
    this.#confirmed.set(directory, root);
    if (gitEntry && root === undefined) this.#unconfirmedGitEntries.add(directory);
    else this.#unconfirmedGitEntries.delete(directory);
    return root;
  }

  /** The highest folder from `path` up that a lookup found with no `.git` entry and that holds one now. */
  #outgrownAbove(path: string): string | undefined {
    let outgrown: string | undefined;
    for (let folder = path; ; folder = dirname(folder)) {
      if (this.#outgrown(folder)) outgrown = folder;
      if (dirname(folder) === folder) return outgrown;
    }
  }

  /** Whether a lookup found `folder` with no `.git` entry and it holds one now. A stat, only for a folder kept as plain. */
  #outgrown(folder: string): boolean {
    return this.#keptAsPlain(folder) && holdsGitEntry(folder);
  }

  /** Whether the kept answer for `folder` is that it held no `.git` entry. */
  #keptAsPlain(folder: string): boolean {
    return this.#confirmed.get(folder) === undefined && this.#confirmed.has(folder) && !this.#unconfirmedGitEntries.has(folder);
  }

  /** Drops every answer kept at and below `folder`. */
  #forgetWithin(folder: string): void {
    for (const answers of [this.#roots, this.#confirmed, this.#submodules, this.#unconfirmedGitEntries]) {
      for (const kept of answers.keys()) if (within(folder, kept)) answers.delete(kept);
    }
  }
}
