/**
 * Which Git checkout holds a file: the nearest folder above it that holds a `.git` entry and that Git
 * confirms as a checkout's top level, found the same way from the file's lexical and real paths. Whether that
 * checkout judges the file with its own rules, or leaves it to the checkout around it, is the router's decision.
 */
import { existsSync, lstatSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { physicalPath } from "./checkout-control.js";
import type { RerailDatabase } from "../storage/database.js";
import { recordedPlainRoot } from "./repository-registry.js";
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

/** Resolves the checkout boundary at the time a watched path is routed. */
export class CheckoutOwners {
  readonly #database: RerailDatabase | undefined;

  constructor(database?: RerailDatabase) {
    this.#database = database;
  }

  /**
   * The checkout holding `absolute`, found from its lexical path and again from its real path: undefined when the two
   * differ (a symlink into another checkout) or when the real path leaves the owner (a symlink escaping it).
   */
  ownerOf(absolute: string): CheckoutOwner | undefined {
    const lexical = resolve(absolute);
    const real = physicalPath(lexical);
    const root = this.#rootOf(lexical, real);
    if (root === undefined || !within(root, real)) return undefined;
    if (real !== lexical && this.#rootOf(real, real) !== root) return undefined;
    return { checkoutRoot: root, path: relative(root, real).split(sep).join("/") };
  }

  /**
   * Whether the checkout at `root` (a physical checkout top level) is a submodule the checkout around it declares: a
   * gitlink in that checkout's effective index and a path in its current `.gitmodules`.
   */
  isDeclaredSubmodule(root: string): boolean {
    const enclosing = this.ownerOf(dirname(root));
    return enclosing !== undefined && this.declaredSubmodules(enclosing.checkoutRoot).includes(root);
  }

  /**
   * The submodules the checkout at `root` declares that are checked out: each `.gitmodules` path that its index holds
   * as a gitlink and that Git confirms as a checkout top level. Physical roots.
   */
  declaredSubmodules(root: string): string[] {
    if (!existsSync(join(root, ".gitmodules"))) return [];
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
    for (let directory = path; ; directory = dirname(directory)) {
      const root = this.#confirm(directory);
      if (root !== undefined) return root;
      if (dirname(directory) === directory) break;
    }
    return this.#database && recordedPlainRoot(this.#database, real);
  }

  #confirm(directory: string): string | undefined {
    if (!holdsGitEntry(directory)) return undefined;
    const location = repositoryIdentityLocation(directory);
    const root = physicalPath(directory);
    return location.git && physicalPath(location.workspaceRoot) === root ? root : undefined;
  }
}
