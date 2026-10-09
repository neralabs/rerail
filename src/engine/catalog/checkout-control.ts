import { existsSync, realpathSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

/** True when `candidate` is `root` or below it; canonicalize both first when symlinked identity matters. */
export function withinCheckout(root: string, candidate: string): boolean {
  const path = relative(root, resolve(candidate));
  return !isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`);
}

/** The canonical location of `path`: its deepest existing ancestor resolved through symlinks, plus the part not yet created. */
export function physicalPath(path: string): string {
  let existing = resolve(path);
  const missing: string[] = [];
  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) break;
    missing.unshift(basename(existing));
    existing = parent;
  }
  return join(realpathSync(existing), ...missing);
}
