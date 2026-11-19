import { existsSync, lstatSync, realpathSync } from "node:fs";
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
 * the earlier-key read of `SqliteAuditControlStore` (`#settingsRow`, `#earlierKey`, `#savedHere`, `#outsideGit`,
 * `settingsUnderEither`, `recordedRepository`, `SpeltFolder`), `CheckoutContext.earlierCheckoutPath` with `earlierKeyOf`,
 * `earlierKeyAt`, `EARLIER_KEYS_HELD`, `spelledCheckout` and `ownStarted`'s second key in `repository-registry.ts`;
 * the `spelling` value of `GET /api/presentation` (`readingSpelling` in `operations.ts`, `DaemonState.readsSpelling`,
 * and the `spelling` that `presentationQuery` in `session-presentation.ts` hands the daemon for this lookup).
 */
export function earlierRealpath(path: string): string {
  return realpathOn("linux")(path);
}

/**
 * The canonical location of `path`: its deepest existing ancestor resolved through symlinks (as {@link checkoutRealpath}
 * does on `platform`), plus the part not yet created. A file or folder whose real path the system refuses, such as one
 * the user is denied on Windows, counts as a part below that ancestor, unless it is a link (see `resolvedThrough`).
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
 * The native realpath of `path` with nothing taken off its text first, so the system resolves its names in turn and
 * follows each link before the `..` after it, as Git resolves a path it reads from a file on Linux and macOS; `resolve`
 * and the JS realpath cancel `link/..` as text. Throws where the system cannot resolve it. On Windows Node takes a `..`
 * off the text before it asks the system, as Windows itself does.
 */
export function realpathNamesInTurn(path: string): string {
  return realpathSync.native(path);
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

/**
 * `path` resolved from its deepest ancestor `realpath` resolves, with the names below that ancestor as `path` spells
 * them: a part not yet created, and a part the system refuses to resolve. On Windows the native realpath refuses a file
 * or folder the user is denied, and one whose delete is pending (`EPERM`), where `lstat` and Node's JS realpath answer,
 * and a path can go between `existsSync` and the realpath (`ENOENT`). Such a part is no link (an `lstat` says so), so
 * the realpath of its folder is its own folder's, and every lookup a walk makes answers for it as on Linux and macOS: the
 * walk then reports that path alone, from its own read, and checks the rest. A link the system refuses keeps the
 * refusal, since where it leads is not known.
 */
function resolvedThrough(path: string, realpath: (existing: string) => string): string {
  let existing = resolve(path);
  const below: string[] = [];
  for (;;) {
    const real = existsSync(existing) ? resolvedUnlessRefused(existing, realpath) : undefined;
    const parent = dirname(existing);
    if (real !== undefined || parent === existing) return join(real ?? realpath(existing), ...below);
    below.unshift(basename(existing));
    existing = parent;
  }
}

/** The codes a realpath of a path `existsSync` found gives when the system refuses the path, or it has gone since. */
const REFUSED_OR_GONE = new Set(["EPERM", "EACCES", "ENOENT"]);

/** `realpath(path)`, or undefined when it refuses a path that is no link, or one gone since; any other failure is thrown. */
function resolvedUnlessRefused(path: string, realpath: (existing: string) => string): string | undefined {
  try {
    return realpath(path);
  } catch (error) {
    if (refusedOrGone(error) && !isLink(path)) return undefined;
    throw error;
  }
}

function refusedOrGone(error: unknown): boolean {
  return error instanceof Error && "code" in error && REFUSED_OR_GONE.has(String(error.code));
}

/** Whether `path` is a link, or cannot be told from one: an `lstat` that fails for any reason but its absence. */
function isLink(path: string): boolean {
  try {
    return lstatSync(path, { throwIfNoEntry: false })?.isSymbolicLink() === true;
  } catch {
    return true;
  }
}
