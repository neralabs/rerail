/**
 * Which Git checkout holds a file: the nearest folder above it that holds a `.git` entry and that Git
 * confirms as a checkout's top level, found the same way from the file's lexical and real paths. Whether that
 * checkout judges the file with its own rules, or leaves it to the checkout around it, is the router's decision.
 */
import { existsSync, lstatSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { physicalPath } from "./checkout-control.js";
import type { RerailDatabase } from "../storage/database.js";
import { recordedCheckoutsVersionReader, recordedPlainRoot } from "./repository-registry.js";
import { runGit } from "../git.js";
import { repositoryIdentityLocation } from "./repository-identity.js";

export interface CheckoutOwner {
  /** Physical (symlink-resolved) top-level folder of the owning checkout. */
  readonly checkoutRoot: string;
  /** The file's real path relative to `checkoutRoot`, POSIX; `""` for the root itself. */
  readonly path: string;
}

/**
 * Whether `directory` holds a `.git` file or folder: a candidate checkout top level, until Git confirms it. Asked of
 * the path itself on every lookup, where the answer is mostly no: a missing entry returns, since a thrown error cost
 * twice the stat.
 */
function holdsGitEntry(directory: string): boolean {
  try {
    return lstatSync(join(directory, ".git"), { throwIfNoEntry: false }) !== undefined;
  } catch {
    return false;
  }
}

/** Whether `directory` may hold an entry called `name`: true when it does, or when the lstat cannot tell. */
function holdsEntry(directory: string, name: string): boolean {
  try {
    return lstatSync(join(directory, name), { throwIfNoEntry: false }) !== undefined;
  } catch {
    return true;
  }
}

/** A `.git` entry as it was on disk when Git was asked about it. */
interface GitEntry {
  /** The Git folder a `.git` file names; undefined for a `.git` folder, or a file that names none. */
  readonly gitFolder: string | undefined;
  /** {@link gitEntryStamp} read before Git was asked. */
  readonly stamp: string;
}

/**
 * The `.git` entry `directory` holds, read before Git is asked about it so that a change Git has not seen shows; undefined
 * when it holds none. Reads a `.git` file for the Git folder it names.
 */
function gitEntryAt(directory: string): GitEntry | undefined {
  const entry = join(directory, ".git");
  let file: boolean;
  try {
    const stats = lstatSync(entry, { throwIfNoEntry: false });
    if (stats === undefined) return undefined;
    file = stats.isFile();
  } catch {
    return undefined;
  }
  let gitFolder: string | undefined;
  if (file) {
    try {
      // A `.git` file holds `gitdir: <path>`, relative to the folder or absolute.
      const named = /^gitdir: (.+)/u.exec(readFileSync(entry, "utf8"))?.[1]?.trim();
      gitFolder = named === undefined ? undefined : resolve(directory, named);
    } catch {
      gitFolder = undefined;
    }
  }
  return { gitFolder, stamp: gitEntryStamp(directory, gitFolder) };
}

/**
 * The lstat of the `.git` entry at `directory` and of the Git folder it names, in one string that changes when either
 * does. Git writes a worktree's `.git` file before the `HEAD` in the Git folder that file names, and `git init` its
 * `.git` folder before the `HEAD` in it; a lookup between the two finds no checkout, and the `HEAD` that completes it
 * changes the folder it lands in.
 */
function gitEntryStamp(directory: string, gitFolder: string | undefined): string {
  const entry = lstatStamp(join(directory, ".git"));
  return gitFolder === undefined ? entry : `${entry} ${lstatStamp(gitFolder)}`;
}

/**
 * What a confirmed checkout's `.git` entry is, in one string that changes when the entry, or the Git folder a `.git`
 * file names, is moved away, replaced or rewritten: their identity, not their times, so Git writing in the Git folder
 * (an index, a ref) leaves it as it was.
 */
function gitEntryIdentity(directory: string, gitFolder: string | undefined): string {
  const entry = join(directory, ".git");
  // A `.git` file is stamped whole (the Git folder it names is in it), the Git folder it names by its identity.
  return gitFolder === undefined ? identityStamp(entry) : `${lstatStamp(entry)} ${identityStamp(gitFolder)}`;
}

function identityStamp(path: string): string {
  try {
    const stats = lstatSync(path, { throwIfNoEntry: false });
    return stats === undefined ? "none" : `${stats.ino}:${stats.birthtimeMs}:${stats.isDirectory() ? "d" : "f"}`;
  } catch (error) {
    return error instanceof Error && "code" in error ? String(error.code) : "unreadable";
  }
}

function lstatStamp(path: string): string {
  try {
    const stats = lstatSync(path, { throwIfNoEntry: false });
    return stats === undefined ? "none" : `${stats.ino}:${stats.size}:${stats.mtimeMs}:${stats.ctimeMs}`;
  } catch (error) {
    return error instanceof Error && "code" in error ? String(error.code) : "unreadable";
  }
}

function within(root: string, candidate: string): boolean {
  const path = relative(root, candidate);
  return path === "" || (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`));
}

/**
 * Resolves owning checkouts for one runtime. Each folder's `.git` entry is looked at once, and each candidate root
 * is confirmed once with Git, so a lookup costs a map access per file once its folders are known. The answers are
 * the file system as it was at the last point (`lookAgain`): a lookup between two points takes no change, and the next
 * point looks again at everything kept, so after it every answer is the disk's at that point. When the catalog
 * is given, every answer is forgotten once its recorded checkouts change: a checkout Rerail records later (a folder that
 * became a checkout) then owns its files from the next lookup, without a restart. A recorded checkout outside Git owns
 * the files below it that no Git checkout owns while the catalog records it and its `.rerail` marker names it.
 */
export class CheckoutOwners {
  readonly #recordedVersion: (() => string) | undefined;
  #version: string | undefined;
  /** Lexical folder → physical root of the checkout that owns it (undefined: no Git checkout above it). */
  readonly #roots = new Map<string, string | undefined>();
  /** Lexical folder → its physical root when it holds `.git` and Git confirms it as a top level. */
  readonly #confirmed = new Map<string, string | undefined>();
  /**
   * Physical checkout root → whether the checkout around it declares it as a submodule (a gitlink), with the
   * enclosing checkout and the stamp of its `.gitmodules` and Git folder it was read under (`submoduleStamp`): a point
   * that finds either changed asks Git again.
   */
  readonly #submodules = new Map<string, { declared: boolean; enclosing: string; stamp: string }>();
  /**
   * The folders `#confirm` found holding a `.git` entry that Git did not take for a top level (one naming a Git folder
   * that is gone, or one Git is still making), told apart from those that held none, each with that entry as it was:
   * `lookAgain` asks Git again only once the entry, or the Git folder it names, changed, so such an entry costs one
   * Git process, not one per write below it, and a worktree looked up before Git finished making it is found once Git
   * has. `#confirm` keeps it in step for every folder `#confirmed` holds.
   */
  readonly #unconfirmedGitEntries = new Map<string, GitEntry>();
  /** The folders Git confirmed as a checkout's top level, each with its `.git` entry's identity (`gitEntryIdentity`). */
  readonly #confirmedGitEntries = new Map<string, string>();
  readonly #database: RerailDatabase | undefined;

  /** `database`'s recorded checkouts, read for plain roots and for the version that forgets every answer. */
  constructor(database?: RerailDatabase) {
    this.#database = database;
    this.#recordedVersion = database === undefined ? undefined : recordedCheckoutsVersionReader(database);
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
   * The point after which every answer is the file system's as it is now, and until the next one no change is taken.
   * Drops the answers kept at and below each folder that changed since a lookup asked: one found with no `.git` entry
   * that holds one now (`git worktree add`, from any repository, or `git init`), one whose `.git` entry Git did not take
   * for a top level that changed since (a lookup can land between Git writing the `.git` entry and the `HEAD` that makes
   * it one), and a confirmed checkout whose `.git` entry was moved away or replaced; and each submodule answer whose
   * enclosing `.gitmodules` or Git folder changed (a gitlink staged or removed, the shared index moved). Costs a stat per
   * folder kept as plain, one to three per `.git` entry kept, and two per submodule answer; Git runs only for what
   * changed, at the next lookup. Returns whether it dropped any.
   */
  lookAgain(): boolean {
    this.#revalidate();
    const outgrown = [...this.#confirmed.keys()].filter(folder => this.#outgrown(folder));
    // The highest of each changed chain: dropping it drops the answers below it.
    const highest = outgrown.filter(folder => !outgrown.some(other => other !== folder && within(other, folder)));
    for (const folder of highest) this.#forgetWithin(folder);
    let dropped = highest.length > 0;
    for (const [root, kept] of this.#submodules) {
      if (submoduleStamp(kept.enclosing) === kept.stamp) continue;
      this.#submodules.delete(root);
      dropped = true;
    }
    return dropped;
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
   * gitlink in that checkout's index and a path in its `.gitmodules`, as they were at the last point (`lookAgain`). Git
   * runs only in the enclosing checkout, once per root between the points that find them changed.
   */
  isDeclaredSubmodule(root: string): boolean {
    this.#revalidate();
    const kept = this.#submodules.get(root);
    if (kept !== undefined) return kept.declared;
    const enclosing = this.ownerOf(dirname(root));
    if (enclosing === undefined) return false;
    // Stamped before Git reads them, so a change Git has not seen shows at the next point.
    const stamp = submoduleStamp(enclosing.checkoutRoot);
    const declared = this.declaredSubmodules(enclosing.checkoutRoot).includes(root);
    this.#submodules.set(root, { declared, enclosing: enclosing.checkoutRoot, stamp });
    return declared;
  }

  /**
   * The submodules the checkout at `root` declares that are checked out: each `.gitmodules` path that its index holds
   * as a gitlink and that Git confirms as a checkout top level. Physical roots. A checkout with no `.gitmodules` entry
   * declares none, which Git would answer by reading nothing, so Git is not started to say so.
   */
  declaredSubmodules(root: string): string[] {
    if (!holdsEntry(root, ".gitmodules")) return [];
    const configured = runGit(root, ["config", "--file", ".gitmodules", "--null", "--get-regexp", String.raw`^submodule\..*\.path$`]);
    if (configured.status !== 0) return [];
    const paths = new Set(configured.stdout.split("\0").filter(Boolean).map(entry => entry.slice(entry.indexOf("\n") + 1)));
    if (paths.size === 0) return [];
    const index = runGit(root, ["--literal-pathspecs", "ls-files", "--stage", "-z", "--", ...paths]);
    if (index.status !== 0) return [];
    return index.stdout.split("\0")
      .filter(entry => entry.startsWith("160000 "))
      .map(entry => entry.slice(entry.indexOf("\t") + 1))
      .filter(path => paths.has(path))
      .map(path => join(root, path))
      .filter(folder => within(root, folder) && this.#confirm(folder) === physicalPath(folder));
  }

  /** The checkout root above `path` (the path itself when it is one), searched from `path`; `real` finds plain roots. */
  #rootOf(path: string, real: string): string | undefined {
    // Only folders are remembered: the path itself is a checkout root only when it holds `.git` (as kept, once a lookup
    // asked: a change is the next point's).
    const own = this.#confirmed.has(path) || holdsGitEntry(path) ? this.#confirm(path) : undefined;
    return own ?? this.#gitRoot(dirname(path)) ?? (this.#database && recordedPlainRoot(this.#database, real));
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
    this.#confirmedGitEntries.clear();
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
   * undefined. Kept until a point finds the entry changed (`lookAgain`): between points a lookup takes the kept answer.
   */
  #confirm(directory: string): string | undefined {
    if (this.#confirmed.has(directory)) return this.#confirmed.get(directory);
    const gitEntry = gitEntryAt(directory);
    const root = gitEntry === undefined ? undefined : topLevelAt(directory);
    this.#confirmed.set(directory, root);
    if (gitEntry !== undefined && root === undefined) this.#unconfirmedGitEntries.set(directory, gitEntry);
    else this.#unconfirmedGitEntries.delete(directory);
    if (gitEntry !== undefined && root !== undefined) this.#confirmedGitEntries.set(directory, gitEntryIdentity(directory, gitEntry.gitFolder));
    else this.#confirmedGitEntries.delete(directory);
    return root;
  }

  /**
   * Whether Git may answer for `folder` differently than when a lookup asked: it held no `.git` entry and holds one now,
   * its `.git` entry Git did not take for a top level, or the Git folder it names, changed, or Git confirmed it and its
   * `.git` entry, or the Git folder it names, was moved away or replaced. A stat for a folder kept as plain, one to
   * three for a kept `.git` entry.
   */
  #outgrown(folder: string): boolean {
    const unconfirmed = this.#unconfirmedGitEntries.get(folder);
    if (unconfirmed !== undefined) return gitEntryStamp(folder, unconfirmed.gitFolder) !== unconfirmed.stamp;
    const confirmed = this.#confirmedGitEntries.get(folder);
    if (confirmed !== undefined) return gitEntryIdentity(folder, gitEntryAt(folder)?.gitFolder) !== confirmed;
    return this.#keptAsPlain(folder) && holdsGitEntry(folder);
  }

  /** Whether the kept answer for `folder` is that it held no `.git` entry. */
  #keptAsPlain(folder: string): boolean {
    return this.#confirmed.get(folder) === undefined && this.#confirmed.has(folder) && !this.#unconfirmedGitEntries.has(folder);
  }

  /** Drops every answer kept at and below `folder`. */
  #forgetWithin(folder: string): void {
    for (const answers of [this.#roots, this.#confirmed, this.#submodules, this.#unconfirmedGitEntries, this.#confirmedGitEntries]) {
      for (const kept of answers.keys()) if (within(folder, kept)) answers.delete(kept);
    }
  }
}

/** The physical top level of the checkout whose `.git` entry `directory` holds, when Git takes `directory` for it. */
function topLevelAt(directory: string): string | undefined {
  const location = repositoryIdentityLocation(directory);
  const top = location.git ? physicalPath(location.workspaceRoot) : undefined;
  return top !== undefined && top === physicalPath(directory) ? top : undefined;
}

/**
 * What a checkout's submodule declarations were read from, in one string that changes when they may: its `.gitmodules`,
 * and its Git folder, which Git writes a new index into (and a split index's shared one) by renaming it in.
 */
function submoduleStamp(root: string): string {
  const gitFolder = gitEntryAt(root)?.gitFolder ?? join(root, ".git");
  return `${lstatStamp(join(root, ".gitmodules"))} ${lstatStamp(gitFolder)}`;
}
