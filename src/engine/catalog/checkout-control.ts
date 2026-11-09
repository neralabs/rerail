import { existsSync, realpathSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

/** True when `candidate` is `root` or below it; canonicalize both first when symlinked identity matters. */
export function withinCheckout(root: string, candidate: string): boolean {
  const path = relative(root, resolve(candidate));
  return !isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`);
}

/** The realpath checkout keys go through on `platform` (see {@link checkoutRealpath}). */
function realpathOn(platform: NodeJS.Platform): (path: string) => string {
  return platform === "win32" ? realpathSync.native : realpathSync;
}

/**
 * The realpath of `path` every checkout key goes through. On Windows it is the native one, which spells each name as
 * stored and the drive letter in upper case, as Git prints a top level: the JS realpath keeps the caller's spelling, so
 * a session opened at `c:\work\app` (VS Code's spelling) never matched the checkout Git names `C:/work/app`, and no
 * rule ran in it. Elsewhere it is the JS realpath, which keeps the case a macOS path was typed in (see
 * {@link storedPath}).
 */
export function checkoutRealpath(path: string): string {
  return realpathOn(process.platform)(path);
}

/**
 * The canonical location of `path`: its deepest existing ancestor resolved through symlinks (as {@link checkoutRealpath}
 * does on `platform`), plus the part not yet created.
 */
export function physicalPath(path: string, platform: NodeJS.Platform = process.platform): string {
  return resolvedThrough(path, realpathOn(platform));
}

/**
 * `physicalPath` spelled as the filesystem stores each name. On a case-insensitive filesystem (macOS by default)
 * `physicalPath` keeps the case `path` was typed in, while `git rev-parse --show-toplevel` gives the stored case; a path
 * Git records as it was typed, as `git worktree add` does, must be read with this to compare equal to a top level.
 */
export function storedPath(path: string): string {
  return resolvedThrough(path, realpathSync.native);
}

function resolvedThrough(path: string, realpath: (existing: string) => string): string {
  let existing = resolve(path);
  const missing: string[] = [];
  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) break;
    missing.unshift(basename(existing));
    existing = parent;
  }
  return join(realpath(existing), ...missing);
}
