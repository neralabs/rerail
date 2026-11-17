import { existsSync, realpathSync } from "node:fs";
import { realpath as realpathAsync } from "node:fs/promises";
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
 * The realpath Rerail 0.1.4 and earlier keyed records by on every platform: the JS one, which on Windows keeps the
 * letter case, drive letter, mapped drive and 8.3 name the caller spelt. Only for finding a record those versions saved
 * under the folder as its session spelt it; this build keys every record through {@link checkoutRealpath}.
 *
 * The lookup under that earlier key is for Windows installs upgrading from 0.1.4 or earlier, which saved a folder's
 * audit settings, and the start state of a folder outside Git, under it. It copies and stores nothing. Once no install
 * upgrades from those versions, delete this function, {@link earlierPhysicalPath}, and what the compiler then names:
 * the earlier-key read of `SqliteAuditControlStore` (`#settingsRow`, `#earlierKey`, `settingsUnderEither`),
 * `CheckoutContext.earlierCheckoutPath` with `earlierKeyOf` and `ownStarted`'s second key in `repository-registry.ts`;
 * and `startStateFolder` in `session-presentation.ts`, which hands the daemon the folder as spelt for this lookup.
 */
export function earlierRealpath(path: string): string {
  return realpathOn("linux")(path);
}

/**
 * The canonical location of `path`: its deepest existing ancestor resolved through symlinks (as {@link checkoutRealpath}
 * does on `platform`), plus the part not yet created.
 */
export function physicalPath(path: string, platform: NodeJS.Platform = process.platform): string {
  return resolvedThrough(path, realpathOn(platform));
}

/** {@link physicalPath} as Rerail 0.1.4 and earlier gave it, through {@link earlierRealpath}: the key they recorded a checkout under. */
export function earlierPhysicalPath(path: string): string {
  return resolvedThrough(path, earlierRealpath);
}

/**
 * `physicalPath` spelled as the filesystem stores each name. On a case-insensitive filesystem (macOS by default)
 * `physicalPath` keeps the case `path` was typed in, while `git rev-parse --show-toplevel` gives the stored case; a path
 * Git records as it was typed, as `git worktree add` does, must be read with this to compare equal to a top level.
 */
export function storedPath(path: string): string {
  return resolvedThrough(path, realpathSync.native);
}

/**
 * {@link storedPath} without blocking: a folder on a share that does not answer holds one of the pool's threads until
 * it does, and never the caller's. `fs.promises.realpath` is the native realpath, as `realpathSync.native` is.
 */
export async function storedPathAsync(path: string): Promise<string> {
  const missing: string[] = [];
  for (let existing = resolve(path); ; existing = dirname(existing)) {
    try {
      return join(await realpathAsync(existing), ...missing);
    } catch (error) {
      if (dirname(existing) === existing) throw error;
      missing.unshift(basename(existing));
    }
  }
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
