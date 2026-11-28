/**
 * The folders between a folder asked about and the top level of the checkout Git placed it in, which Git's search for a
 * repository passed on its way up, and whether one of them has since become a place where that search ends: `git init`
 * there, a clone or a submodule made at that path. Read with no Git process: a folder that held no `.git` entry costs
 * one lstat at each check; one whose `.git` entry Git's search passed over (an empty `.git` folder, say) is read as that
 * search reads it ({@link searchedIn}), so the entry filled in place counts too.
 */
import { storedForm } from "../checkout-control.js";
import { foldersGitSearches } from "./git-search.js";
import { holdsNoGitEntry, realOf, searchedIn, searchStep } from "./git-placement.js";

/** A folder Git's search passed below the top level, and what that search read there if it held a `.git` entry. */
export interface FolderBelowTop {
  readonly folder: string;
  /** What Git's search read in the folder ({@link searchedIn}); undefined where it held no `.git` entry. */
  readonly searched: string | undefined;
}

/**
 * Each folder Git's search reads from `requested`'s real path up to `top`, the top level Git named, `top` left out; all
 * of them, as far as that search goes, where it never reaches `top` (a `core.worktree` naming a folder elsewhere).
 * Undefined, so the next check reads the folder afresh, where the real path cannot be read, and where Git's search ends
 * in a folder below `top`: one that became a repository after Git placed `requested`, before this read.
 */
export function foldersBelowTop(requested: string, top: string): readonly FolderBelowTop[] | undefined {
  const real = realOf(requested);
  if (real === undefined) return undefined;
  const below: FolderBelowTop[] = [];
  for (const folder of foldersGitSearches(real)) {
    const read = belowOrTop(folder, top);
    if (read === "top") return below;
    if (read === undefined) return undefined;
    below.push(read);
  }
  return below;
}

/**
 * `folder` as a folder below `top`; `"top"` where it is `top`, as written or as the disk spells both ({@link storedForm});
 * undefined where Git's search ends in it below `top`.
 */
function belowOrTop(folder: string, top: string): FolderBelowTop | "top" | undefined {
  if (folder === top) return "top";
  if (holdsNoGitEntry(folder)) return { folder, searched: undefined };
  const step = searchStep(folder);
  if (!step.ends) return { folder, searched: step.entries };
  return storedForm(folder) === storedForm(top) ? "top" : undefined;
}

/**
 * Whether no folder of `below` became a place where Git's search ends: each that held no `.git` entry still holds none,
 * and each that held one reads as it did. False where `below` could not be read.
 */
export function noneBecameRepository(below: readonly FolderBelowTop[] | undefined): boolean {
  return below !== undefined && below.every(({ folder, searched }) => (searched === undefined ? holdsNoGitEntry(folder) : searchedIn(folder) === searched));
}
