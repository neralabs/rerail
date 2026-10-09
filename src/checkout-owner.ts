/**
 * Which Git checkout holds a file: the nearest folder above it that holds a `.git` entry and that Git
 * confirms as a checkout's top level, found the same way from the file's lexical and real paths. Whether that
 * checkout judges the file with its own rules, or leaves it to the checkout around it, is the router's decision.
 */
import { lstatSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { physicalPath } from "./checkout-control.js";
import { runGit } from "./git.js";
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
    if (this.#confirmed.has(directory)) return this.#confirmed.get(directory);
    let root: string | undefined;
    if (holdsGitEntry(directory)) {
      const location = repositoryIdentityLocation(directory);
      const top = location.git ? physicalPath(location.workspaceRoot) : undefined;
      if (top !== undefined && top === physicalPath(directory)) root = top;
    }
    this.#confirmed.set(directory, root);
    return root;
  }
}
