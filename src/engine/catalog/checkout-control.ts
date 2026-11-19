import { existsSync, realpathSync } from "node:fs";
import { isUtf8 } from "node:buffer";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

/** True when `candidate` is `root` or below it; canonicalize both first when symlinked identity matters. */
export function withinCheckout(root: string, candidate: string): boolean {
  const path = relative(root, resolve(candidate));
  return !isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`);
}

/** The canonical location of `path`: its deepest existing ancestor resolved through symlinks, plus the part not yet created. */
export function physicalPath(path: string): string {
  return resolvedThrough(path, realpathSync);
}

/**
 * `physicalPath` spelled as the filesystem stores each name. On a case-insensitive filesystem (macOS by default)
 * `physicalPath` keeps the case `path` was typed in, while `git rev-parse --show-toplevel` gives the stored case; a path
 * Git records as it was typed, as `git worktree add` does, must be read with this to compare equal to a top level.
 */
export function storedPath(path: string): string {
  return resolvedThrough(path, realpathSync.native);
}

/** A text-only caller must never open a different checkout through a lossy conversion of the physical path. */
export function textRealPath(path: string): string {
  const bytes = realpathSync.native(path, { encoding: "buffer" });
  if (!isUtf8(bytes)) throw new Error(`Checkout path is not valid UTF-8: ${JSON.stringify(path)}`);
  return bytes.toString();
}

/** The stored Unix scope spelling from analysis_scope::directory_in. Captured keys are never opened as paths. */
export function storedScopeDirectory(path: string): string {
  if (process.platform === "win32") return storedPath(path);
  const { existing, missing } = existingAncestor(path);
  const base = realpathSync.native(existing, { encoding: "buffer" });
  const bytes = missing.length === 0 ? base : Buffer.concat([base, Buffer.from((base.at(-1) === 47 ? "" : "/") + missing.join("/"))]);
  const text = bytes.toString();
  if (!text.includes("\uFFFD")) return text;
  const components = [];
  let start = 0;
  for (;;) {
    const end = bytes.indexOf(47, start);
    const part = bytes.subarray(start, end === -1 ? bytes.length : end);
    const name = part.toString();
    components.push(name.includes("\uFFFD") ? `./${part.toString("hex")}` : name);
    if (end === -1) return components.join("/");
    start = end + 1;
  }
}

function resolvedThrough(path: string, realpath: (existing: string) => string): string {
  const { existing, missing } = existingAncestor(path);
  return join(realpath(existing), ...missing);
}

function existingAncestor(path: string): { existing: string; missing: string[] } {
  let existing = resolve(path);
  const missing: string[] = [];
  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) break;
    missing.unshift(basename(existing));
    existing = parent;
  }
  return { existing, missing };
}
