/**
 * Which Git checkout holds a file: the nearest folder above it that holds a `.git` entry and that Git
 * confirms as a checkout's top level, found the same way from the file's lexical and real paths. Whether that
 * checkout judges the file with its own rules, or leaves it to the checkout around it, is the router's decision.
 */
import { lstatSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { physicalPath } from "./checkout-control.js";
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

function within(root: string, candidate: string): boolean {
  const path = relative(root, candidate);
  return path === "" || (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`));
}

/** The entry `name` of the folder `owner` names: in the same checkout, one level down. */
function ownerWithin(owner: CheckoutOwner, name: string): CheckoutOwner {
  return { checkoutRoot: owner.checkoutRoot, path: owner.path === "" ? name : `${owner.path}/${name}` };
}

/**
 * What a walk found of a folder (`CheckoutOwners.walk`): the Git checkout that holds it, if one does, and the checkout
 * that owns it, which is that one or else a recorded checkout outside Git.
 */
interface WalkedFolder {
  readonly inGit: CheckoutOwner | undefined;
  readonly owner: CheckoutOwner | undefined;
}

/**
 * Resolves owning checkouts for one runtime. Each folder's `.git` entry is looked at once, and each candidate root
 * is confirmed once with Git, so a lookup costs a map access per file once its folders are known. When
 * `recordedVersion` is given, every answer is forgotten once it changes: a checkout Rerail records later (a folder that
 * became a checkout) then owns its files from the next lookup, without a restart. A walk (`walk`) works each folder's
 * checkout out again from the folder above it.
 */
export class CheckoutOwners {
  readonly #recordedVersion: (() => string) | undefined;
  #version: string | undefined;
  /** Lexical folder → physical root of the checkout that owns it (undefined: no Git checkout above it). */
  readonly #roots = new Map<string, string | undefined>();
  /**
   * Lexical folder that held a `.git` entry when Git was asked about it → its physical root when Git confirms it as a
   * top level. A folder with no entry has no answer here, so a walk tells from it an entry that came or went (`walk`).
   */
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
   * `ownerOf` for one walk, which asks it only of real paths: the folders it starts from, and folders it reached by
   * listing the folder above, never through a link. A folder's checkout is then its own when it holds a `.git` entry
   * Git confirms, and else the one of the folder above it, so a walk resolves no real path: it looks once at the `.git`
   * entry of each folder it asks of, and of each folder above those. Each walk looks again, so it finds a folder that
   * became a checkout or stopped being one wherever it is, with no record to say so; where it finds an entry came or
   * went, it forgets what `ownerOf` kept at and below that folder, and a tool call's lookup agrees with it. The recorded
   * checkouts' version is read once, here, not at each lookup.
   */
  walk(): (folder: string) => CheckoutOwner | undefined {
    this.#revalidate();
    const found = new Map<string, WalkedFolder>();
    const find = (folder: string): WalkedFolder => {
      let known = found.get(folder);
      if (known === undefined) {
        const parent = dirname(folder);
        known = this.#walkedFolder(folder, parent === folder ? undefined : find(parent));
        found.set(folder, known);
      }
      return known;
    };
    return folder => find(resolve(folder)).owner;
  }

  /** What a walk finds of the real folder `folder`, below the folder it found `above` (none for a filesystem root). */
  #walkedFolder(folder: string, above: WalkedFolder | undefined): WalkedFolder {
    const entry = holdsGitEntry(folder);
    // Git was asked about each folder that held an entry then: one that came or went since may have made a checkout or
    // ended one, and what was kept at and below the folder rests on that.
    if (entry !== this.#confirmed.has(folder)) this.#forgetAt(folder);
    const root = entry ? this.#confirm(folder) : undefined;
    const inGit = root === undefined ? above?.inGit && ownerWithin(above.inGit, basename(folder)) : { checkoutRoot: root, path: "" };
    if (inGit !== undefined) return { inGit, owner: inGit };
    const plain = this.#plainRoot(folder);
    return { inGit, owner: plain === undefined ? undefined : { checkoutRoot: plain, path: relative(plain, folder).split(sep).join("/") } };
  }

  /**
   * Forgets the answers that rest on the `.git` entry of the real folder `folder`, which came or went: Git's answer for
   * it, which checkout holds it and each folder below it, and whether the checkout around each checkout below it
   * declares that one a submodule. A folder below it keeps Git's answer of whether it is a checkout itself, which rests
   * on its own entry, and a folder outside it keeps every answer.
   */
  #forgetAt(folder: string): void {
    this.#confirmed.delete(folder);
    for (const answers of [this.#roots, this.#submodules]) {
      for (const key of answers.keys()) if (within(folder, key)) answers.delete(key);
    }
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

  /** Git's answer of whether `directory` is a checkout's top level, asked once while it holds a `.git` entry. */
  #confirm(directory: string): string | undefined {
    if (this.#confirmed.has(directory)) return this.#confirmed.get(directory);
    // A folder with no entry is no top level; no answer is kept for it, so `#confirmed` holds the folders that had one.
    if (!holdsGitEntry(directory)) return undefined;
    const location = repositoryIdentityLocation(directory);
    const top = location.git ? physicalPath(location.workspaceRoot) : undefined;
    const root = top !== undefined && top === physicalPath(directory) ? top : undefined;
    this.#confirmed.set(directory, root);
    return root;
  }
}
