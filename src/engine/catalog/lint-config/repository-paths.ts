/**
 * Paths a lint config writes, as paths in the repository: PHPStan's includes and ignored paths, Psalm's and
 * PHP_CodeSniffer's directories and rulesets are relative to the config file's folder, and set-up scopes settings to
 * whole folders of the repository only.
 */
import { lstatSync } from "node:fs";
import { join, posix } from "node:path";

/**
 * `written`, relative to folder `from` (relative to the root, `""` for the root), as a normalized path relative to the
 * root without a trailing `/`: `""` for the root itself; undefined for an absolute path or one outside the repository.
 */
export function repositoryPath(from: string, written: string): string | undefined {
  if (/^(?:\/|[A-Za-z]:[\\/])/u.test(written)) return undefined;
  const path = posix.normalize(posix.join(".", from, written)).replace(/\/+$/u, "");
  if (/^\.\.(?:\/|$)/u.test(path)) return undefined;
  return path === "." ? "" : path;
}

/** Whether `path` (relative to the root) is a folder of the repository, reached through no symbolic link. */
export function isRepositoryFolder(root: string, path: string): boolean {
  const parts = path === "" ? [] : path.split("/");
  try {
    return parts.every((_, index) => !lstatSync(join(root, ...parts.slice(0, index + 1))).isSymbolicLink()) && lstatSync(join(root, path)).isDirectory();
  } catch {
    return false;
  }
}
