/**
 * The checkout of each folder and file one walk found, for the router's walk view. A file the walk found is no link, so
 * its checkout is its folder's. A folder whose listing named no `.git` entry is no checkout's top level, so its checkout
 * is its parent's, one level down, unless a recorded checkout outside Git starts there: the walk asks the disk nothing
 * of it. Every other folder gets the full lookup once a walk.
 */
import { join } from "node:path";
import type { CheckoutOwner } from "../checkout-owner.js";

/** `owner`'s checkout for the entry `name` in the folder `owner` names. */
function below(owner: CheckoutOwner, name: string): CheckoutOwner {
  return { checkoutRoot: owner.checkoutRoot, path: owner.path === "" ? name : `${owner.path}/${name}` };
}

/** The folder holding `path` (session-relative, `/`-separated) and its name there; undefined for the walk's own top. */
function parentAndName(path: string): { readonly parent: string; readonly name: string } | undefined {
  if (path === "") return undefined;
  const slash = path.lastIndexOf("/");
  return slash < 0 ? { parent: "", name: path } : { parent: path.slice(0, slash), name: path.slice(slash + 1) };
}

export class WalkFolderOwners {
  /** Each folder's checkout for this walk: its full lookup, or its parent's when its listing named no `.git` entry. */
  readonly #folders = new Map<string, CheckoutOwner | undefined>();
  readonly #lookUp: (folder: string) => CheckoutOwner | undefined;
  readonly #plainRootAt: (real: string) => boolean;

  /**
   * `lookUp`: a folder's full lookup, which asks the disk. `plainRootAt`: whether a recorded checkout outside Git has its
   * top level at a physical folder (`CheckoutOwners.plainRootAt`).
   */
  constructor(lookUp: (folder: string) => CheckoutOwner | undefined, plainRootAt: (real: string) => boolean) {
    this.#lookUp = lookUp;
    this.#plainRootAt = plainRootAt;
  }

  /** The checkout of `folder` (session-relative, `/`-separated): the one kept for this walk, else its full lookup, kept. */
  folder(folder: string): CheckoutOwner | undefined {
    if (!this.#folders.has(folder)) this.#folders.set(folder, this.#lookUp(folder));
    return this.#folders.get(folder);
  }

  /** The checkout of the file `path`: its folder's. */
  file(path: string): CheckoutOwner | undefined {
    const place = parentAndName(path);
    if (place === undefined) return this.folder(path);
    const owner = this.folder(place.parent);
    return owner === undefined ? undefined : below(owner, place.name);
  }

  /**
   * The walk listed `folder`. One whose listing named a `.git` entry gets the full lookup, or keeps the one its parent's
   * listing made it ask (`ignored`); one whose listing named none takes its parent's checkout, the listing being newer
   * than any lookup of it.
   */
  listed(folder: string, holdsGitEntry: boolean): void {
    const parents = holdsGitEntry ? undefined : this.#parents(folder);
    if (parents === undefined) this.folder(folder);
    else this.#folders.set(folder, parents.owner);
  }

  /**
   * Whether `ignores` holds for the checkout of `folder`, a folder its parent's listing named, before the walk lists it.
   * Asked of its parent's checkout first: when that does not ignore it, neither does a checkout of its own, whose top
   * level no ignore rule names, nor the parent's, which a `.git` Git does not take for a top level leaves as its owner.
   * Only a folder it ignores gets the full lookup, which finds the checkout of its own that owns its files.
   */
  ignored(folder: string, ignores: (owner: CheckoutOwner | undefined) => boolean): boolean {
    const parents = this.#folders.has(folder) ? undefined : this.#parents(folder);
    if (parents !== undefined && !ignores(parents.owner)) return false;
    return ignores(this.folder(folder));
  }

  /**
   * `folder`'s checkout when it holds no `.git` entry: its parent's, one level down. Undefined where that does not hold
   * without a lookup: a parent not looked up in this walk, one no checkout holds (a recorded checkout outside Git may
   * start below it), or a recorded checkout outside Git at `folder` itself.
   */
  #parents(folder: string): { readonly owner: CheckoutOwner } | undefined {
    const place = parentAndName(folder);
    const parent = place === undefined ? undefined : this.#folders.get(place.parent);
    if (place === undefined || parent === undefined) return undefined;
    const owner = below(parent, place.name);
    return this.#plainRootAt(join(owner.checkoutRoot, owner.path)) ? undefined : { owner };
  }
}
